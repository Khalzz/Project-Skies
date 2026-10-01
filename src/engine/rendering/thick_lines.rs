//! Thick lines - segments drawn a fixed number of pixels wide whatever their
//! distance, for views made of lines (the play scene's F10 tactical map).
//! Each segment is one GPU instance that the shader (thick_lines.wgsl)
//! widens into a quad on screen, so a big, unchanging set (a terrain's
//! contours) is uploaded once (`set_static`) and only what moves is sent
//! every frame (`dynamic`).
//!
//! Drawn over whatever's there with alpha blending and no depth test - what's
//! drawn last is on top.

use wgpu::util::DeviceExt;

use crate::engine::rendering::camera::handler::CameraResources;

/// One segment, in world space - `width` in pixels, `color` RGBA (alpha
/// blends).
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LineInstance {
    pub start: [f32; 3],
    pub end: [f32; 3],
    pub color: [f32; 4],
    pub width: f32,
}

impl LineInstance {
    const ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<LineInstance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

/// The shader's own uniform - see thick_lines.wgsl's LineUniform.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct LineUniform {
    camera_position: [f32; 4],
    screen: [f32; 4],
}

pub struct ThickLines {
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    /// Uploaded once by `set_static` - drawn first.
    static_buffer: Option<(wgpu::Buffer, u32)>,
    /// Refilled every frame by whoever draws with it - drawn after the
    /// static set, in order.
    pub dynamic: Vec<LineInstance>,
    dynamic_buffer: Option<(wgpu::Buffer, usize)>,
}

impl ThickLines {
    pub fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat, camera: &CameraResources) -> Self {
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Thick lines uniform"),
            contents: bytemuck::cast_slice(&[LineUniform { camera_position: [0.0; 4], screen: [1.0; 4] }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Thick lines uniform layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            }],
        });
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Thick lines uniform bind group"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() }],
        });

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Thick lines pipeline layout"),
            bind_group_layouts: &[&uniform_layout, &camera.bind_group_layout],
            push_constant_ranges: &[],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Thick lines shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/thick_lines.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Thick lines pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[LineInstance::layout()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: color_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                // Quads can come out either winding once projected.
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState { count: 1, mask: !0, alpha_to_coverage_enabled: false },
            multiview: None,
            cache: None,
        });

        Self { pipeline, uniform_buffer, uniform_bind_group, static_buffer: None, dynamic: Vec::new(), dynamic_buffer: None }
    }

    /// Replaces the static set - drawn every time, under `dynamic`. Empty
    /// clears it.
    pub fn set_static(&mut self, device: &wgpu::Device, lines: &[LineInstance]) {
        self.static_buffer = (!lines.is_empty()).then(|| {
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Thick lines static"),
                contents: bytemuck::cast_slice(lines),
                usage: wgpu::BufferUsages::VERTEX,
            });
            (buffer, lines.len() as u32)
        });
    }

    /// Draws the static set, then `dynamic`, into `render_pass` - seen from
    /// the camera at world `camera_position` whose view_proj is bound in
    /// `camera_bind_group`, on a screen `screen` pixels big.
    pub fn draw<'pass>(&'pass mut self, device: &wgpu::Device, queue: &wgpu::Queue, render_pass: &mut wgpu::RenderPass<'pass>, camera_bind_group: &'pass wgpu::BindGroup, camera_position: [f32; 3], screen: [f32; 2]) {
        let uniform = LineUniform { camera_position: [camera_position[0], camera_position[1], camera_position[2], 1.0], screen: [screen[0], screen[1], 0.0, 0.0] };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniform]));

        // The dynamic set's buffer, grown when it's outgrown.
        if !self.dynamic.is_empty() {
            let needed = self.dynamic.len();
            if self.dynamic_buffer.as_ref().is_none_or(|(_, capacity)| *capacity < needed) {
                let capacity = needed.next_power_of_two();
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Thick lines dynamic"),
                    size: (capacity * std::mem::size_of::<LineInstance>()) as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                self.dynamic_buffer = Some((buffer, capacity));
            }
            if let Some((buffer, _)) = &self.dynamic_buffer {
                queue.write_buffer(buffer, 0, bytemuck::cast_slice(&self.dynamic));
            }
        }

        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &self.uniform_bind_group, &[]);
        render_pass.set_bind_group(1, camera_bind_group, &[]);
        if let Some((buffer, count)) = &self.static_buffer {
            render_pass.set_vertex_buffer(0, buffer.slice(..));
            render_pass.draw(0..6, 0..*count);
        }
        if let (Some((buffer, _)), false) = (&self.dynamic_buffer, self.dynamic.is_empty()) {
            render_pass.set_vertex_buffer(0, buffer.slice(..));
            render_pass.draw(0..6, 0..self.dynamic.len() as u32);
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_shader_is_valid_wgsl() {
        let source = include_str!("../shaders/thick_lines.wgsl");
        let module = wgpu::naga::front::wgsl::parse_str(source).unwrap_or_else(|error| panic!("{}", error.emit_to_string(source)));
        wgpu::naga::valid::Validator::new(wgpu::naga::valid::ValidationFlags::all(), wgpu::naga::valid::Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|error| panic!("{error:?}"));
    }
}
