use wgpu::{util::DeviceExt, BindGroup, BindGroupLayout, BindGroupLayoutDescriptor, Buffer, Device, RenderPipeline, SurfaceConfiguration};

use crate::engine::rendering::{camera::CameraHandler, models::textures::Texture};

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
    pub texture: Texture,
    pub bind_group_layout: BindGroupLayout,
    pub bind_group: BindGroup,
    pub render_pipeline: RenderPipeline,
    pub vertex_buffer: Buffer,
}

impl SkyboxRender {
    pub fn new(device: &Device, config: &SurfaceConfiguration, camera: &CameraHandler, texture: Texture) -> Self {
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
            bind_group_layouts: &[&camera.bind_group_layout, &bind_group_layout],
            push_constant_ranges: &[],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Skybox Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/skybox.wgsl").into()),
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Skybox Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[SkyboxVertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
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
                // Drawn first, behind everything, and shouldn't occlude or be occluded by
                // its own (arbitrary) cube depth - so it neither writes nor tests depth.
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
        });

        Self { texture, bind_group_layout, bind_group, render_pipeline, vertex_buffer }
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
        self.texture = texture;
    }

    pub fn render<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>, camera_bind_group: &'a wgpu::BindGroup) {
        render_pass.set_pipeline(&self.render_pipeline);
        render_pass.set_bind_group(0, camera_bind_group, &[]);
        render_pass.set_bind_group(1, &self.bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.draw(0..SKYBOX_VERTICES.len() as u32, 0..1);
    }
}
