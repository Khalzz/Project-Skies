use std::collections::{HashMap, VecDeque};
use std::ops::Range;

use nalgebra::{Matrix4, Vector3, Vector4};
use wgpu::util::DeviceExt;
use wgpu::{BindGroup, BindGroupLayout, Buffer, Device, Queue, RenderPipeline, Sampler, SurfaceConfiguration, TextureView};

use crate::engine::rendering::camera::handler::CameraResources;
use crate::engine::rendering::enviroment::light::Light;
use crate::engine::rendering::enviroment::ocean::{OceanSettings, OceanUniform, OceanWaves, MAX_WAKE_POINTS};
use crate::engine::rendering::instance_management::InstanceRaw;
use crate::engine::rendering::models::model::{self, Vertex};
use crate::engine::rendering::models::textures::Texture;
use crate::engine::rendering::ui::rendering_utils;
use crate::resources::WaterTile;

/// Something flying low over the water, stirring it up - see
/// `WaterRenderData::set_wake_source`. True world position and velocity.
#[derive(Clone, Copy, Debug)]
pub struct WakeSource {
    pub position: Vector3<f32>,
    pub velocity: Vector3<f32>,
}

// The wake: how it's strengthened by speed and closeness to the water, and
// how long its trail lasts. A source fades in below WAKE_MAX_HEIGHT above
// the water (full at WAKE_FULL_HEIGHT) and above WAKE_MIN_SPEED horizontal
// speed (full at WAKE_FULL_SPEED, m/s - ~80 and ~290 knots). The trail
// keeps a point every WAKE_SAMPLE_SECONDS, MAX_WAKE_POINTS of them.
const WAKE_MAX_HEIGHT: f32 = 35.0;
const WAKE_FULL_HEIGHT: f32 = 5.0;
const WAKE_MIN_SPEED: f32 = 40.0;
const WAKE_FULL_SPEED: f32 = 150.0;
const WAKE_SAMPLE_SECONDS: f32 = 0.1;
const WAKE_TRAIL_SECONDS: f32 = WAKE_SAMPLE_SECONDS * MAX_WAKE_POINTS as f32;
// How far past the trail's own points the wake can draw (its widest foam
// plus the ripples around the plane) - for the shader's early-out circle.
const WAKE_REACH: f32 = 120.0;

/// One point of the wake's trail - where the source was, how long ago, and
/// how strongly it was stirring the water then.
#[derive(Clone, Copy, Debug)]
struct WakePoint {
    x: f64,
    z: f64,
    age: f32,
    intensity: f32,
}

/// What the ocean's last water pass drew vs. could have - see
/// `WaterRenderData::draw_stats` (shown on the F3 debug panel).
#[derive(Clone, Copy, Debug, Default)]
pub struct OceanDrawStats {
    pub tiles_drawn: u32,
    pub tiles_total: u32,
    pub triangles_drawn: u64,
    pub triangles_total: u64,
}

/// The four side planes of a view volume, (a, b, c, d) with a point inside
/// when a*x + b*y + c*z + d >= 0 - in true world space. No near/far planes:
/// the four sides already meet at the camera, so they only enclose what's
/// in front of it, and the ocean is meant to reach past any far plane.
type Frustum = [Vector4<f32>; 4];

