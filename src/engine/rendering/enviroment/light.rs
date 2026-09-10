use wgpu::{util::DeviceExt, BindGroup, BindGroupLayout, Buffer, Device, RenderPipeline, SurfaceConfiguration};

use crate::engine::rendering::{camera::handler::CameraResources, models::{model::{self, Vertex}, textures::Texture}, ui::rendering_utils};

// Byte layout has to match WGSL's own struct-member-offset rule exactly
// (roundUp(align(member), offset(end of previous member)) - see
// https://gpuweb.github.io/gpuweb/wgsl/#structure-member-layout): vec3<f32>
// aligns to 16, but a scalar right after one packs in immediately rather
// than waiting for the next 16-byte boundary (that's std140's rule, not
// WGSL's) - position(0..12)+_padding(12..16), color(16..28), time(28..32),
// camera_position(32..44)+_padding2(44..48). camera_position starts exactly
// on a 16-byte boundary (32) so it needs no extra padding before it either;
// _padding2 exists only because the struct's own alignment (16, from the
// vec3s) requires the total size to be a multiple of 16, and 44 isn't.
// Getting this wrong doesn't error - the shader just silently reads whatever
// raw bytes land at the offset it expects instead of the real value - burned
// once already (see git history), so any new field added here needs the
// same offset check.
#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LightUniform {
    pub position: [f32; 3],
    pub _padding: u32,
    pub color: [f32; 3],
    // Seconds of elapsed animation time - piggybacks on this buffer purely
    // because it's already bound (group 3) to every model-drawing pipeline
    // and rewritten once a frame (see App::run's own "lighting update"
    // block); water.wgsl is the only shader that reads it today, but it's
    // not otherwise light-specific. Devices here cap bind groups at 4 (see
    // WaterRenderData's own doc comment for why a dedicated 5th group isn't
    // an option), so reusing an existing one was the only way to get a
    // per-frame value into a second pipeline without also touching every
    // other pipeline's layout.
    pub time: f32,
    // Camera's true world-space position, same piggyback trick as `time`.
    // water.wgsl's vertex model matrix already bakes in camera-relative
    // translation (see GameObject::to_raw), so this is what lets the wave
    // function recover true world position instead of sampling relative to
    // wherever the camera happens to be.
    pub camera_position: [f32; 3],
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
    pub fn new(device: &Device, config: &SurfaceConfiguration, camera: &CameraResources) -> Self {
        let uniform = LightUniform {
            position: [0.0, 5.0, 0.0],
            _padding: 0,
            color: [1.0, 1.0, 1.0],
            time: 0.0,
            camera_position: [0.0, 0.0, 0.0],
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
                source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/light.wgsl").into()),
            };
            
            rendering_utils::create_render_pipeline(
                &device,
                &layout,
                config.format,
                Some(Texture::DEPTH_FORMAT),
                &[model::ModelVertex::desc()],
                shader,
                Some(wgpu::Face::Back),
                true,
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