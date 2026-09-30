//! The engine side of particles and trails - simulates and draws whatever
//! the scene's nodes' `ParticleEmitters` ask for.
//!
//! PARTICLES live on the GPU: one big buffer of particle slots, each
//! particle emitter owning a fixed ring of them (`max_particles`, the oldest
//! replaced when full - no free-list bookkeeping). Spawning happens here on
//! the CPU (new particles are written straight into the ring), everything
//! after that on the GPU: a compute pass (particles_update.wgsl) ages and
//! moves them every frame, and particles.wgsl draws each as a quad, read
//! straight from the buffer.
//!
//! TRAILS are few (tens at most), so their points stay here on the CPU and
//! are rebuilt into camera-facing ribbons each frame (trails.wgsl).
//!
//! Everything is CAMERA-RELATIVE, like the rest of the renderer: trail
//! points are kept in f64 world space and converted each frame; particles
//! are shifted by however far the camera moved, in the compute pass. That
//! keeps both precise however far from the world origin you fly.
//!
//! New particles are spread across the frame - positioned between where the
//! emitter was last frame and where it is now, and aged to match - so a
//! plane at 700 knots (~6 m per frame) leaves a smooth stream, not clumps.

use std::collections::{HashMap, HashSet, VecDeque};

use nalgebra::{Matrix4, UnitQuaternion, Vector3};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use wgpu::util::DeviceExt;

use super::effect::{Blend, EmissionShape, Facing, ParticleEffect, ParticleShape, TrailEffect, TrailFacing, WaterResponse};
use super::emitter::{Emitter, EmitterKind};
use crate::engine::rendering::models::textures::Texture;

/// Particle slots across every emitter (64 bytes each - 4 MB).
const PARTICLE_CAPACITY: u32 = 1 << 16;
/// The most one particle emitter can hold.
const MAX_PARTICLES_PER_EMITTER: u32 = 16384;
/// Particle emitters alive at once (their settings live in a GPU table).
const MAX_EMITTERS: u32 = 256;
/// The most points one trail keeps.
const MAX_TRAIL_POINTS: usize = 4096;
const WORKGROUP_SIZE: u32 = 64;
/// Must match the camera projection's far plane (see camera/handler.rs) -
/// the soft fade's depth comparison depends on it.
const FAR_PLANE: f32 = 4_000_000.0;
/// How long an emitter's velocity is averaged over (s) - long enough to
/// smooth out physics steps landing unevenly across rendered frames, short
/// enough to follow real speed changes.
const VELOCITY_SMOOTHING_SECONDS: f32 = 0.08;
/// Sky light on lit particles, on top of the sun's.
const AMBIENT: f32 = 0.45;

// ── GPU layouts (must match particles_update.wgsl / particles.wgsl / trails.wgsl) ──

#[repr(C)]
#[derive(Copy, Clone, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuParticle {
    position: [f32; 3],
    age: f32,
    velocity: [f32; 3],
    lifetime: f32,
    rotation: f32,
    spin: f32,
    seed: f32,
    intensity: f32,
    emitter: u32,
    /// The emitter's velocity at spawn (see Facing::Velocity).
    carrier: [f32; 3],
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuEmitter {
    forces: [f32; 4],
    look: [f32; 4],
    shape: [f32; 4],
    flags: [f32; 4],
    scales: [f32; 4],
    size: [[f32; 4]; 2],
    color: [[f32; 4]; 8],
}

#[repr(C)]
#[derive(Copy, Clone, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuFrame {
    camera_right: [f32; 4],
    camera_up: [f32; 4],
    camera_forward: [f32; 4],
    camera_delta: [f32; 4],
    wind: [f32; 4],
    sun_direction: [f32; 4],
    sun_color: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct TrailVertex {
    position: [f32; 3],
    uv: [f32; 2],
    color: [f32; 4],
    params: [f32; 4],
}

impl TrailVertex {
    const ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x4, 3 => Float32x4];

    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<TrailVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBUTES,
        }
    }
}

impl GpuEmitter {
    fn from_effect(effect: &ParticleEffect) -> Self {
        let look = &effect.look;
        let (facing, stretch) = match look.facing {
            Facing::Camera => (0.0, 0.0),
            Facing::Velocity { stretch } => (1.0, stretch),
            Facing::Horizontal => (2.0, 0.0),
        };
        let shape = match &look.shape {
            ParticleShape::SoftCircle => [0.0, 1.0, 1.0, 0.0],
            ParticleShape::Texture(_) => [1.0, 1.0, 1.0, 0.0],
            ParticleShape::Flipbook { columns, rows, fps, .. } => [2.0, (*columns).max(1) as f32, (*rows).max(1) as f32, *fps],
            ParticleShape::Faceted { sides } => [3.0, (*sides).clamp(3, 32) as f32, 1.0, 0.0],
        };
        let (water, restitution) = match effect.collision.water {
            WaterResponse::None => (0.0, 0.0),
            WaterResponse::Die => (1.0, 0.0),
            WaterResponse::Bounce { restitution } => (2.0, restitution),
            WaterResponse::Float => (3.0, 0.0),
        };
        let flag = |on: bool| if on { 1.0 } else { 0.0 };
        let mut size = [[0.0; 4]; 2];
        let mut color = [[0.0; 4]; 8];
        for i in 0..8 {
            let t = i as f32 / 7.0;
            size[i / 4][i % 4] = look.size.sample(t);
            color[i] = look.color.sample(t);
        }
        let forces = &effect.forces;
        Self {
            forces: [forces.gravity, forces.drag, forces.wind, forces.turbulence],
            look: [look.brightness, facing, stretch, look.soft_fade_distance.max(0.0)],
            shape,
            flags: [flag(look.lit), water, restitution, 0.0],
            scales: [flag(effect.intensity_scales.alpha), flag(effect.intensity_scales.size), 0.0, 0.0],
            size,
            color,
        }
    }
}

