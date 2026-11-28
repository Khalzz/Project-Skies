/*
    Aerial perspective over the world (see aerial_perspective.wgsl): a frame plan step after the world is drawn,
    before it's copied onto the screen. The scene hands it the camera each frame (`set_view`, from after_cameras);
    the step reads the depth and the sun at draw time, draws the hazed world into its own image and copies that
    back over the world.
*/

use std::sync::Mutex;

use bytemuck::{Pod, Zeroable};
use nalgebra::Vector3;
use pankarta_software::prelude::{App, Scene};
use wgpu::util::DeviceExt;

use crate::prefabs::world::atmosphere::{Atmosphere, WORLD_NODE};

pub const STEP: &str = "aerial_perspective";

// Clear-day air: Rayleigh's sea-level extinction (red, green, blue per meter) over its 8 km scale height, and a
// thinner-topped haze (about 200 km visibility at sea level, 1.2 km scale height)
const RAYLEIGH: [f32; 4] = [5.8e-6, 13.5e-6, 33.1e-6, 8000.0];
const MIE: (f32, f32) = (2.0e-5, 1200.0);
// How thick the air is drawn, against the real one
const DENSITY: f32 = 1.0;
// The air's glow looking straight down, against the sky's at the same angle up: dimmer, so it tints blue
// instead of washing white
const DOWN_BRIGHTNESS: f32 = 0.55;
const DESATURATION: f32 = 0.6; // how grey fully thick air leaves what's behind it
const FARTHEST_SEA: f32 = 400_000.0; // m, where the sky's sea is taken to be when nothing was drawn there

// The engine's procedural sky (SkyboxRender::new_procedural, sky.wgsl)
const SKY_HORIZON: [f32; 4] = [0.342, 0.587, 0.711, 1.0];
const SKY_ZENITH: [f32; 4] = [0.014, 0.147, 0.430, 1.0];
const SKY_HAZE: [f32; 4] = [0.448, 0.604, 0.711, 0.6]; // w: how much of the haze band is on the horizon
const SKY_SUN: [f32; 4] = [1.0, 0.95, 0.85, 1.0];

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Params {
    ray_right: [f32; 4],
    ray_up: [f32; 4],
    ray_forward: [f32; 4],
    depth: [f32; 4],
    sun: [f32; 4],
    sun_color: [f32; 4],
    sky_horizon: [f32; 4],
    sky_zenith: [f32; 4],
    sky_haze: [f32; 4],
    rayleigh: [f32; 4],
    mie: [f32; 4],
    look: [f32; 4],
}

struct Pipeline {
    format: wgpu::TextureFormat,
    layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    hazed: Option<wgpu::Texture>, // the world with the air over it, copied back onto the world
}

static VIEW: Mutex<Option<Params>> = Mutex::new(None);
static PIPELINE: Mutex<Option<Pipeline>> = Mutex::new(None);

// This frame's camera: where it looks, how wide, how high above the sea
pub fn set_view(scene: &Scene, app: &App) {
    let camera = scene.cameras.active();
    if camera.is_orthographic() {
        *VIEW.lock().unwrap() = None;
        return;
    }
    let forward = camera.forward();
    let right = forward.cross(&camera.up()).normalize();
    let up = right.cross(&forward);
    let size = app.window.pixel_size;
    let aspect = size.width as f32 / (size.height as f32).max(1.0);
    let tan_y = (camera.fovy().to_radians() * 0.5).tan();
    let projection = camera.projection_matrix();

    let sea_level = Atmosphere::from_world(scene.get_node(WORLD_NODE), &app.origin).sea_level;
    let height = camera.position().y - sea_level;

    let vector = |v: Vector3<f32>, w: f32| [v.x, v.y, v.z, w];
    *VIEW.lock().unwrap() = Some(Params {
        ray_right: vector(right * tan_y * aspect, 0.0),
        ray_up: vector(up * tan_y, 0.0),
        ray_forward: vector(forward, height),
        depth: [projection[(2, 2)], projection[(2, 3)], FARTHEST_SEA, 0.0],
        sun: [0.0; 4],
        sun_color: SKY_SUN,
        sky_horizon: SKY_HORIZON,
        sky_zenith: SKY_ZENITH,
        sky_haze: SKY_HAZE,
        rayleigh: [RAYLEIGH[0] * DENSITY, RAYLEIGH[1] * DENSITY, RAYLEIGH[2] * DENSITY, RAYLEIGH[3]],
        mie: [MIE.0 * DENSITY, MIE.1, 0.0, 0.0],
        look: [DOWN_BRIGHTNESS, DESATURATION, 0.0, 0.0],
    });
}

// The frame plan step: over the world drawn so far, before it's copied onto the screen
pub fn draw(app: &mut App, encoder: &mut wgpu::CommandEncoder, _view: &wgpu::TextureView) {
    let Some(mut params) = *VIEW.lock().unwrap() else { return };
    let renderer = &app.renderer;
    let sun = Vector3::from(renderer.light.uniform.position).normalize();
    params.sun = [sun.x, sun.y, sun.z, 0.0];

    // Everything drawn so far, water included
    renderer.depth_render.snapshot_depth(encoder);

    let world = &renderer.blur.scene_color.texture;
    let format = world.format();
    let size = world.size();
    let mut state = PIPELINE.lock().unwrap();
    if state.as_ref().is_none_or(|state| state.format != format) {
        *state = Some(build(&renderer.device, format));
    }
    let Some(state) = state.as_mut() else { return };
    if state.hazed.as_ref().is_none_or(|hazed| hazed.size() != size) {
        state.hazed = Some(renderer.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(STEP),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        }));
    }
    let Some(hazed) = state.hazed.as_ref() else { return };

    let buffer = renderer.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(STEP),
        contents: bytemuck::bytes_of(&params),
        usage: wgpu::BufferUsages::UNIFORM,
    });
    let bind_group = renderer.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(STEP),
        layout: &state.layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&renderer.depth_render.foam_depth_copy.view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&renderer.blur.scene_color.view),
            },
        ],
    });

    let hazed_view = hazed.create_view(&wgpu::TextureViewDescriptor::default());
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(STEP),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &hazed_view,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        pass.set_pipeline(&state.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
    encoder.copy_texture_to_texture(hazed.as_image_copy(), world.as_image_copy(), size);
}

fn build(device: &wgpu::Device, format: wgpu::TextureFormat) -> Pipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(STEP),
        source: wgpu::ShaderSource::Wgsl(include_str!("aerial_perspective.wgsl").into()),
    });
    let texture = |binding: u32, sample_type: wgpu::TextureSampleType| wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            multisampled: false,
            view_dimension: wgpu::TextureViewDimension::D2,
            sample_type,
        },
        count: None,
    };
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some(STEP),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            texture(1, wgpu::TextureSampleType::Depth),
            texture(2, wgpu::TextureSampleType::Float { filterable: false }),
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(STEP),
        bind_group_layouts: &[&layout],
        push_constant_ranges: &[],
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(STEP),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    });

    Pipeline { format, layout, pipeline, hazed: None }
}
