//! The sea - its waves (see `waves`) and the wake a jet leaves on it flying
//! fast and low - drawn with the game's ocean shader (assets/shaders/
//! ocean.wgsl) on the node that has `Ocean::material`.
//!
//! A scene with a sea keeps an `Ocean` and calls `update` from its
//! `after_cameras` every frame: that's when the camera is where it's drawn
//! from, and the waves are worked out around exactly that point (in f64 -
//! see `waves`), so they don't swim against the mesh.

mod waves;

use std::collections::VecDeque;

use nalgebra::Vector3;

use crate::engine::scene_manager::properties::Material;
use crate::engine::scene_manager::scene::Scene;

pub use waves::{OceanSettings, OceanWaves, MAX_WAKE_POINTS, MAX_WAVES, WAKE_NOISE_PERIOD};

/// The sea's shader (assets/shaders/ocean.wgsl).
pub const OCEAN_SHADER: &str = "ocean";

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

/// Something flying low over the water, stirring it up - see
/// `Ocean::set_wake_source`. True world position and velocity.
#[derive(Clone, Copy, Debug)]
pub struct WakeSource {
    pub position: Vector3<f32>,
    pub velocity: Vector3<f32>,
}

/// One point of the wake's trail - where the source was, how long ago, and
/// how strongly it was stirring the water then.
#[derive(Clone, Copy, Debug)]
struct WakePoint {
    x: f64,
    z: f64,
    age: f32,
    intensity: f32,
}

pub struct Ocean {
    waves: OceanWaves,
    // The low-flying plane's wake - see `set_wake_source`/`update_wake`.
    wake_source: Option<WakeSource>,
    wake_trail: VecDeque<WakePoint>,
    wake_sample_timer: f32,
    // Extra XZ added to the camera when phasing the waves, then cleared.
    // The title uses it so the sea streams past a camera that stays put.
    flow: (f32, f32),
}

impl Default for Ocean {
    fn default() -> Self {
        Self::new(OceanSettings::default())
    }
}

impl Ocean {
    pub fn new(settings: OceanSettings) -> Self {
        Self { waves: OceanWaves::new(settings), wake_source: None, wake_trail: VecDeque::new(), wake_sample_timer: 0.0, flow: (0.0, 0.0) }
    }

    /// What the sea's node is drawn with - the ocean shader, its values
    /// filled in every frame by `update`.
    pub fn material() -> Material {
        Material::shader(OCEAN_SHADER)
    }

    /// The sea's rest level (world Y) - see OceanWaves::sea_level.
    pub fn sea_level(&self) -> f32 {
        self.waves.sea_level()
    }

    pub fn settings(&self) -> &OceanSettings {
        self.waves.settings()
    }

    /// The wind over it - toward where it blows, m/s.
    pub fn wind(&self) -> Vector3<f32> {
        let settings = self.settings();
        Vector3::new(settings.wind_direction.cos(), 0.0, settings.wind_direction.sin()) * settings.wind_speed
    }

    /// Slides the wave field by `x`/`z` (world units) for this frame, as if
    /// the camera had moved there. Cleared by `update`, so a scene sets it
    /// every frame it wants the sea to stream; every other scene stays put.
    pub fn set_flow(&mut self, x: f32, z: f32) {
        self.flow = (x, z);
    }

    /// Slides an already-laid foam trail by `x`/`z` (world units). The title
    /// jet doesn't move, so its wake is carried along with the sea instead.
    pub fn scroll_wake(&mut self, x: f32, z: f32) {
        for point in &mut self.wake_trail {
            point.x += x as f64;
            point.z += z as f64;
        }
    }

    /// What's stirring up the water this frame - the game calls this every
    /// frame (`None` when nothing is), before `update`. How strongly it shows
    /// depends on its speed and how close it is to the water (see WAKE_*);
    /// far enough up or slow enough, it leaves nothing.
    pub fn set_wake_source(&mut self, source: Option<WakeSource>) {
        self.wake_source = source;
    }