// ── Public API ─────────────────────────────────────────────────────────────

/// What `ParticleRenderer::update` needs to know about this frame.
pub struct ParticleFrameInfo {
    pub camera_position: Vector3<f32>,
    /// The camera's view matrix (`Camera::calc_matrix` - camera at origin).
    pub camera_view: Matrix4<f32>,
    pub near: f32,
    /// 0 while paused - everything holds still.
    pub delta_time: f32,
    pub sea_level: f32,
    /// World wind velocity (m/s).
    pub wind: Vector3<f32>,
    /// Toward the sun.
    pub sun_direction: Vector3<f32>,
    pub sun_color: Vector3<f32>,
}

/// One emitter as it is this frame - which node it's on, and where that
/// node is.
pub struct EmitterSource<'a> {
    pub node_id: &'a str,
    pub name: &'a str,
    pub emitter: &'a Emitter,
    pub position: Vector3<f32>,
    pub rotation: UnitQuaternion<f32>,
}

// ── Per-emitter state ──────────────────────────────────────────────────────

struct TrailPoint {
    position: Vector3<f64>,
    /// Its own motion (world, m/s) - `TrailEffect::point_velocity` as it
    /// was when it was laid.
    velocity: Vector3<f32>,
    /// Counts up point by point along the trail - a coordinate along it
    /// that sticks to each point (for the shader's patterns).
    serial: f32,
    age: f32,
    intensity: f32,
    /// Connected to the next OLDER point (false where the trail was
    /// interrupted - the emitter was disabled for a while).
    joined: bool,
}

#[derive(Default)]
struct TrailState {
    /// Newest first.
    points: VecDeque<TrailPoint>,
    effect: Option<TrailEffect>,
    /// Where the emitter is right now, while it's laying trail.
    head: Option<(Vector3<f64>, f32)>,
    /// Where the emitter was when it laid the newest point - the next one is
    /// spaced from here.
    last_laid: Option<Vector3<f64>>,
    /// The next point's `serial` (wrapped, to stay precise).
    next_serial: f32,
}

struct EmitterRuntime {
    seen: bool,
    is_trail: bool,
    blend: Blend,
    sprite: Option<String>,
    /// When the last thing it put out will be gone (s, renderer time).
    alive_until: f32,
    // Particles.
    slot: Option<u32>,
    range: Option<(u32, u32)>,
    cursor: u32,
    rate_carry: f32,
    distance_carry: f32,
    burst_clocks: Vec<Option<f32>>,
    previous: Option<(Vector3<f32>, UnitQuaternion<f32>)>,
    /// Smoothed emitter velocity - see VELOCITY_SMOOTHING_SECONDS.
    velocity: Option<Vector3<f32>>,
    // Trails.
    trail: TrailState,
}

impl EmitterRuntime {
    fn new(is_trail: bool) -> Self {
        Self {
            seen: false,
            is_trail,
            blend: Blend::Alpha,
            sprite: None,
            alive_until: 0.0,
            slot: None,
            range: None,
            cursor: 0,
            rate_carry: 0.0,
            distance_carry: 0.0,
            burst_clocks: Vec::new(),
            previous: None,
            velocity: None,
            trail: TrailState::default(),
        }
    }
}

struct ParticleDraw {
    blend: Blend,
    sprite: Option<String>,
    first: u32,
    count: u32,
}

struct TrailDraw {
    blend: Blend,
    sprite: Option<String>,
    indices: std::ops::Range<u32>,
}

pub struct ParticleRenderer {
    particle_buffer: wgpu::Buffer,
    emitter_buffer: wgpu::Buffer,
    frame_buffer: wgpu::Buffer,

    compute_pipeline: wgpu::ComputePipeline,
    compute_bind_group: wgpu::BindGroup,
    particle_bind_group: wgpu::BindGroup,
    trail_frame_bind_group: wgpu::BindGroup,
    depth_layout: wgpu::BindGroupLayout,
    depth_bind_group: wgpu::BindGroup,
    sprite_layout: wgpu::BindGroupLayout,
    sprite_sampler: wgpu::Sampler,
    white_sprite: wgpu::BindGroup,
    sprites: HashMap<String, wgpu::BindGroup>,
    failed_sprites: HashSet<String>,
    particle_pipelines: [wgpu::RenderPipeline; 2],
    trail_pipelines: [wgpu::RenderPipeline; 2],

    trail_vertex_buffer: wgpu::Buffer,
    trail_index_buffer: wgpu::Buffer,
    trail_vertex_capacity: usize,
    trail_index_capacity: usize,

    free_ranges: Vec<(u32, u32)>,
    free_slots: Vec<u32>,
    runtimes: HashMap<(String, String), EmitterRuntime>,
    warned: HashSet<(String, String)>,
    rng: StdRng,
    previous_camera: Option<Vector3<f64>>,
    time: f32,
    /// Set by `update`, cleared by `simulate`.
    simulation_pending: bool,

    particle_draws: Vec<ParticleDraw>,
    trail_draws: Vec<TrailDraw>,
}

fn blend_state(blend: Blend) -> wgpu::BlendState {
    // Shaders output premultiplied color.
    match blend {
        Blend::Additive => wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
        },
        Blend::Alpha => wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING,
    }
}

fn blend_index(blend: Blend) -> usize {
    match blend {
        Blend::Alpha => 0,
        Blend::Additive => 1,
    }
}

fn storage_entry(binding: u32, visibility: wgpu::ShaderStages, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only }, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    }
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    }
}

