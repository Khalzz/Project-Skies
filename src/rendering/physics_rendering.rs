use wgpu::{util::DeviceExt, BindGroup, BindGroupLayout, Buffer, Device, PipelineLayout, RenderPipeline, SurfaceConfiguration};
use crate::primitive::manual_vertex::ManualVertex;

use super::{camera::{Camera, CameraRenderizable}, rendering_utils, textures::Texture};

pub struct RenderPhysics {
    pub renderizable_lines: Vec<[ManualVertex; 2]>,
    pub vertex_buffer: Buffer,
    pub index_buffer: Buffer,
    pub bind_group_layout: BindGroupLayout,
    pub bind_group: BindGroup,
    pub render_pipeline: RenderPipeline,
}

impl RenderPhysics {
    pub fn new(device: &Device, config: &SurfaceConfiguration, camera: &CameraRenderizable) -> Self{
        let vertex = [ManualVertex::default(); 2];
        let indices = [0, 1];

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ManualVertex Buffer"),
            contents: bytemuck::cast_slice(&vertex),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Index Buffer"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        // Define the bind group layout
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ManualVertex Bind Group Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform, // Change this to Uniform
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: vertex_buffer.as_entire_binding(),
            }],
            label: None,
        });

        let render_pipeline = {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Physics Pipeline Layout"),
                bind_group_layouts: &[&bind_group_layout, &camera.bind_group_layout],
                push_constant_ranges: &[],
            });

            let shader = wgpu::ShaderModuleDescriptor {
                label: Some("Physics Shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/physics_lines.wgsl").into()),
            };
            
            rendering_utils::create_line_render_pipeline(
                &device,
                &layout,
                config.format,
                None,
                &[ManualVertex::desc()],
                shader,
            )
        };

        Self {
            vertex_buffer,
            index_buffer,
            renderizable_lines: vec![], 
            bind_group_layout,
            bind_group,
            render_pipeline,
        }
    }
}