/// A second draw pipeline, alongside `App::render_pipeline`, used only for
/// model_refs in `App::water_shaded_models` (see `render_pass.rs`'s own
/// `render_water_pass`, which is what actually draws with this instead of
/// the generic `draw_model_instanced_from_list` every other model goes
/// through). Group 3 (light) still rides the same shared bind group every
/// other pipeline uses (see `LightUniform::time`'s own doc comment), and
/// groups 1/2 (camera/mesh transform) reuse the same layouts too - but group
/// 0 is its own thing here: not the model's own material (water doesn't
/// sample a texture at all, see water.wgsl's own doc comment), instead a
/// snapshot of the depth buffer (`DepthRender::foam_depth_copy`) the shader
/// samples to find solid geometry near/behind it, for shore-intersection
/// foam - plus the ocean's wave field (`ocean`, uploaded fresh every frame
/// by `update`). That's also why water can't go through the shared
/// `draw_model_instanced_from_list`/`draw_mesh_instanced` helpers other
/// models use - those unconditionally bind the mesh's own *material* at
/// group 0, which would stomp this.
pub struct WaterRenderData {
    depth_bind_group_layout: BindGroupLayout,
    depth_bind_group: BindGroup,
    pub render_pipeline: RenderPipeline,
    /// The waves the shader draws - and the sea's rest level gameplay treats
    /// as the flat water surface (`ocean.sea_level()`).
    pub ocean: OceanWaves,
    ocean_buffer: Buffer,
    // Altitude tiers of the ocean mesh (see resources::WaterTier) - each
    // tier's minimum camera altitude, ascending - and the one being drawn.
    tier_altitudes: Vec<f32>,
    tier: usize,
    // Culling tiles per ocean mesh name (see resources::WaterTile).
    tiles: HashMap<String, Vec<WaterTile>>,
    // F8 debug (see `toggle_culling_freeze`): while set, culling keeps using
    // the view it captured, so you can look around and see what's skipped.
    freeze_culling: bool,
    frozen_frustum: Option<Frustum>,
    stats: OceanDrawStats,
    // The low-flying plane's wake - see `set_wake_source`/`update_wake`.
    wake_source: Option<WakeSource>,
    wake_trail: VecDeque<WakePoint>,
    wake_sample_timer: f32,
}

impl WaterRenderData {
    pub fn new(device: &Device, config: &SurfaceConfiguration, camera: &CameraResources, light: &Light, depth_copy_view: &TextureView, depth_copy_sampler: &Sampler) -> Self {
        let ocean = OceanWaves::new(OceanSettings::default());
        let ocean_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Ocean Buffer"),
            contents: bytemuck::cast_slice(&[ocean.uniform(0.0, 0.0)]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let depth_bind_group_layout = Self::create_depth_bind_group_layout(device);
        let depth_bind_group = Self::create_depth_bind_group(device, &depth_bind_group_layout, depth_copy_view, depth_copy_sampler, &ocean_buffer);

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Water Pipeline Layout"),
            bind_group_layouts: &[
                &depth_bind_group_layout,
                &camera.bind_group_layout,
                &model::Mesh::create_bind_group_layout(device),
                &light.rendering_data.bind_group_layout,
            ],
            push_constant_ranges: &[],
        });