    /// How hard `source` stirs the water, 0..1 - stronger the lower and
    /// faster it goes (see WAKE_*). The same strength the wake is drawn
    /// with, for anything that should match it (e.g. spray particles).
    pub fn wake_strength(&self, source: &WakeSource) -> f32 {
        let height = source.position.y - self.sea_level();
        let speed = Vector3::new(source.velocity.x, 0.0, source.velocity.z).magnitude();
        let smoothstep = |edge0: f32, edge1: f32, x: f32| {
            let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        (1.0 - smoothstep(WAKE_FULL_HEIGHT, WAKE_MAX_HEIGHT, height)) * smoothstep(WAKE_MIN_SPEED, WAKE_FULL_SPEED, speed)
    }

    /// Advances the waves by `delta_time` and puts this frame's waves and
    /// wake into the material of the node `node` (the sea - see
    /// `material`), worked out around where the camera is now. From the
    /// scene's `after_cameras`, every frame (see the module doc).
    pub fn update(&mut self, scene: &mut Scene, node: &str, delta_time: f32) {
        let camera_position = scene.cameras.active().camera.position().coords;
        self.waves.advance(delta_time);
        let intensity = self.update_wake(delta_time);
        let (flow_x, flow_z) = self.flow;
        self.flow = (0.0, 0.0);
        let mut params = self.waves.params(camera_position.x + flow_x, camera_position.z + flow_z);
        self.write_wake(&mut params, camera_position, intensity);
        let Some(material) = scene.content.nodes.get_mut(node).and_then(|node| node.get_property_mut::<Material>()) else { return };
        for (name, values) in params {
            material.set(&name, values);
        }
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
            // A source that jumped (the title, then a level) must not drag
            // one foam line between the two places.
            if self.wake_trail.front().is_some_and(|point| {
                let dx = point.x - source.position.x as f64;
                let dz = point.z - source.position.z as f64;
                dx * dx + dz * dz > 500.0 * 500.0
            }) {
                self.wake_trail.clear();
            }
            if intensity > 0.0 || !self.wake_trail.is_empty() {
                self.wake_trail.push_front(WakePoint { x: source.position.x as f64, z: source.position.z as f64, age: 0.0, intensity });
                self.wake_trail.truncate(MAX_WAKE_POINTS);
            }
        }
        intensity
    }

    // The wake's params (see ocean.wgsl's `Ocean`), camera-relative (in f64,
    // like the waves' phases - stays precise far from the origin) - all zero
    // when there's none.
    fn write_wake(&self, params: &mut std::collections::BTreeMap<String, Vec<f32>>, camera_position: Vector3<f32>, intensity: f32) {
        let mut head = [0.0f32; 4];
        let mut bounds = [0.0f32; 4];
        let mut info = [0.0f32; 4];
        let mut points = vec![0.0f32; MAX_WAKE_POINTS * 4];
        let mut anchor = [0.0f32; 4];

        let relative = |x: f64, z: f64| [(x - camera_position.x as f64) as f32, (z - camera_position.z as f64) as f32];
        let active = intensity > 0.0 || self.wake_trail.iter().any(|point| point.intensity > 0.0);
        if let Some(source) = self.wake_source.filter(|_| active) {
            let at = relative(source.position.x as f64, source.position.z as f64);
            let height = source.position.y - self.sea_level();
            head = [at[0], at[1], height, intensity];

            let (mut min, mut max) = (at, at);
            for (slot, point) in self.wake_trail.iter().enumerate() {
                let [x, z] = relative(point.x, point.z);
                points[slot * 4..slot * 4 + 4].copy_from_slice(&[x, z, point.age, point.intensity]);
                min = [min[0].min(x), min[1].min(z)];
                max = [max[0].max(x), max[1].max(z)];
            }
            let center = [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5];
            let radius = 0.5 * ((max[0] - min[0]).powi(2) + (max[1] - min[1]).powi(2)).sqrt() + WAKE_REACH;
            bounds = [center[0], center[1], radius, WAKE_TRAIL_SECONDS];
            // Which way it's flying, flattened onto the water - shapes the
            // blast patch and the V of ripples behind it. Straight ahead (+Z)
            // if it's barely moving (there's no wake then anyway).
            let heading = Vector3::new(source.velocity.x, 0.0, source.velocity.z).try_normalize(1e-3).unwrap_or(Vector3::z());
            info = [self.wake_trail.len() as f32, heading.x, heading.z, 0.0];
            // Where the camera is on the (repeating) foam noise.
            anchor = [(camera_position.x as f64).rem_euclid(WAKE_NOISE_PERIOD) as f32, (camera_position.z as f64).rem_euclid(WAKE_NOISE_PERIOD) as f32, 0.0, 0.0];
        }
        params.insert("wake_head".to_owned(), head.to_vec());
        params.insert("wake_bounds".to_owned(), bounds.to_vec());
        params.insert("wake_info".to_owned(), info.to_vec());
        params.insert("wake_points".to_owned(), points);
        params.insert("wake_anchor".to_owned(), anchor.to_vec());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_writes_every_param_the_ocean_shader_reads() {
        // engine::rendering::shader_library's tests check ocean.wgsl reads
        // exactly these.
        let ocean = Ocean::default();
        let mut params = ocean.waves.params(0.0, 0.0);
        ocean.write_wake(&mut params, Vector3::zeros(), 0.0);
        let mut names: Vec<&str> = params.keys().map(String::as_str).collect();
        names.sort_unstable();
        let mut expected = ["params", "shading", "waves", "wake_head", "wake_bounds", "wake_info", "wake_points", "wake_anchor"];
        expected.sort_unstable();
        assert_eq!(names, expected);
        assert_eq!(params["waves"].len(), MAX_WAVES * 2 * 4);
        assert_eq!(params["wake_points"].len(), MAX_WAKE_POINTS * 4);
    }
}
