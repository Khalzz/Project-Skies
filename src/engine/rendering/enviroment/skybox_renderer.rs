use wgpu::{util::DeviceExt, BindGroup, BindGroupLayout, BindGroupLayoutDescriptor, Buffer, Device, RenderPipeline, SurfaceConfiguration};

use crate::engine::rendering::models::textures::Texture;

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct SkyboxVertex {
    position: [f32; 3],
}

impl SkyboxVertex {
    fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<SkyboxVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x3,
            }],
        }
    }
}

// A cube centered on the camera. Only the direction from the origin to each vertex
// matters (it's used as the cubemap sample vector), so the size is arbitrary and
// winding doesn't matter either since the pipeline disables backface culling.
#[rustfmt::skip]
const SKYBOX_VERTICES: &[SkyboxVertex] = &[
    // -X
    SkyboxVertex { position: [-1.0, -1.0, -1.0] }, SkyboxVertex { position: [-1.0, -1.0,  1.0] }, SkyboxVertex { position: [-1.0,  1.0,  1.0] },
    SkyboxVertex { position: [-1.0, -1.0, -1.0] }, SkyboxVertex { position: [-1.0,  1.0,  1.0] }, SkyboxVertex { position: [-1.0,  1.0, -1.0] },
    // +X
    SkyboxVertex { position: [ 1.0, -1.0, -1.0] }, SkyboxVertex { position: [ 1.0,  1.0, -1.0] }, SkyboxVertex { position: [ 1.0,  1.0,  1.0] },
    SkyboxVertex { position: [ 1.0, -1.0, -1.0] }, SkyboxVertex { position: [ 1.0,  1.0,  1.0] }, SkyboxVertex { position: [ 1.0, -1.0,  1.0] },
    // -Y
    SkyboxVertex { position: [-1.0, -1.0, -1.0] }, SkyboxVertex { position: [ 1.0, -1.0, -1.0] }, SkyboxVertex { position: [ 1.0, -1.0,  1.0] },
    SkyboxVertex { position: [-1.0, -1.0, -1.0] }, SkyboxVertex { position: [ 1.0, -1.0,  1.0] }, SkyboxVertex { position: [-1.0, -1.0,  1.0] },
    // +Y
    SkyboxVertex { position: [-1.0,  1.0, -1.0] }, SkyboxVertex { position: [-1.0,  1.0,  1.0] }, SkyboxVertex { position: [ 1.0,  1.0,  1.0] },
    SkyboxVertex { position: [-1.0,  1.0, -1.0] }, SkyboxVertex { position: [ 1.0,  1.0,  1.0] }, SkyboxVertex { position: [ 1.0,  1.0, -1.0] },
    // -Z
    SkyboxVertex { position: [-1.0, -1.0, -1.0] }, SkyboxVertex { position: [-1.0,  1.0, -1.0] }, SkyboxVertex { position: [ 1.0,  1.0, -1.0] },
    SkyboxVertex { position: [-1.0, -1.0, -1.0] }, SkyboxVertex { position: [ 1.0,  1.0, -1.0] }, SkyboxVertex { position: [ 1.0, -1.0, -1.0] },
    // +Z
    SkyboxVertex { position: [-1.0, -1.0,  1.0] }, SkyboxVertex { position: [ 1.0, -1.0,  1.0] }, SkyboxVertex { position: [ 1.0,  1.0,  1.0] },
    SkyboxVertex { position: [-1.0, -1.0,  1.0] }, SkyboxVertex { position: [ 1.0,  1.0,  1.0] }, SkyboxVertex { position: [-1.0,  1.0,  1.0] },
];

pub struct SkyboxRender {
    // `None` for a procedural sky (see `new_procedural`) - it has no cubemap,
    // its group-1 bind group is a small params uniform instead.
    pub texture: Option<Texture>,
    pub bind_group_layout: BindGroupLayout,
    pub bind_group: BindGroup,
    pub render_pipeline: RenderPipeline,
    pub vertex_buffer: Buffer,
}

// Uniform for the procedural sky shader (sky.wgsl). All colours are linear
// RGB; the alpha slots are padding.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct SkyUniform {
    sun_direction: [f32; 4],
    zenith_color: [f32; 4],
    horizon_color: [f32; 4],
    sun_color: [f32; 4],
}

