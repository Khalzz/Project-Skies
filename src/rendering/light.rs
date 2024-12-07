use wgpu::{util::DeviceExt, BindGroup, BindGroupLayout, Buffer, Device, RenderPipeline, SurfaceConfiguration};

use super::{camera::CameraRenderizable, model::{self, Vertex}, rendering_utils, textures::Texture};


#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LightUniform {
    pub position: [f32; 3],
    pub _padding: u32,
    pub color: [f32; 3],
    pub _padding2: u32,
}

pub struct LightRenderData {
    pub bind_group_layout: BindGroupLayout,
    pub bind_group: BindGroup,
    pub buffer: Buffer,
    pub render_pipeline: RenderPipeline
}

/// # Light
/// This struct is dedicated for light creation and rendering of them.
/// 
/// ## Values:
/// - uniform: The uniform data that will be given to the shader to render the light
pub struct Light {
    pub uniform: LightUniform,
    pub rendering_data: LightRenderData

}

impl Light {
    pub fn new(device: &Device, config: &SurfaceConfiguration, camera: &CameraRenderizable) -> Self {
        let uniform = LightUniform {
            position: [0.0, 5.0, 0.0],
            _padding: 0,
            color: [1.0, 1.0, 1.0],
            _padding2: 0,
        };

        let buffer = device.create_buffer_init(
            &wgpu::util::BufferInitDescriptor {
                label: Some("Light VB"),
                contents: bytemuck::cast_slice(&[uniform]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, // remember that copydst let us change later elements like position of objects
            }
        );

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
            label: None,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
            label: None,
        });

        let render_pipeline = {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Light Pipeline Layout"),
                bind_group_layouts: &[&camera.bind_group_layout, &bind_group_layout],
                push_constant_ranges: &[],
            });

            let shader = wgpu::ShaderModuleDescriptor {
                label: Some("Light Shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/light.wgsl").into()),
            };
            
            rendering_utils::create_render_pipeline(
                &device,
                &layout,
                config.format,
                Some(Texture::DEPTH_FORMAT),
                &[model::ModelVertex::desc()],
                shader,
            )
        };

        let rendering_data = LightRenderData {
            bind_group_layout,
            bind_group,
            buffer,
            render_pipeline,
        };

        Self {
            uniform,
            rendering_data,
        }
    }
}