        let shader = wgpu::ShaderModuleDescriptor {
            label: Some("Water Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/water.wgsl").into()),
        };

        let render_pipeline = rendering_utils::create_render_pipeline(
            device,
            &layout,
            config.format,
            Some(Texture::DEPTH_FORMAT),
            &[model::ModelVertex::desc(), InstanceRaw::desc()],
            shader,
            Some(wgpu::Face::Back),
            true,
        );

        Self { depth_bind_group_layout, depth_bind_group, render_pipeline, ocean, ocean_buffer, tier_altitudes: Vec::new(), tier: 0, tiles: HashMap::new(), freeze_culling: false, frozen_frustum: None, stats: OceanDrawStats::default(), wake_source: None, wake_trail: VecDeque::new(), wake_sample_timer: 0.0 }
    }

    /// Advances the waves and uploads them for this frame, phased around
    /// `camera_position` (true world position - the same value water.wgsl's
    /// vertices are camera-relative to). Call once per frame, before drawing.
    pub fn update(&mut self, queue: &Queue, delta_time: f32, camera_position: Vector3<f32>) {
        self.update_tier(camera_position.y);
        self.ocean.advance(delta_time);
        let intensity = self.update_wake(delta_time);
        let mut uniform = self.ocean.uniform(camera_position.x, camera_position.z);
        self.write_wake(&mut uniform, camera_position, intensity);
        queue.write_buffer(&self.ocean_buffer, 0, bytemuck::cast_slice(&[uniform]));
    }

    /// Each ocean mesh tier's minimum camera altitude, ascending (see
    /// resources::WaterTier) - set once, when the ocean model is registered.
    pub fn set_tier_altitudes(&mut self, altitudes: Vec<f32>) {
        self.tier_altitudes = altitudes;
        self.tier = 0;
    }

    /// Which ocean mesh tier to draw this frame (see `render_water_pass`).
    pub fn tier(&self) -> usize {
        self.tier
    }

    // Steps up a tier once the camera reaches the next tier's altitude, and
    // back down only once it's 10% below the current tier's - so hovering
    // right at a boundary doesn't flip between the two every frame.
    fn update_tier(&mut self, altitude: f32) {
        const STEP_DOWN_MARGIN: f32 = 0.9;
        while self.tier + 1 < self.tier_altitudes.len() && altitude >= self.tier_altitudes[self.tier + 1] {
            self.tier += 1;
        }
        while self.tier > 0 && altitude < self.tier_altitudes[self.tier] * STEP_DOWN_MARGIN {
            self.tier -= 1;
        }
    }

    /// An ocean mesh's culling tiles (see resources::WaterTile) - set once,
    /// when it's registered. Meshes without tiles always draw whole.
    pub fn set_mesh_tiles(&mut self, mesh_name: String, tiles: Vec<WaterTile>) {
        self.tiles.insert(mesh_name, tiles);
    }

    /// F8 debug: freezes culling at the current view (or unfreezes it) - fly
    /// or turn around while frozen and the ocean only exists inside the view
    /// it was frozen at, so you can see exactly which tiles are skipped.
    pub fn toggle_culling_freeze(&mut self) {
        self.freeze_culling = !self.freeze_culling;
        self.frozen_frustum = None;
        println!("Ocean culling: {}", if self.freeze_culling { "FROZEN at the current view - look around to see what's skipped" } else { "live" });
    }

    /// Last water pass's culling results - see OceanDrawStats.
    pub fn draw_stats(&self) -> OceanDrawStats {
        self.stats
    }

    /// Resets the culling counters - call once at the start of each water pass.
    pub fn begin_culling_stats(&mut self) {
        self.stats = OceanDrawStats::default();
    }

    /// The view volume's side planes in true world space, from the camera's
    /// camera-relative `view_proj` (see CameraUniform) and its true position.
    pub fn world_frustum(view_proj: &Matrix4<f32>, camera_position: Vector3<f32>) -> Frustum {
        // Clip space keeps -w <= x, y <= w - each inequality is a plane
        // (Gribb/Hartmann): row 3 plus/minus row 0 (left/right) and row 1
        // (bottom/top). Then shifted from camera-relative to world space.
        let row = |i: usize| view_proj.row(i).transpose();
        let planes = [row(3) + row(0), row(3) - row(0), row(3) + row(1), row(3) - row(1)];
        planes.map(|plane| {
            let normal = plane.xyz();
            Vector4::new(plane.x, plane.y, plane.z, plane.w - normal.dot(&camera_position))
        })
    }

    /// The index ranges of `mesh_name` to draw this frame: its tiles that
    /// intersect `frustum` (or the frozen one, see `toggle_culling_freeze`),
    /// neighbours merged into one range. `mesh_position` is where the mesh's
    /// origin is in true world space. `None` if the mesh has no tiles (draw
    /// it whole). Also adds to this pass's `draw_stats`.
    pub fn visible_ranges(&mut self, mesh_name: &str, frustum: Frustum, mesh_position: Vector3<f32>) -> Option<Vec<Range<u32>>> {
        let frustum = if self.freeze_culling { *self.frozen_frustum.get_or_insert(frustum) } else { frustum };
        let tiles = self.tiles.get(mesh_name)?;

        let mut ranges: Vec<Range<u32>> = Vec::new();
        for tile in tiles {
            self.stats.tiles_total += 1;
            self.stats.triangles_total += (tile.index_count / 3) as u64;

            let min = mesh_position + Vector3::from(tile.min);
            let max = mesh_position + Vector3::from(tile.max);
            // A box is out of view once it's entirely behind any one plane -
            // test the corner furthest along that plane's normal.
            let outside = frustum.iter().any(|plane| {
                let corner = Vector3::new(
                    if plane.x >= 0.0 { max.x } else { min.x },
                    if plane.y >= 0.0 { max.y } else { min.y },
                    if plane.z >= 0.0 { max.z } else { min.z },
                );
                plane.xyz().dot(&corner) + plane.w < 0.0
            });
            if outside {
                continue;
            }

            self.stats.tiles_drawn += 1;
            self.stats.triangles_drawn += (tile.index_count / 3) as u64;
            let range = tile.first_index..tile.first_index + tile.index_count;
            match ranges.last_mut() {
                Some(last) if last.end == range.start => last.end = range.end,
                _ => ranges.push(range),
            }
        }
        Some(ranges)
    }

    /// What's stirring up the water this frame - the game calls this every
    /// frame (`None` when nothing is), before the water updates. How strongly
    /// it shows depends on its speed and how close it is to the water (see
    /// WAKE_*); far enough up or slow enough, it leaves nothing.
    pub fn set_wake_source(&mut self, source: Option<WakeSource>) {
        self.wake_source = source;
    }

    /// How hard `source` stirs the water, 0..1 - stronger the lower and
    /// faster it goes (see WAKE_*). The same strength the wake is drawn
    /// with, for anything that should match it (e.g. spray particles).
    pub fn wake_strength(&self, source: &WakeSource) -> f32 {
        let height = source.position.y - self.ocean.sea_level();
        let speed = Vector3::new(source.velocity.x, 0.0, source.velocity.z).magnitude();
        let smoothstep = |edge0: f32, edge1: f32, x: f32| {
            let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        (1.0 - smoothstep(WAKE_FULL_HEIGHT, WAKE_MAX_HEIGHT, height)) * smoothstep(WAKE_MIN_SPEED, WAKE_FULL_SPEED, speed)
    }

    // Ages the trail, drops points older than WAKE_TRAIL_SECONDS, and adds a
    // new one every WAKE_SAMPLE_SECONDS. Returns the source's intensity now.
    fn update_wake(&mut self, delta_time: f32) -> f32 {
        for point in self.wake_trail.iter_mut() {
            point.age += delta_time;
        }
        while self.wake_trail.back().is_some_and(|point| point.age > WAKE_TRAIL_SECONDS) {
            self.wake_trail.pop_back();
        }

        let Some(source) = self.wake_source else { return 0.0 };
        let intensity = self.wake_strength(&source);

        self.wake_sample_timer += delta_time;
        if self.wake_sample_timer >= WAKE_SAMPLE_SECONDS {
            self.wake_sample_timer = 0.0;
            if intensity > 0.0 || !self.wake_trail.is_empty() {
                self.wake_trail.push_front(WakePoint { x: source.position.x as f64, z: source.position.z as f64, age: 0.0, intensity });
                self.wake_trail.truncate(MAX_WAKE_POINTS);
            }
        }
        intensity
    }

    // Fills in the wake part of this frame's uniform, camera-relative (in
    // f64, like the waves' phases - stays precise far from the origin).
    fn write_wake(&self, uniform: &mut OceanUniform, camera_position: Vector3<f32>, intensity: f32) {
        let relative = |x: f64, z: f64| [(x - camera_position.x as f64) as f32, (z - camera_position.z as f64) as f32];
        let active = intensity > 0.0 || self.wake_trail.iter().any(|point| point.intensity > 0.0);
        let Some(source) = self.wake_source.filter(|_| active) else { return };

        let head = relative(source.position.x as f64, source.position.z as f64);
        let height = source.position.y - self.ocean.sea_level();
        uniform.wake_head = [head[0], head[1], height, intensity];

        let (mut min, mut max) = (head, head);
        for (slot, point) in self.wake_trail.iter().enumerate() {
            let [x, z] = relative(point.x, point.z);
            uniform.wake_points[slot] = [x, z, point.age, point.intensity];
            min = [min[0].min(x), min[1].min(z)];
            max = [max[0].max(x), max[1].max(z)];
        }
        let center = [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5];
        let radius = 0.5 * ((max[0] - min[0]).powi(2) + (max[1] - min[1]).powi(2)).sqrt() + WAKE_REACH;
        uniform.wake_bounds = [center[0], center[1], radius, WAKE_TRAIL_SECONDS];
        // Which way it's flying, flattened onto the water - shapes the blast
        // patch and the V of ripples behind it. Straight ahead (+Z) if it's
        // barely moving (there's no wake then anyway).
        let heading = Vector3::new(source.velocity.x, 0.0, source.velocity.z).try_normalize(1e-3).unwrap_or(Vector3::z());
        uniform.wake_info = [self.wake_trail.len() as f32, heading.x, heading.z, 0.0];
        // Where the camera is on the (repeating) foam noise.
        let period = crate::engine::rendering::enviroment::ocean::WAKE_NOISE_PERIOD;
        uniform.wake_anchor = [(camera_position.x as f64).rem_euclid(period) as f32, (camera_position.z as f64).rem_euclid(period) as f32, 0.0, 0.0];
    }

    /// Regenerates the ocean from new settings (wind, choppiness, ...) - the
    /// next `update` uploads it.
    pub fn set_ocean_settings(&mut self, settings: OceanSettings) {
        self.ocean = OceanWaves::new(settings);
    }

    // Non-filtering/non-comparison, Float{filterable: false} sample type -
    // the same shape DepthRender's own bind_group_layout already uses to
    // sample this exact Depth32Float format as a plain texture (see
    // depth_map.wgsl, the debug depth-visualization shader) - proven to work
    // in this engine already, so mirrored here rather than reaching for
    // WGSL's dedicated texture_depth_2d/Depth sample type instead. No
    // uniform buffer entry for near/far (DepthRender's own layout has one) -
    // water.wgsl gets near/far from its own NEAR/FAR constants instead, see
    // that file's own comment on why. Binding 2 is the ocean's wave field
    // (see `OceanUniform`), read by both stages.
    fn create_depth_bind_group_layout(device: &Device) -> BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("water_depth_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        })
    }

    fn create_depth_bind_group(device: &Device, layout: &BindGroupLayout, depth_copy_view: &TextureView, depth_copy_sampler: &Sampler, ocean_buffer: &Buffer) -> BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("water_depth_bind_group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(depth_copy_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(depth_copy_sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: ocean_buffer.as_entire_binding() },
            ],
        })
    }

    pub fn bind_group(&self) -> &BindGroup {
        &self.depth_bind_group
    }

    /// Rebuilds the depth bind group against a freshly-resized
    /// `foam_depth_copy` - call from `App::resize`, after
    /// `DepthRender::resize` has already run (a bind group is tied to a
    /// specific texture view; the old one becomes invalid the instant the
    /// view it pointed at is dropped/replaced).
    pub fn rebuild_depth_bind_group(&mut self, device: &Device, depth_copy_view: &TextureView, depth_copy_sampler: &Sampler) {
        self.depth_bind_group = Self::create_depth_bind_group(device, &self.depth_bind_group_layout, depth_copy_view, depth_copy_sampler, &self.ocean_buffer);
    }
}
