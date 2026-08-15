use wgpu::{util::DeviceExt, BindGroup, BindGroupLayout, BindGroupLayoutDescriptor, Buffer, Device, RenderPipeline, SurfaceConfiguration};

use crate::engine::primitive::manual_vertex::ManualVertexTexturized;
use crate::engine::rendering::models::textures::Texture;

const QUAD_VERTICES: &[ManualVertexTexturized] = &[
    ManualVertexTexturized { position: [-1.0, -1.0, 0.0], tex_coords: [0.0, 1.0] },
    ManualVertexTexturized { position: [1.0, -1.0, 0.0], tex_coords: [1.0, 1.0] },
    ManualVertexTexturized { position: [1.0, 1.0, 0.0], tex_coords: [1.0, 0.0] },
    ManualVertexTexturized { position: [-1.0, 1.0, 0.0], tex_coords: [0.0, 0.0] },
];
const QUAD_INDICES: &[u16] = &[0, 1, 2, 0, 2, 3];

// Per-iteration step multiplier for the compounding blur passes (see
// BlurRender::render) - doubling each time is what lets 3 cheap 9-tap passes
// reach a large, smooth effective radius instead of needing a single pass with
// a huge (and therefore sparse/ghosty) tap spread.
const BLUR_STEPS: [f32; 3] = [1.0, 2.0, 4.0];

/// The pixel radius UiNode::set_background_blur maps to "fully blurred" - values
/// at or above this sample `blurred` outright, values in between linearly
/// crossfade between `scene_color` (sharp) and `blurred` (see text_shader.wgsl).
/// Matches Tailwind's backdrop-blur-3xl (~64px).
pub const MAX_BLUR_RADIUS: f32 = 64.0;

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct DirectionUniform {
    direction: [f32; 2],
    _padding: [f32; 2],
}

// Deliberately not Texture::create_texture - that helper's sampler always sets
// `compare: Some(...)` (meant for depth textures), which a Filtering-typed sampler
// binding (see bind_group_layout below) rejects at bind-group-creation time. These
// are plain color render targets, so a plain non-comparison linear sampler.
fn offscreen_texture(device: &Device, width: u32, height: u32, format: wgpu::TextureFormat, label: &str) -> Texture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: width.max(1), height: height.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });
    Texture { texture, view, sampler }
}

fn direction_buffer(device: &Device, direction: [f32; 2], label: &str) -> Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(label),
        contents: bytemuck::cast_slice(&[DirectionUniform { direction, _padding: [0.0, 0.0] }]),
        usage: wgpu::BufferUsages::UNIFORM,
    })
}

fn make_bind_group(device: &Device, layout: &BindGroupLayout, source: &Texture, direction: &Buffer, label: &str) -> BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&source.view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&source.sampler) },
            wgpu::BindGroupEntry { binding: 2, resource: direction.as_entire_binding() },
        ],
    })
}

fn fullscreen_pipeline(device: &Device, layout: &wgpu::PipelineLayout, shader: &wgpu::ShaderModule, format: wgpu::TextureFormat) -> RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("fullscreen_pipeline"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            buffers: &[ManualVertexTexturized::desc()],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: Some(wgpu::Face::Back),
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    })
}

/// Real screen-space background blur ("backdrop-filter: blur()") for UI panels -
/// see `UiNode::set_background_blur`. The 3D scene renders into `scene_color`
/// every frame (an offscreen copy - see `App::render_opaque_pass`/
/// `render_transparent_pass`) instead of straight into the swapchain, since the UI
/// pass needs something already-rendered to sample from before it draws.
///
/// `render` then (1) blits that sharp copy onto the swapchain, so the scene still
/// looks completely normal everywhere nothing's actually blurred, and (2) builds
/// `blurred`: a full-resolution, densely-sampled blur reached by chaining three
/// 9-tap separable Gaussian passes (H+V each) at growing step sizes (see
/// BLUR_STEPS) rather than one pass with a huge, sparse tap spread - the latter
/// is what produced a "same element repeated" ghosting artifact an earlier
/// version of this had at large radii. Every UI node with `background_blur` set
/// then crossfades between `scene_color` and this single shared `blurred`
/// texture based on its own radius (see `MAX_BLUR_RADIUS`/`Ui::blur_bind_group`)
/// - not a physically exact reproduction of "blur(Npx)" at every radius, but
/// smooth and artifact-free at every strength, which matters more for a UI
/// effect.
pub struct BlurRender {
    pub scene_color: Texture,
    blur_a: Texture,
    pub blurred: Texture,
    blit_bind_group: BindGroup,
    blit_pipeline: RenderPipeline,
    // One bind group per compounding pass, in order - even indices are H passes
    // (write into blur_a), odd are V passes (write into blurred), see `render`.
    blur_passes: Vec<BindGroup>,
    blur_pipeline: RenderPipeline,
    vertex_buffer: Buffer,
    index_buffer: Buffer,
}

