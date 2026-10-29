use wgpu::{util::DeviceExt, BindGroup, BindGroupLayoutDescriptor, Buffer, Device, RenderPipeline, SurfaceConfiguration, TextureView};

use crate::rendering::vertex::QuadVertex;
use crate::rendering::models::textures::Texture;

const QUAD_VERTICES: &[QuadVertex] = &[
    QuadVertex { position: [-1.0, -1.0, 0.0], tex_coords: [0.0, 1.0] },
    QuadVertex { position: [1.0, -1.0, 0.0], tex_coords: [1.0, 1.0] },
    QuadVertex { position: [1.0, 1.0, 0.0], tex_coords: [1.0, 0.0] },
    QuadVertex { position: [-1.0, 1.0, 0.0], tex_coords: [0.0, 0.0] },
];
const QUAD_INDICES: &[u16] = &[0, 1, 2, 0, 2, 3];

/// How many blur strengths there are - level `k` (1-based) blurs about
/// `2^(k+1)` pixels: 4, 8, 16, 32, 64. ui_template.wgsl's BLUR_LEVELS must
/// match.
pub const BLUR_LEVELS: u32 = 5;

/// The pixel radius UiNode::set_background_blur reaches the strongest level
/// at - Tailwind's backdrop-blur-3xl (~64px). See ui_template.wgsl.
pub const MAX_BLUR_RADIUS: f32 = 64.0;

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct HalfTexelUniform {
    half_texel: [f32; 2],
    _padding: [f32; 2],
}

/// `size` halved `level` times, never below 1.
fn level_size(size: (u32, u32), level: u32) -> (u32, u32) {
    ((size.0 >> level).max(1), (size.1 >> level).max(1))
}

fn color_texture(device: &Device, (width, height): (u32, u32), layers: u32, format: wgpu::TextureFormat, label: &str) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: width.max(1), height: height.max(1), depth_or_array_layers: layers },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

// Deliberately not Texture::create_texture's - that one's sampler always sets
// `compare: Some(...)` (meant for depth textures), which a Filtering-typed
// sampler binding rejects at bind-group-creation time. These are plain color
// targets, so a plain non-comparison linear sampler.
fn linear_sampler(device: &Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    })
}

pub(crate) fn offscreen_texture(device: &Device, width: u32, height: u32, format: wgpu::TextureFormat, label: &str) -> Texture {
    let texture = color_texture(device, (width, height), 1, format, label);
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Texture { texture, view, sampler: linear_sampler(device) }
}

fn fullscreen_pipeline(device: &Device, layout: &wgpu::PipelineLayout, shader: &wgpu::ShaderModule, entry_point: &str, format: wgpu::TextureFormat) -> RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(entry_point),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            buffers: &[QuadVertex::desc()],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(entry_point),
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

/// One step of the blur chain: `source` (bound in `bind_group`) halved
/// (`up: false`) or doubled into `target`.
struct BlurPass {
    up: bool,
    bind_group: BindGroup,
    target: TextureView,
}

/// Real screen-space background blur ("backdrop-filter: blur()") for UI panels -
/// see `UiNode::set_background_blur`. The 3D scene renders into `scene_color`
/// every frame (an offscreen copy - see `App::render_opaque_pass`/
/// `render_transparent_pass`) instead of straight into the swapchain, since the UI
/// pass needs something already-rendered to sample from before it draws.
///
/// The blur is a Dual Kawase chain (see blur.wgsl): the image is halved five
/// times (½ down to 1/32), and each of those is doubled back up to ½
/// resolution - `BLUR_LEVELS` blurs of growing strength, one layer each of
/// `levels`. A blurred node picks between them by its radius (see
/// ui_template.wgsl), so a small radius really is a small blur. Everything
/// runs at ½ resolution or lower, the taps sit between texels so the bilinear
/// filter does part of the averaging, and the cost hardly depends on the
/// radius.
///
/// It only runs on frames that need it - a node with `background_blur`, or a
/// Fullscreen shader reading "blurred" (see App::run_frame_plan).
pub struct BlurRender {
    /// The world drawn this frame - what the frame plan's Fullscreen step
    /// copies onto the screen (its "scene" input, see shader_library).
    pub scene_color: Texture,
    /// The frame as it is right before the always-on-top UI (a modal) -
    /// world and HUD both - so that layer's blur shows the HUD behind it
    /// instead of only the world. See `blur_backdrop`.
    pub backdrop: Texture,
    /// The `BLUR_LEVELS` blurs, weakest first, at ½ resolution - one array
    /// layer each, kept alive for `levels_view`/`blurred_view`.
    _levels: wgpu::Texture,
    /// All the levels, for the UI (a `texture_2d_array`).
    pub levels_view: TextureView,
    /// The strongest level alone - a Fullscreen shader's "blurred" input.
    pub blurred_view: TextureView,
    pub sampler: wgpu::Sampler,
    /// The chain's smaller images (¼ and below) - kept alive for `passes`.
    _scratch: Vec<wgpu::Texture>,
    /// The first step - halving `scene_color`, or `backdrop` - then the rest.
    first_from_scene: BlurPass,
    first_from_backdrop: BlurPass,
    passes: Vec<BlurPass>,
    down_pipeline: RenderPipeline,
    up_pipeline: RenderPipeline,
    vertex_buffer: Buffer,
    index_buffer: Buffer,
}