impl ParticleRenderer {
    /// `depth_view` is the scene-depth snapshot soft particles fade against
    /// (DepthRender::foam_depth_copy - see `App::render_particle_pass`).
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, color_format: wgpu::TextureFormat, camera_layout: &wgpu::BindGroupLayout, depth_view: &wgpu::TextureView) -> Self {
        let particle_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particles"),
            size: PARTICLE_CAPACITY as u64 * std::mem::size_of::<GpuParticle>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let emitter_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("particle_emitters"),
            size: MAX_EMITTERS as u64 * std::mem::size_of::<GpuEmitter>() as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let frame_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("particle_frame"),
            contents: bytemuck::bytes_of(&GpuFrame::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // ── Compute (update) ──
        let compute_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle_update_layout"),
            entries: &[
                storage_entry(0, wgpu::ShaderStages::COMPUTE, false),
                storage_entry(1, wgpu::ShaderStages::COMPUTE, true),
                uniform_entry(2, wgpu::ShaderStages::COMPUTE),
            ],
        });
        let compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particle_update_bind_group"),
            layout: &compute_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: particle_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: emitter_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: frame_buffer.as_entire_binding() },
            ],
        });
        let compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Particle Update Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/particles_update.wgsl").into()),
        });
        let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("particle_update_pipeline"),
            layout: Some(&device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("particle_update_pipeline_layout"),
                bind_group_layouts: &[&compute_layout],
                push_constant_ranges: &[],
            })),
            module: &compute_shader,
            entry_point: Some("update"),
            compilation_options: Default::default(),
            cache: None,
        });

        // ── Shared render layouts ──
        let vertex_fragment = wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT;
        let particle_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle_render_layout"),
            entries: &[
                storage_entry(0, wgpu::ShaderStages::VERTEX, true),
                storage_entry(1, vertex_fragment, true),
                uniform_entry(2, vertex_fragment),
            ],
        });
        let particle_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particle_render_bind_group"),
            layout: &particle_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: particle_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: emitter_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: frame_buffer.as_entire_binding() },
            ],
        });
        let trail_frame_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("trail_frame_layout"),
            entries: &[uniform_entry(0, vertex_fragment)],
        });
        let trail_frame_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("trail_frame_bind_group"),
            layout: &trail_frame_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: frame_buffer.as_entire_binding() }],
        });
        let depth_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle_depth_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                },
                count: None,
            }],
        });
        let depth_bind_group = Self::make_depth_bind_group(device, &depth_layout, depth_view);
        let sprite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("particle_sprite_layout"),
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
            ],
        });
        let sprite_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("particle_sprite_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let white = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 255, 255, 255])));
        let white_texture = Texture::from_image(&white, device, queue, Some("particle_white")).expect("1x1 white texture");
        let white_sprite = Self::make_sprite_bind_group(device, &sprite_layout, &sprite_sampler, &white_texture);

        // ── Render pipelines ──
        let depth_stencil = wgpu::DepthStencilState {
            format: Texture::DEPTH_FORMAT,
            // Tested against the scene (reversed-Z: nearer = greater), never written.
            depth_write_enabled: false,
            depth_compare: wgpu::CompareFunction::Greater,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        };
        let make_pipeline = |label: &str, shader: &wgpu::ShaderModule, layout: &wgpu::PipelineLayout, buffers: &[wgpu::VertexBufferLayout], blend: Blend| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState { module: shader, entry_point: Some("vs_main"), buffers, compilation_options: Default::default() },
                fragment: Some(wgpu::FragmentState {
                    module: shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState { format: color_format, blend: Some(blend_state(blend)), write_mask: wgpu::ColorWrites::ALL })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                depth_stencil: Some(depth_stencil.clone()),
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            })
        };
        let particle_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Particle Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/particles.wgsl").into()),
        });
        let particle_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("particle_pipeline_layout"),
            bind_group_layouts: &[&particle_layout, camera_layout, &depth_layout, &sprite_layout],
            push_constant_ranges: &[],
        });
        let particle_pipelines = [Blend::Alpha, Blend::Additive].map(|blend| make_pipeline("particle_pipeline", &particle_shader, &particle_pipeline_layout, &[], blend));
        let trail_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Trail Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/trails.wgsl").into()),
        });
        let trail_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("trail_pipeline_layout"),
            bind_group_layouts: &[&trail_frame_layout, camera_layout, &depth_layout, &sprite_layout],
            push_constant_ranges: &[],
        });
        let trail_pipelines = [Blend::Alpha, Blend::Additive].map(|blend| make_pipeline("trail_pipeline", &trail_shader, &trail_pipeline_layout, &[TrailVertex::layout()], blend));

        let trail_vertex_capacity = 4096;
        let trail_index_capacity = 4096 * 3;
        let (trail_vertex_buffer, trail_index_buffer) = Self::make_trail_buffers(device, trail_vertex_capacity, trail_index_capacity);

        Self {
            particle_buffer,
            emitter_buffer,
            frame_buffer,
            compute_pipeline,
            compute_bind_group,
            particle_bind_group,
            trail_frame_bind_group,
            depth_layout,
            depth_bind_group,
            sprite_layout,
            sprite_sampler,
            white_sprite,
            sprites: HashMap::new(),
            failed_sprites: HashSet::new(),
            particle_pipelines,
            trail_pipelines,
            trail_vertex_buffer,
            trail_index_buffer,
            trail_vertex_capacity,
            trail_index_capacity,
            free_ranges: vec![(0, PARTICLE_CAPACITY)],
            free_slots: (0..MAX_EMITTERS).rev().collect(),
            runtimes: HashMap::new(),
            warned: HashSet::new(),
            rng: StdRng::from_entropy(),
            previous_camera: None,
            time: 0.0,
            simulation_pending: false,
            particle_draws: Vec::new(),
            trail_draws: Vec::new(),
        }
    }

    fn make_depth_bind_group(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, depth_view: &wgpu::TextureView) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particle_depth_bind_group"),
            layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(depth_view) }],
        })
    }

    fn make_sprite_bind_group(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, sampler: &wgpu::Sampler, texture: &Texture) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("particle_sprite_bind_group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&texture.view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
            ],
        })
    }

    fn make_trail_buffers(device: &wgpu::Device, vertices: usize, indices: usize) -> (wgpu::Buffer, wgpu::Buffer) {
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("trail_vertices"),
            size: (vertices * std::mem::size_of::<TrailVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("trail_indices"),
            size: (indices * std::mem::size_of::<u32>()) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        (vertex_buffer, index_buffer)
    }

    /// The scene-depth snapshot is rebuilt on resize - rebind it.
    pub fn rebuild_depth_bind_group(&mut self, device: &wgpu::Device, depth_view: &wgpu::TextureView) {
        self.depth_bind_group = Self::make_depth_bind_group(device, &self.depth_layout, depth_view);
    }

    /// Anything to simulate or draw this frame.
    pub fn has_work(&self) -> bool {
        !self.particle_draws.is_empty() || !self.trail_draws.is_empty()
    }

    // ── Per frame ──

    /// Spawns, ages and lays trail for every emitter in `sources` (each
    /// node's `ParticleEmitters`, this frame), lets go of the ones that are
    /// gone once what they put out has died, and prepares this frame's
    /// draws. Call once per frame, before rendering.
    pub fn update<'a>(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &ParticleFrameInfo, sources: impl IntoIterator<Item = EmitterSource<'a>>) {
        let dt = frame.delta_time.max(0.0);
        self.time += dt;
        let camera = frame.camera_position.cast::<f64>();
        let previous_camera = self.previous_camera.unwrap_or(camera);
        self.previous_camera = Some(camera);

        for runtime in self.runtimes.values_mut() {
            runtime.seen = false;
        }

        for source in sources {
            let key = (source.node_id.to_owned(), source.name.to_owned());
            let is_trail = matches!(source.emitter.kind, EmitterKind::Trail(_));
            let mut runtime = match self.runtimes.remove(&key) {
                Some(runtime) if runtime.is_trail == is_trail => runtime,
                Some(mut stale) => {
                    self.release(queue, &mut stale);
                    EmitterRuntime::new(is_trail)
                }
                None => EmitterRuntime::new(is_trail),
            };
            runtime.seen = true;
            match &source.emitter.kind {
                EmitterKind::Particles(effect) => self.update_particle_emitter(device, queue, &key, &mut runtime, &source, effect, dt, previous_camera),
                EmitterKind::Trail(effect) => self.update_trail_emitter(device, queue, &mut runtime, &source, effect, dt, frame.wind),
            }
            self.runtimes.insert(key, runtime);
        }

        // Emitters that are gone (or whose node is): no more spawning, but
        // what they put out lives out its life first.
        let time = self.time;
        let finished: Vec<(String, String)> = self.runtimes.iter_mut()
            .filter(|(_, runtime)| !runtime.seen)
            .filter_map(|(key, runtime)| {
                if runtime.is_trail {
                    runtime.trail.head = None;
                    if let Some(effect) = runtime.trail.effect.clone() {
                        Self::age_trail(&mut runtime.trail, &effect, dt, frame.wind);
                    }
                    runtime.trail.points.is_empty().then(|| key.clone())
                } else {
                    (time >= runtime.alive_until).then(|| key.clone())
                }
            })
            .collect();
        for key in finished {
            if let Some(mut runtime) = self.runtimes.remove(&key) {
                self.release(queue, &mut runtime);
            }
        }

        // Only emitters with something possibly still alive - an idle one
        // (e.g. wreck smoke before any crash) costs nothing.
        self.particle_draws = self.runtimes.values()
            .filter(|runtime| !runtime.is_trail && time < runtime.alive_until)
            .filter_map(|runtime| runtime.range.map(|(first, count)| ParticleDraw { blend: runtime.blend, sprite: runtime.sprite.clone(), first, count }))
            .collect();
        self.build_trails(device, queue, frame);

        let view = frame.camera_view;
        let right = Vector3::new(view[(0, 0)], view[(0, 1)], view[(0, 2)]);
        let up = Vector3::new(view[(1, 0)], view[(1, 1)], view[(1, 2)]);
        let forward = -Vector3::new(view[(2, 0)], view[(2, 1)], view[(2, 2)]);
        let camera_delta = (camera - previous_camera).cast::<f32>();
        let sun = frame.sun_direction.try_normalize(1e-6).unwrap_or_else(Vector3::y);
        let gpu_frame = GpuFrame {
            camera_right: [right.x, right.y, right.z, frame.near],
            camera_up: [up.x, up.y, up.z, FAR_PLANE],
            // Wrapped so it stays precise over a long session (it only feeds turbulence).
            camera_forward: [forward.x, forward.y, forward.z, self.time % 1000.0],
            camera_delta: [camera_delta.x, camera_delta.y, camera_delta.z, dt],
            wind: [frame.wind.x, frame.wind.y, frame.wind.z, frame.sea_level - frame.camera_position.y],
            sun_direction: [sun.x, sun.y, sun.z, AMBIENT],
            sun_color: [frame.sun_color.x, frame.sun_color.y, frame.sun_color.z, 0.0],
        };
        queue.write_buffer(&self.frame_buffer, 0, bytemuck::bytes_of(&gpu_frame));
        self.simulation_pending = true;
    }

    fn warn_once(&mut self, key: &(String, String), message: &str) {
        if self.warned.insert(key.clone()) {
            eprintln!("particles: emitter '{}' on '{}': {message}", key.1, key.0);
        }
    }

    /// Gives back an emitter's particle slots (clearing them, so nothing it
    /// left there lingers) and its settings slot.
    fn release(&mut self, queue: &wgpu::Queue, runtime: &mut EmitterRuntime) {
        if let Some((first, count)) = runtime.range.take() {
            let zeros = vec![GpuParticle::default(); count as usize];
            queue.write_buffer(&self.particle_buffer, first as u64 * std::mem::size_of::<GpuParticle>() as u64, bytemuck::cast_slice(&zeros));
            self.free_range(first, count);
        }
        if let Some(slot) = runtime.slot.take() {
            self.free_slots.push(slot);
        }
    }

    fn allocate_range(&mut self, count: u32) -> Option<(u32, u32)> {
        let index = self.free_ranges.iter().position(|(_, free)| *free >= count)?;
        let (first, free) = self.free_ranges[index];
        if free == count {
            self.free_ranges.remove(index);
        } else {
            self.free_ranges[index] = (first + count, free - count);
        }
        Some((first, count))
    }

    fn free_range(&mut self, first: u32, count: u32) {
        self.free_ranges.push((first, count));
        self.free_ranges.sort_by_key(|(start, _)| *start);
        let mut merged: Vec<(u32, u32)> = Vec::with_capacity(self.free_ranges.len());
        for (start, length) in self.free_ranges.drain(..) {
            match merged.last_mut() {
                Some((last_start, last_length)) if *last_start + *last_length == start => *last_length += length,
                _ => merged.push((start, length)),
            }
        }
        self.free_ranges = merged;
    }

    fn ensure_sprite(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, path: &str) {
        if self.sprites.contains_key(path) || self.failed_sprites.contains(path) {
            return;
        }
        let texture = crate::resources::load_asset_binary(path)
            .map_err(|error| error.to_string())
            .and_then(|bytes| image::load_from_memory(&bytes).map_err(|error| error.to_string()))
            .and_then(|image| Texture::from_image(&image, device, queue, Some(path)).map_err(|error| error.to_string()));
        match texture {
            Ok(texture) => {
                let bind_group = Self::make_sprite_bind_group(device, &self.sprite_layout, &self.sprite_sampler, &texture);
                self.sprites.insert(path.to_owned(), bind_group);
            }
            Err(error) => {
                eprintln!("particles: couldn't load sprite 'assets/{path}': {error} - drawing without it");
                self.failed_sprites.insert(path.to_owned());
            }
        }
    }

    fn sprite(&self, path: &Option<String>) -> &wgpu::BindGroup {
        path.as_ref().and_then(|path| self.sprites.get(path)).unwrap_or(&self.white_sprite)
    }

    // ── Particles ──

    #[allow(clippy::too_many_arguments)]
    fn update_particle_emitter(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, key: &(String, String), runtime: &mut EmitterRuntime, source: &EmitterSource, effect: &ParticleEffect, dt: f32, previous_camera: Vector3<f64>) {
        // A ring sized to the effect - reallocated if that changes.
        let max = effect.max_particles.clamp(1, MAX_PARTICLES_PER_EMITTER);
        if runtime.range.map_or(true, |(_, count)| count != max) {
            if let Some((first, count)) = runtime.range.take() {
                let zeros = vec![GpuParticle::default(); count as usize];
                queue.write_buffer(&self.particle_buffer, first as u64 * std::mem::size_of::<GpuParticle>() as u64, bytemuck::cast_slice(&zeros));
                self.free_range(first, count);
            }
            runtime.range = self.allocate_range(max);
            runtime.cursor = 0;
            if runtime.range.is_none() {
                self.warn_once(key, "no room left in the particle pool - not emitting");
                return;
            }
        }
        if runtime.slot.is_none() {
            runtime.slot = self.free_slots.pop();
        }
        let (Some(slot), Some((first, count))) = (runtime.slot, runtime.range) else {
            self.warn_once(key, "too many particle emitters at once - not emitting");
            return;
        };
        queue.write_buffer(&self.emitter_buffer, slot as u64 * std::mem::size_of::<GpuEmitter>() as u64, bytemuck::bytes_of(&GpuEmitter::from_effect(effect)));
        runtime.blend = effect.look.blend;
        runtime.sprite = match &effect.look.shape {
            ParticleShape::SoftCircle | ParticleShape::Faceted { .. } => None,
            ParticleShape::Texture(path) | ParticleShape::Flipbook { path, .. } => Some(path.clone()),
        };
        if let Some(path) = runtime.sprite.clone() {
            self.ensure_sprite(device, queue, &path);
        }

        let emitter = source.emitter;
        let current = (source.position, source.rotation);
        let previous = runtime.previous.unwrap_or(current);
        runtime.previous = Some(current);
        // The emitter's velocity, smoothed: a node moved by physics only
        // moves when a physics step lands (120 Hz), not every rendered
        // frame - measured frame to frame it flickers between 0 and ~2x
        // its real speed, and particles inheriting that would shoot ahead.
        if dt > 0.0 {
            let measured = (current.0 - previous.0) / dt;
            let blend = 1.0 - (-dt / VELOCITY_SMOOTHING_SECONDS).exp();
            runtime.velocity = Some(runtime.velocity.map_or(measured, |velocity| velocity.lerp(&measured, blend)));
        }

        // How many this frame.
        let intensity = emitter.intensity.clamp(0.0, 1.0);
        let rate_scale = if effect.intensity_scales.rate { intensity } else { 1.0 };
        let mut spawn_count = 0.0f32;
        if emitter.enabled && dt > 0.0 {
            runtime.rate_carry += effect.spawn.rate * rate_scale * dt;
            runtime.distance_carry += effect.spawn.per_meter * rate_scale * (current.0 - previous.0).magnitude();
            spawn_count += runtime.rate_carry.floor() + runtime.distance_carry.floor();
            runtime.rate_carry = runtime.rate_carry.fract();
            runtime.distance_carry = runtime.distance_carry.fract();

            if runtime.burst_clocks.len() != effect.spawn.bursts.len() {
                runtime.burst_clocks = vec![None; effect.spawn.bursts.len()];
            }
            for (burst, clock) in effect.spawn.bursts.iter().zip(runtime.burst_clocks.iter_mut()) {
                let fire = match clock {
                    None => {
                        *clock = Some(0.0);
                        true
                    }
                    Some(elapsed) => {
                        *elapsed += dt;
                        match burst.every {
                            Some(every) if every > 0.0 && *elapsed >= every => {
                                *elapsed -= every;
                                true
                            }
                            _ => false,
                        }
                    }
                };
                if fire {
                    spawn_count += (burst.count as f32 * rate_scale).round();
                }
            }
        } else if !emitter.enabled {
            // Re-enabling fires the bursts again.
            runtime.burst_clocks.iter_mut().for_each(|clock| *clock = None);
            runtime.rate_carry = 0.0;
            runtime.distance_carry = 0.0;
        }
        let spawn_count = (spawn_count as u32).min(count);
        if spawn_count == 0 {
            return;
        }

        let node_velocity = runtime.velocity.unwrap_or_else(Vector3::zeros);
        let speed_scale = if effect.intensity_scales.speed { intensity } else { 1.0 };
        let direction = effect.motion.direction.try_normalize(1e-6).unwrap_or_else(Vector3::z);
        let mut spawned = Vec::with_capacity(spawn_count as usize);
        for index in 0..spawn_count {
            // Spread across the frame (see the module doc).
            let f = (index as f32 + self.rng.gen::<f32>()) / spawn_count as f32;
            let node_position = previous.0.lerp(&current.0, f);
            let node_rotation = previous.1.slerp(&current.1, f);
            let frame_rotation = node_rotation * emitter.rotation;
            let origin = node_position + node_rotation * emitter.offset;
            let position = origin + frame_rotation * sample_shape(&effect.emission_shape, &mut self.rng);
            let spread = cone_direction(direction, effect.motion.cone_angle, &mut self.rng);
            let launch = if effect.motion.world_direction { spread } else { frame_rotation * spread };
            let inherited = node_velocity * effect.motion.inherit_velocity;
            let velocity = launch * effect.motion.speed.sample(&mut self.rng) * speed_scale + inherited;
            // Relative to LAST frame's camera - the update pass moves every
            // particle by this frame's camera motion, new ones included.
            let relative = (position.cast::<f64>() - previous_camera).cast::<f32>();
            spawned.push(GpuParticle {
                position: relative.into(),
                age: -f * dt,
                velocity: velocity.into(),
                lifetime: effect.spawn.lifetime.sample(&mut self.rng).max(0.01),
                rotation: self.rng.gen_range(0.0..std::f32::consts::TAU),
                spin: effect.motion.spin.sample(&mut self.rng).to_radians(),
                seed: self.rng.gen(),
                intensity,
                emitter: slot,
                // Streaks are drawn relative to the emitter's motion (see
                // Facing::Velocity) - whether or not the particle kept any of it.
                carrier: node_velocity.into(),
            });
        }
        runtime.alive_until = self.time + effect.spawn.lifetime.largest();

        // Into the ring, wrapping around.
        let stride = std::mem::size_of::<GpuParticle>() as u64;
        let space_to_end = (count - runtime.cursor) as usize;
        let (head, tail) = spawned.split_at(spawned.len().min(space_to_end));
        queue.write_buffer(&self.particle_buffer, (first + runtime.cursor) as u64 * stride, bytemuck::cast_slice(head));
        if !tail.is_empty() {
            queue.write_buffer(&self.particle_buffer, first as u64 * stride, bytemuck::cast_slice(tail));
        }
        runtime.cursor = (runtime.cursor + spawn_count) % count;
    }

    // ── Trails ──

    fn age_trail(trail: &mut TrailState, effect: &TrailEffect, dt: f32, wind: Vector3<f32>) {
        let drift = (wind * effect.wind * dt).cast::<f64>();
        for point in trail.points.iter_mut() {
            point.age += dt;
            point.position += drift + (point.velocity * dt).cast::<f64>();
        }
        while trail.points.back().is_some_and(|point| point.age >= effect.lifetime) {
            trail.points.pop_back();
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn update_trail_emitter(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, runtime: &mut EmitterRuntime, source: &EmitterSource, effect: &TrailEffect, dt: f32, wind: Vector3<f32>) {
        runtime.blend = effect.blend;
        runtime.sprite = effect.texture.clone();
        if let Some(path) = &effect.texture {
            self.ensure_sprite(device, queue, path);
        }
        let trail = &mut runtime.trail;
        Self::age_trail(trail, effect, dt, wind);
        trail.effect = Some(effect.clone());

        let emitter = source.emitter;
        if !emitter.enabled {
            trail.head = None;
            return;
        }
        let origin = (source.position + source.rotation * emitter.offset).cast::<f64>();
        let intensity = emitter.intensity.clamp(0.0, 1.0);
        let velocity = source.rotation * emitter.rotation * effect.point_velocity;
        let laying = trail.head.is_some() && !trail.points.is_empty();
        if !laying {
            // Starting (or restarting after a gap) - not joined to what's older.
            trail.points.push_front(TrailPoint { position: origin, velocity, serial: trail.next_serial, age: 0.0, intensity, joined: false });
            trail.next_serial = (trail.next_serial + 1.0) % 4096.0;
            trail.last_laid = Some(origin);
        } else {
            // A point every `segment_spacing` along the way here, aged by
            // how early in the frame it was passed - spaced along where the
            // emitter went (the points themselves may have moved off since).
            let spacing = effect.segment_spacing.max(0.05) as f64;
            let newest = trail.last_laid.unwrap_or(origin);
            let travel = origin - newest;
            let distance = travel.norm();
            if distance >= spacing {
                let direction = travel / distance;
                let steps = ((distance / spacing) as usize).min(MAX_TRAIL_POINTS);
                for step in 1..=steps {
                    let along = spacing * step as f64;
                    let age = dt * (1.0 - (along / distance) as f32);
                    let laid = newest + direction * along;
                    // Already moved for the part of the frame since it was laid.
                    let position = laid + (velocity * age).cast::<f64>();
                    trail.points.push_front(TrailPoint { position, velocity, serial: trail.next_serial, age, intensity, joined: true });
                    trail.next_serial = (trail.next_serial + 1.0) % 4096.0;
                    trail.last_laid = Some(laid);
                }
            }
        }
        trail.points.truncate(MAX_TRAIL_POINTS);
        trail.head = Some((origin, intensity));
        runtime.alive_until = self.time + effect.lifetime;
    }

    /// Every trail as a camera-facing ribbon, into the trail buffers.
    fn build_trails(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: &ParticleFrameInfo) {
        let camera = frame.camera_position.cast::<f64>();
        let view = frame.camera_view;
        let camera_right = Vector3::new(view[(0, 0)], view[(0, 1)], view[(0, 2)]);
        let mut vertices: Vec<TrailVertex> = Vec::new();
        let mut indices: Vec<u32> = Vec::new();
        let mut draws = Vec::new();
        let time = self.time;

        for runtime in self.runtimes.values().filter(|runtime| runtime.is_trail) {
            let (Some(effect), trail) = (&runtime.trail.effect, &runtime.trail) else { continue };
            let first_index = indices.len() as u32;

            // Split into connected runs: (camera-relative position, age,
            // intensity, serial).
            let mut runs: Vec<Vec<(Vector3<f32>, f32, f32, f32)>> = Vec::new();
            let mut run: Vec<(Vector3<f32>, f32, f32, f32)> = Vec::new();
            if let Some((head, intensity)) = trail.head {
                run.push(((head - camera).cast::<f32>(), 0.0, intensity, trail.next_serial));
            }
            for point in &trail.points {
                run.push(((point.position - camera).cast::<f32>(), point.age, point.intensity, point.serial));
                if !point.joined {
                    runs.push(std::mem::take(&mut run));
                }
            }
            runs.push(run);

            for run in runs.iter().filter(|run| run.len() >= 2) {
                let base = vertices.len() as u32;
                for (i, (position, age, intensity, serial)) in run.iter().enumerate() {
                    let t = (age / effect.lifetime.max(1e-3)).clamp(0.0, 1.0);
                    let newer = run[i.saturating_sub(1)].0;
                    let older = run[(i + 1).min(run.len() - 1)].0;
                    let tangent = (newer - older).try_normalize(1e-6).unwrap_or(camera_right);
                    let to_camera = (-position).try_normalize(1e-6).unwrap_or(Vector3::y());
                    let side = tangent.cross(&to_camera).try_normalize(1e-6).unwrap_or(camera_right);
                    let mut width = effect.width.sample(t);
                    if effect.intensity_scales.size {
                        width *= intensity;
                    }
                    let mut color = effect.color.sample(t);
                    if effect.intensity_scales.alpha {
                        color[3] *= intensity;
                    }
                    let rgba = [color[0] * effect.brightness, color[1] * effect.brightness, color[2] * effect.brightness, color[3]];
                    let u = t + time * effect.uv_scroll;
                    match effect.facing {
                        TrailFacing::Camera => {
                            let params = [if effect.lit { 1.0 } else { 0.0 }, effect.soft_fade_distance.max(0.0), 0.0, *serial];
                            for (sign, v) in [(-1.0f32, 0.0f32), (1.0, 1.0)] {
                                let corner = position + side * (width * 0.5 * sign);
                                vertices.push(TrailVertex { position: corner.into(), uv: [u, v], color: rgba, params });
                            }
                        }
                        TrailFacing::Upright => {
                            // Standing on the point, `width` tall.
                            let params = [if effect.lit { 1.0 } else { 0.0 }, effect.soft_fade_distance.max(0.0), 1.0, *serial];
                            for v in [0.0f32, 1.0] {
                                let corner = position + Vector3::y() * (width * v);
                                vertices.push(TrailVertex { position: corner.into(), uv: [u, v], color: rgba, params });
                            }
                        }
                        TrailFacing::Ridge => {
                            // A triangular cross-section across the trail:
                            // two base corners on the ground either side,
                            // `width` apart times RIDGE_BASE, and a peak
                            // `width` up - its height and sideways lean
                            // jittered per point (stuck to the point via its
                            // serial), so the ridge line zig-zags and the
                            // faces break into irregular triangles.
                            let across = Vector3::y().cross(&tangent).try_normalize(1e-6).unwrap_or(camera_right);
                            let half_base = width * RIDGE_BASE * 0.5;
                            let height = width * (0.75 + 0.5 * hash(*serial));
                            let lean = (hash(*serial + 91.0) - 0.5) * half_base;
                            let params = [if effect.lit { 1.0 } else { 0.0 }, effect.soft_fade_distance.max(0.0), 2.0, *serial];
                            for (corner, v) in [
                                (position - across * half_base, 0.0f32),
                                (position + Vector3::y() * height + across * lean, 1.0),
                                (position + across * half_base, 0.0),
                            ] {
                                vertices.push(TrailVertex { position: corner.into(), uv: [u, v], color: rgba, params });
                            }
                        }
                    }
                }
                match effect.facing {
                    TrailFacing::Ridge => {
                        // Per segment: the inner slope (base-in, peak) and
                        // the outer one (peak, base-out), two triangles each.
                        for i in 0..(run.len() as u32 - 1) {
                            let a = base + i * 3;
                            let b = a + 3;
                            indices.extend_from_slice(&[a, a + 1, b, a + 1, b + 1, b]);
                            indices.extend_from_slice(&[a + 1, a + 2, b + 1, a + 2, b + 2, b + 1]);
                        }
                    }
                    _ => {
                        for i in 0..(run.len() as u32 - 1) {
                            let a = base + i * 2;
                            indices.extend_from_slice(&[a, a + 1, a + 2, a + 1, a + 3, a + 2]);
                        }
                    }
                }
            }
            let last_index = indices.len() as u32;
            if last_index > first_index {
                draws.push(TrailDraw { blend: runtime.blend, sprite: runtime.sprite.clone(), indices: first_index..last_index });
            }
        }

        if vertices.len() > self.trail_vertex_capacity || indices.len() > self.trail_index_capacity {
            self.trail_vertex_capacity = self.trail_vertex_capacity.max(vertices.len().next_power_of_two());
            self.trail_index_capacity = self.trail_index_capacity.max(indices.len().next_power_of_two());
            (self.trail_vertex_buffer, self.trail_index_buffer) = Self::make_trail_buffers(device, self.trail_vertex_capacity, self.trail_index_capacity);
        }
        if !vertices.is_empty() {
            queue.write_buffer(&self.trail_vertex_buffer, 0, bytemuck::cast_slice(&vertices));
            queue.write_buffer(&self.trail_index_buffer, 0, bytemuck::cast_slice(&indices));
        }
        self.trail_draws = draws;
    }

    // ── Rendering ──

    /// The update pass - ages and moves every particle (see
    /// particles_update.wgsl). Record before `draw`'s render pass.
    /// Runs once per `update` - a frame rendered without one (nothing new
    /// written) doesn't move them a second time.
    pub fn simulate(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if !std::mem::take(&mut self.simulation_pending) || self.particle_draws.is_empty() {
            return;
        }
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("Particle Update Pass"), timestamp_writes: None });
        pass.set_pipeline(&self.compute_pipeline);
        pass.set_bind_group(0, &self.compute_bind_group, &[]);
        pass.dispatch_workgroups(PARTICLE_CAPACITY.div_ceil(WORKGROUP_SIZE), 1, 1);
    }

    /// Draws every trail and particle into `pass` (which must have the
    /// scene's color and depth attached) - alpha-blended ones first, then
    /// additive ones on top.
    pub fn draw(&self, pass: &mut wgpu::RenderPass, camera_bind_group: &wgpu::BindGroup) {
        pass.set_bind_group(1, camera_bind_group, &[]);
        pass.set_bind_group(2, &self.depth_bind_group, &[]);
        for blend in [Blend::Alpha, Blend::Additive] {
            if self.trail_draws.iter().any(|draw| draw.blend == blend) {
                pass.set_pipeline(&self.trail_pipelines[blend_index(blend)]);
                pass.set_bind_group(0, &self.trail_frame_bind_group, &[]);
                pass.set_vertex_buffer(0, self.trail_vertex_buffer.slice(..));
                pass.set_index_buffer(self.trail_index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                for draw in self.trail_draws.iter().filter(|draw| draw.blend == blend) {
                    pass.set_bind_group(3, self.sprite(&draw.sprite), &[]);
                    pass.draw_indexed(draw.indices.clone(), 0, 0..1);
                }
            }
            if self.particle_draws.iter().any(|draw| draw.blend == blend) {
                pass.set_pipeline(&self.particle_pipelines[blend_index(blend)]);
                pass.set_bind_group(0, &self.particle_bind_group, &[]);
                for draw in self.particle_draws.iter().filter(|draw| draw.blend == blend) {
                    pass.set_bind_group(3, self.sprite(&draw.sprite), &[]);
                    pass.draw(0..6, draw.first..draw.first + draw.count);
                }
            }
        }
    }
}