impl BlurRender {
    pub fn new(device: &Device, config: &SurfaceConfiguration) -> Self {
        let scene_color = offscreen_texture(device, config.width, config.height, config.format, "scene_color");
        let blur_a = offscreen_texture(device, config.width, config.height, config.format, "blur_a");
        let blurred = offscreen_texture(device, config.width, config.height, config.format, "blurred_scene");

        // ── Blit pipeline (scene_color -> swapchain, unblurred) ──
        let blit_layout = Texture::create_bind_group_layout(device);
        let blit_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("scene_blit_bind_group"),
            layout: &blit_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&scene_color.view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&scene_color.sampler) },
            ],
        });
        let blit_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene_blit_pipeline_layout"),
            bind_group_layouts: &[&blit_layout],
            push_constant_ranges: &[],
        });
        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Scene Blit Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/scene_blit.wgsl").into()),
        });
        let blit_pipeline = fullscreen_pipeline(device, &blit_pipeline_layout, &blit_shader, config.format);

        // ── Blur pipeline (compounding separable Gaussian, see BLUR_STEPS) ──
        let blur_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("blur_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        // Six passes total (H then V per step): scene_color -> blur_a -> blurred
        // -> blur_a -> blurred -> blur_a -> blurred - `blurred` always ends up
        // holding the final result regardless of BLUR_STEPS' length. Only the
        // very first H pass reads scene_color; every H pass after that re-blurs
        // the previous iteration's result (`blurred`), which is what compounds
        // the effect - V always reads blur_a, that iteration's fresh H output.
        let inv_w = 1.0 / config.width.max(1) as f32;
        let inv_h = 1.0 / config.height.max(1) as f32;
        let mut blur_passes = Vec::with_capacity(BLUR_STEPS.len() * 2);
        for (i, step) in BLUR_STEPS.iter().enumerate() {
            let h_source = if i == 0 { &scene_color } else { &blurred };
            let h_dir = direction_buffer(device, [step * inv_w, 0.0], "blur_direction_h");
            blur_passes.push(make_bind_group(device, &blur_layout, h_source, &h_dir, "blur_h_bind_group"));

            let v_dir = direction_buffer(device, [0.0, step * inv_h], "blur_direction_v");
            blur_passes.push(make_bind_group(device, &blur_layout, &blur_a, &v_dir, "blur_v_bind_group"));
        }

        let blur_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blur_pipeline_layout"),
            bind_group_layouts: &[&blur_layout],
            push_constant_ranges: &[],
        });
        let blur_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Blur Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/blur.wgsl").into()),
        });
        let blur_pipeline = fullscreen_pipeline(device, &blur_pipeline_layout, &blur_shader, config.format);

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Blur Pass VB"),
            contents: bytemuck::cast_slice(QUAD_VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Blur Pass IB"),
            contents: bytemuck::cast_slice(QUAD_INDICES),
            usage: wgpu::BufferUsages::INDEX,
        });

        Self {
            scene_color,
            blur_a,
            blurred,
            blit_bind_group,
            blit_pipeline,
            blur_passes,
            blur_pipeline,
            vertex_buffer,
            index_buffer,
        }
    }

    // Full recreation rather than an in-place update - resize is rare enough that
    // the extra pipeline/shader rebuild cost doesn't matter, and it rules out an
    // entire class of "forgot to update one of these together" bugs (same
    // trade-off DepthRender/Texture::create_depth_texture already make elsewhere).
    pub fn resize(&mut self, device: &Device, config: &SurfaceConfiguration) {
        *self = Self::new(device, config);
    }

    fn draw_pass(&self, encoder: &mut wgpu::CommandEncoder, pipeline: &RenderPipeline, target: &wgpu::TextureView, bind_group: &BindGroup, label: &str) {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        render_pass.set_pipeline(pipeline);
        render_pass.set_bind_group(0, bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        render_pass.draw_indexed(0..QUAD_INDICES.len() as u32, 0, 0..1);
    }

    /// Runs every frame (see `App::render_scene_passes`), after the 3D scene has
    /// rendered into `scene_color` and before the UI pass: blits that sharp copy
    /// onto `view` (the swapchain), then rebuilds `blurred` via the compounding
    /// blur chain, ready for the UI pass to crossfade toward per node.
    pub fn render(&self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        self.draw_pass(encoder, &self.blit_pipeline, view, &self.blit_bind_group, "Scene Blit Pass");

        for (i, bind_group) in self.blur_passes.iter().enumerate() {
            let target = if i % 2 == 0 { &self.blur_a.view } else { &self.blurred.view };
            self.draw_pass(encoder, &self.blur_pipeline, target, bind_group, "Blur Pass");
        }
    }
}