impl BlurRender {
    pub fn new(device: &Device, config: &SurfaceConfiguration) -> Self {
        let format = config.format;
        let full = (config.width.max(1), config.height.max(1));
        let scene_color = offscreen_texture(device, full.0, full.1, format, "scene_color");
        let backdrop = offscreen_texture(device, full.0, full.1, format, "ui_backdrop");
        let sampler = linear_sampler(device);

        let levels = color_texture(device, level_size(full, 1), BLUR_LEVELS, format, "blur_levels");
        let layer_view = |layer: u32| levels.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2),
            base_array_layer: layer,
            array_layer_count: Some(1),
            ..Default::default()
        });
        let levels_view = levels.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let blurred_view = layer_view(BLUR_LEVELS - 1);

        // Texture + sampler + one uniform (half a texel of the source).
        let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
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
        let pass = |up: bool, source: &TextureView, source_size: (u32, u32), target: TextureView| {
            let uniform = HalfTexelUniform { half_texel: [0.5 / source_size.0 as f32, 0.5 / source_size.1 as f32], _padding: [0.0; 2] };
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("blur_half_texel"),
                contents: bytemuck::cast_slice(&[uniform]),
                usage: wgpu::BufferUsages::UNIFORM,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("blur_pass_bind_group"),
                layout: &layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(source) },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
                    wgpu::BindGroupEntry { binding: 2, resource: buffer.as_entire_binding() },
                ],
            });
            BlurPass { up, bind_group, target }
        };

        // Level 1 is the full image halved once, straight into its layer.
        let first_from_scene = pass(false, &scene_color.view, full, layer_view(0));
        let first_from_backdrop = pass(false, &backdrop.view, full, layer_view(0));

        // down[j]: the image halved j times (j >= 2) - and a same-sized
        // scratch image for the way back up.
        let mut scratch = Vec::new();
        let mut down_views = vec![layer_view(0), layer_view(0)];
        let mut up_views = vec![layer_view(0), layer_view(0)];
        for j in 2..=BLUR_LEVELS {
            let down = color_texture(device, level_size(full, j), 1, format, "blur_down");
            down_views.push(down.create_view(&wgpu::TextureViewDescriptor::default()));
            scratch.push(down);
            let up = color_texture(device, level_size(full, j), 1, format, "blur_up");
            up_views.push(up.create_view(&wgpu::TextureViewDescriptor::default()));
            scratch.push(up);
        }

        let mut passes = Vec::new();
        for j in 2..=BLUR_LEVELS {
            passes.push(pass(false, &down_views[j as usize - 1], level_size(full, j - 1), down_views[j as usize].clone()));
        }
        // Level k: the image halved k times, doubled back to ½ resolution -
        // through the scratch images, into its layer.
        for k in 2..=BLUR_LEVELS {
            let mut source = down_views[k as usize].clone();
            for j in (1..k).rev() {
                let target = if j == 1 { layer_view(k - 1) } else { up_views[j as usize].clone() };
                passes.push(pass(true, &source, level_size(full, j + 1), target.clone()));
                source = target;
            }
        }

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blur_pipeline_layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Blur Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/post/blur.wgsl").into()),
        });
        let down_pipeline = fullscreen_pipeline(device, &pipeline_layout, &shader, "fs_down", format);
        let up_pipeline = fullscreen_pipeline(device, &pipeline_layout, &shader, "fs_up", format);

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
            backdrop,
            _levels: levels,
            levels_view,
            blurred_view,
            sampler,
            _scratch: scratch,
            first_from_scene,
            first_from_backdrop,
            passes,
            down_pipeline,
            up_pipeline,
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

    fn draw_pass(&self, encoder: &mut wgpu::CommandEncoder, pass: &BlurPass) {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(if pass.up { "Blur Up" } else { "Blur Down" }),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &pass.target,
                resolve_target: None,
                // Every pixel is drawn over - nothing to load or clear.
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        render_pass.set_pipeline(if pass.up { &self.up_pipeline } else { &self.down_pipeline });
        render_pass.set_bind_group(0, &pass.bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        render_pass.draw_indexed(0..QUAD_INDICES.len() as u32, 0, 0..1);
    }

    fn run_chain(&self, encoder: &mut wgpu::CommandEncoder, first: &BlurPass) {
        self.draw_pass(encoder, first);
        for pass in &self.passes {
            self.draw_pass(encoder, pass);
        }
    }

    /// Rebuilds the blur levels from `scene_color`, ready for the UI to pick
    /// from per node - once the world's drawn, before the UI (see
    /// App::run_frame_plan).
    pub fn blur_scene(&self, encoder: &mut wgpu::CommandEncoder) {
        self.run_chain(encoder, &self.first_from_scene);
    }

    /// Copies `frame` (what's been drawn so far - the screen) into `backdrop`
    /// and rebuilds the blur levels from it, for the always-on-top UI to
    /// blur. False (nothing done) when `frame` can't be copied from or isn't
    /// the same size and format.
    pub fn blur_backdrop(&self, encoder: &mut wgpu::CommandEncoder, frame: &wgpu::Texture) -> bool {
        let backdrop = &self.backdrop.texture;
        if !frame.usage().contains(wgpu::TextureUsages::COPY_SRC) || frame.size() != backdrop.size() || frame.format() != backdrop.format() {
            return false;
        }
        encoder.copy_texture_to_texture(frame.as_image_copy(), backdrop.as_image_copy(), backdrop.size());
        self.run_chain(encoder, &self.first_from_backdrop);
        true
    }
}