/// A ridge trail's base, as a fraction of its `width` (see TrailFacing::Ridge).
const RIDGE_BASE: f32 = 0.7;

/// Random 0..1 from a number - the same number always gives the same value.
fn hash(x: f32) -> f32 {
    ((x * 127.1).sin() * 43758.547).fract().abs()
}

// ── Emission helpers ───────────────────────────────────────────────────────

/// A random point in the emission shape, in the emitter's frame.
fn sample_shape(shape: &EmissionShape, rng: &mut impl Rng) -> Vector3<f32> {
    match *shape {
        EmissionShape::Point => Vector3::zeros(),
        EmissionShape::Sphere { radius } => loop {
            let point = Vector3::new(rng.gen_range(-1.0..1.0), rng.gen_range(-1.0..1.0), rng.gen_range(-1.0f32..1.0));
            if point.norm_squared() <= 1.0 {
                break point * radius;
            }
        },
        EmissionShape::Disk { radius } => {
            let distance = radius * rng.gen::<f32>().sqrt();
            let angle = rng.gen_range(0.0..std::f32::consts::TAU);
            Vector3::new(angle.cos() * distance, angle.sin() * distance, 0.0)
        }
        EmissionShape::Box { half_extents } => Vector3::new(
            rng.gen_range(-1.0..=1.0f32) * half_extents.x,
            rng.gen_range(-1.0..=1.0f32) * half_extents.y,
            rng.gen_range(-1.0..=1.0f32) * half_extents.z,
        ),
        EmissionShape::Line { length } => Vector3::new((rng.gen::<f32>() - 0.5) * length, 0.0, 0.0),
    }
}

/// A random direction within `degrees` of `axis` (uniform over the cap).
fn cone_direction(axis: Vector3<f32>, degrees: f32, rng: &mut impl Rng) -> Vector3<f32> {
    let cos_max = degrees.clamp(0.0, 180.0).to_radians().cos();
    let cos_theta = 1.0 - rng.gen::<f32>() * (1.0 - cos_max);
    let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
    let phi = rng.gen_range(0.0..std::f32::consts::TAU);
    let helper = if axis.x.abs() < 0.9 { Vector3::x() } else { Vector3::y() };
    let tangent = axis.cross(&helper).normalize();
    let bitangent = axis.cross(&tangent);
    axis * cos_theta + (tangent * phi.cos() + bitangent * phi.sin()) * sin_theta
}