impl SkyboxRender {
    // Only needs the camera's bind group layout (to build a matching pipeline layout),
    // not the whole CameraHandler - keeps this callable from a background asset-loading
    // thread that only has cloned GPU handles, not access to `App`/`CameraHandler`.
    pub fn new(device: &Device, config: &SurfaceConfiguration, camera_bind_group_layout: &BindGroupLayout, texture: Texture) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("skybox_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::Cube,
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
            ],
        });

        let bind_group = Self::create_bind_group(device, &bind_group_layout, &texture);

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Skybox VB"),
            contents: bytemuck::cast_slice(SKYBOX_VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Skybox Pipeline Layout"),
            bind_group_layouts: &[camera_bind_group_layout, &bind_group_layout],
            push_constant_ranges: &[],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Skybox Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/skybox.wgsl").into()),
        });

        let render_pipeline = Self::build_pipeline(device, config, &pipeline_layout, &shader);

        Self { texture: Some(texture), bind_group_layout, bind_group, render_pipeline, vertex_buffer }
    }

    /// Procedural clear-day sea sky - no cubemap. Group 1 is a small params
    /// uniform (sun direction + palette, set once here) instead of a texture;
    /// everything else (the camera-centred cube, the no-depth pipeline, the
    /// `render` path) is identical to a real skybox. Tune the look below.
    pub fn new_procedural(device: &Device, config: &SurfaceConfiguration, camera_bind_group_layout: &BindGroupLayout) -> Self {
        let normalize3 = |v: [f32; 3]| {
            let inv = 1.0 / (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            [v[0] * inv, v[1] * inv, v[2] * inv, 0.0]
        };
        let sky_uniform = SkyUniform {
            // Mid-morning sun, high and off to one side so the disc is visible.
            sun_direction: normalize3([0.25, 0.80, 0.35]),
            zenith_color: [0.19, 0.42, 0.78, 1.0],
            horizon_color: [0.74, 0.83, 0.90, 1.0],
            sun_color: [1.0, 0.95, 0.85, 1.0],
        };

        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("procedural_sky_bind_group_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("procedural_sky_uniform"),
            contents: bytemuck::cast_slice(&[sky_uniform]),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("procedural_sky_bind_group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Procedural Sky VB"),
            contents: bytemuck::cast_slice(SKYBOX_VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Procedural Sky Pipeline Layout"),
            bind_group_layouts: &[camera_bind_group_layout, &bind_group_layout],
            push_constant_ranges: &[],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Procedural Sky Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/sky.wgsl").into()),
        });

        let render_pipeline = Self::build_pipeline(device, config, &pipeline_layout, &shader);

        Self { texture: None, bind_group_layout, bind_group, render_pipeline, vertex_buffer }
    }

    // Shared pipeline setup for skybox.wgsl / sky.wgsl - both take the same
    // camera-centred cube VB, write straight to the swapchain with no blend,
    // and neither write nor test depth (drawn first, behind everything).
    fn build_pipeline(device: &Device, config: &SurfaceConfiguration, pipeline_layout: &wgpu::PipelineLayout, shader: &wgpu::ShaderModule) -> RenderPipeline {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Sky Render Pipeline"),
            layout: Some(pipeline_layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_main"),
                buffers: &[SkyboxVertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                // The camera sits inside the cube, so both winding orders are visible from within it.
                cull_mode: None,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: Texture::DEPTH_FORMAT,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        })
    }

    fn create_bind_group(device: &Device, layout: &BindGroupLayout, texture: &Texture) -> BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("skybox_bind_group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&texture.sampler),
                },
            ],
        })
    }

    /// Swaps in a different cubemap (e.g. after loading real face images) without rebuilding the pipeline.
    #[allow(unused)]
    pub fn set_texture(&mut self, device: &Device, texture: Texture) {
        self.bind_group = Self::create_bind_group(device, &self.bind_group_layout, &texture);
        self.texture = Some(texture);
    }

    pub fn render<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>, camera_bind_group: &'a wgpu::BindGroup) {
        render_pass.set_pipeline(&self.render_pipeline);
        render_pass.set_bind_group(0, camera_bind_group, &[]);
        render_pass.set_bind_group(1, &self.bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.draw(0..SKYBOX_VERTICES.len() as u32, 0..1);
    }
}
