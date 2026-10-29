//! 2D nodes' shapes (see `Shape2D`) - each one GPU instance, a quad turned
//! on screen, with its edge worked out per pixel so circles and turned
//! rectangles come out smooth (shapes_2d.wgsl).
//!
//! Refilled every frame from the scene's nodes (see App::prepare_nodes_2d),
//! already in drawing order, and drawn over what's there with alpha blending
//! and no depth test - the `Nodes2D` step of the frame plan.

use wgpu::util::DeviceExt;

use crate::rendering::sdf_text::srgb_to_linear;

/// One shape, in pixels - see shapes_2d.wgsl's Instance.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ShapeInstance {
    pub center: [f32; 2],
    pub half_size: [f32; 2],
    /// cos and sin of its rotation.
    pub rotation: [f32; 2],
    /// x: 0 a rectangle, 1 a circle; y unused.
    pub kind: [f32; 2],
    pub color: [f32; 4],
}

impl ShapeInstance {
    const ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Float32x2, 4 => Float32x4];

    pub const RECT: f32 = 0.0;
    pub const CIRCLE: f32 = 1.0;
}

pub struct Shapes2D {
    pipeline: wgpu::RenderPipeline,
    screen_buffer: wgpu::Buffer,
    screen_bind_group: wgpu::BindGroup,
    /// This frame's shapes - grown when a frame has more than fit.
    instances: wgpu::Buffer,
    capacity: usize,
    count: u32,
    /// What's in the buffers now - a frame with the same shapes on the same
    /// screen writes nothing.
    uploaded: Vec<ShapeInstance>,
    uploaded_screen: (u32, u32),
    /// Whether the target stores colors as sRGB - then a shape's color is
    /// made linear first, so it comes out as the color it names (the GPU
    /// encodes it back on the way in).
    srgb_target: bool,
}

impl Shapes2D {
    /// Its first instance buffer's room - most frames need no more.
    const INITIAL_CAPACITY: usize = 64;

    pub(crate) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shapes 2D shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/2d/shapes_2d.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Shapes 2D screen layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let screen_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Shapes 2D screen"),
            contents: bytemuck::cast_slice(&[1.0f32, 1.0, 0.0, 0.0]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let screen_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Shapes 2D screen"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: screen_buffer.as_entire_binding() }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Shapes 2D pipeline layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Shapes 2D pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<ShapeInstance>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &ShapeInstance::ATTRIBUTES,
                }],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        Self {
            pipeline,
            screen_buffer,
            screen_bind_group,
            instances: Self::instance_buffer(device, Self::INITIAL_CAPACITY),
            capacity: Self::INITIAL_CAPACITY,
            count: 0,
            uploaded: Vec::new(),
            uploaded_screen: (0, 0),
            srgb_target: format.is_srgb(),
        }
    }

    fn instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Shapes 2D instances"),
            size: (capacity * std::mem::size_of::<ShapeInstance>()) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// This frame's shapes, in drawing order (the first at the back), on a
    /// target `screen_px` pixels big.
    pub(crate) fn set(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, shapes: &[ShapeInstance], screen_px: (u32, u32)) {
        self.count = shapes.len() as u32;
        if shapes.is_empty() {
            return;
        }
        let same: bool = bytemuck::cast_slice::<ShapeInstance, u8>(shapes) == bytemuck::cast_slice::<ShapeInstance, u8>(&self.uploaded);
        if same && screen_px == self.uploaded_screen {
            return;
        }
        self.uploaded.clear();
        self.uploaded.extend_from_slice(shapes);
        self.uploaded_screen = screen_px;
        if shapes.len() > self.capacity {
            self.capacity = shapes.len().next_power_of_two();
            self.instances = Self::instance_buffer(device, self.capacity);
        }
        if self.srgb_target {
            let linear: Vec<ShapeInstance> = shapes.iter().map(|shape| {
                let [r, g, b, a] = shape.color;
                ShapeInstance { color: [srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b), a], ..*shape }
            }).collect();
            queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(&linear));
        } else {
            queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(shapes));
        }
        queue.write_buffer(&self.screen_buffer, 0, bytemuck::cast_slice(&[screen_px.0 as f32, screen_px.1 as f32, 0.0, 0.0]));
    }

    /// Draws this frame's shapes over `view` - nothing when there are none.
    pub(crate) fn draw(&self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        if self.count == 0 {
            return;
        }
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Shapes 2D pass"),
            color_attachments: &[crate::rendering::render_pipeline::render_pass::color_attachment(view, wgpu::LoadOp::Load)],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.screen_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.instances.slice(..));
        render_pass.draw(0..6, 0..self.count);
    }
}
