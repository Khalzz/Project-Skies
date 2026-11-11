//! The foam a jet leaves on the water flying fast and low: where it is now
//! (its source, set every frame) and the trail behind it, sent to the shader
//! camera-relative (ocean.wgsl's `wake_*` params and water_wake).

use std::collections::VecDeque;

use nalgebra::Vector3;

use super::shader::Params;

/// How many trail points the shader takes - must match `MAX_WAKE_POINTS` in
/// ocean.wgsl.
pub(super) const MAX_WAKE_POINTS: usize = 32;
/// The wake foam's noise repeats every this many meters (too far apart to
/// notice) - the shader's WAKE_NOISE_PERIOD, keep in sync.
const WAKE_NOISE_PERIOD: f64 = 4096.0;

// How the wake is strengthened by speed and closeness to the water, and how
// long its trail lasts. A source fades in below WAKE_MAX_HEIGHT above the
// water (full at WAKE_FULL_HEIGHT) and above WAKE_MIN_SPEED horizontal speed
// (full at WAKE_FULL_SPEED, m/s - ~80 and ~290 knots). The trail keeps a
// point every WAKE_SAMPLE_SECONDS, MAX_WAKE_POINTS of them.
const WAKE_MAX_HEIGHT: f32 = 35.0;
const WAKE_FULL_HEIGHT: f32 = 5.0;
const WAKE_MIN_SPEED: f32 = 40.0;
const WAKE_FULL_SPEED: f32 = 150.0;
const WAKE_SAMPLE_SECONDS: f32 = 0.1;
const WAKE_TRAIL_SECONDS: f32 = WAKE_SAMPLE_SECONDS * MAX_WAKE_POINTS as f32;
// How far past the trail's own points the foam can draw (its widest, at the
// trail's end: ocean.wgsl's WAKE_TRAIL_WIDTH + WAKE_TRAIL_SPREAD * 3.2 s, ~25
// m, with room) - for the shader's early-out circle.
const WAKE_REACH: f32 = 30.0;

/// Something flying low over the water, stirring it up - see
/// `Ocean::set_wake_source`. True world position and velocity.
#[derive(Clone, Copy, Debug)]
pub struct WakeSource {
    pub position: Vector3<f32>,
    pub velocity: Vector3<f32>,
}

/// One point of the trail - where the source was, how long ago, and how
/// strongly it was stirring the water then.
#[derive(Clone, Copy, Debug)]
struct WakePoint {
    x: f64,
    z: f64,
    age: f32,
    intensity: f32,
}

#[derive(Default)]
pub(super) struct Wake {
    pub(super) source: Option<WakeSource>,
    trail: VecDeque<WakePoint>,
    sample_timer: f32,
}

/// How hard `source` stirs water whose rest level is `sea_level`, 0..1 -
/// stronger the lower and faster it goes (see WAKE_*).
pub(super) fn strength(source: &WakeSource, sea_level: f32) -> f32 {
    let height = source.position.y - sea_level;
    let speed = Vector3::new(source.velocity.x, 0.0, source.velocity.z).magnitude();
    let smoothstep = |edge0: f32, edge1: f32, x: f32| {
        let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    (1.0 - smoothstep(WAKE_FULL_HEIGHT, WAKE_MAX_HEIGHT, height)) * smoothstep(WAKE_MIN_SPEED, WAKE_FULL_SPEED, speed)
}

impl Wake {
    /// Slides the trail already laid by `x`/`z` (world units).
    pub(super) fn scroll(&mut self, x: f32, z: f32) {
        for point in &mut self.trail {
            point.x += x as f64;
            point.z += z as f64;
        }
    }

    /// Ages the trail, drops points older than WAKE_TRAIL_SECONDS, and adds a
    /// new one every WAKE_SAMPLE_SECONDS. Returns the source's intensity now.
    pub(super) fn update(&mut self, delta_time: f32, sea_level: f32) -> f32 {
        for point in self.trail.iter_mut() {
            point.age += delta_time;
        }
        while self.trail.back().is_some_and(|point| point.age > WAKE_TRAIL_SECONDS) {
            self.trail.pop_back();
        }

        let Some(source) = self.source else { return 0.0 };
        let intensity = strength(&source, sea_level);

        self.sample_timer += delta_time;
        if self.sample_timer >= WAKE_SAMPLE_SECONDS {
            self.sample_timer = 0.0;
            // A source that jumped (the title, then a level) must not drag
            // one foam line between the two places.
            if self.trail.front().is_some_and(|point| {
                let dx = point.x - source.position.x as f64;
                let dz = point.z - source.position.z as f64;
                dx * dx + dz * dz > 500.0 * 500.0
            }) {
                self.trail.clear();
            }
            if intensity > 0.0 || !self.trail.is_empty() {
                self.trail.push_front(WakePoint { x: source.position.x as f64, z: source.position.z as f64, age: 0.0, intensity });
                self.trail.truncate(MAX_WAKE_POINTS);
            }
        }
        intensity
    }

    /// The wake's params (ocean.wgsl's `wake_*`), camera-relative (in f64,
    /// like the waves' phases - stays precise far from the origin) - all zero
    /// but the noise anchor when there's none.
    pub(super) fn write(&self, params: &mut Params, camera: Vector3<f32>, intensity: f32, sea_level: f32) {
        let mut head = [0.0f32; 4];
        let mut bounds = [0.0f32; 4];
        let mut info = [0.0f32; 4];
        let mut points = vec![0.0f32; MAX_WAKE_POINTS * 4];

        let relative = |x: f64, z: f64| [(x - camera.x as f64) as f32, (z - camera.z as f64) as f32];
        let active = intensity > 0.0 || self.trail.iter().any(|point| point.intensity > 0.0);
        if let Some(source) = self.source.filter(|_| active) {
            let at = relative(source.position.x as f64, source.position.z as f64);
            head = [at[0], at[1], source.position.y - sea_level, intensity];

            let (mut min, mut max) = (at, at);
            for (slot, point) in self.trail.iter().enumerate() {
                let [x, z] = relative(point.x, point.z);
                points[slot * 4..slot * 4 + 4].copy_from_slice(&[x, z, point.age, point.intensity]);
                min = [min[0].min(x), min[1].min(z)];
                max = [max[0].max(x), max[1].max(z)];
            }
            let center = [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5];
            let radius = 0.5 * ((max[0] - min[0]).powi(2) + (max[1] - min[1]).powi(2)).sqrt() + WAKE_REACH;
            bounds = [center[0], center[1], radius, WAKE_TRAIL_SECONDS];
            info = [self.trail.len() as f32, 0.0, 0.0, 0.0];
        }
        // Where the camera is on the (repeating) foam noise - always: the
        // ripples' gusts and the depth tint are on it too.
        let anchor = [
            (camera.x as f64).rem_euclid(WAKE_NOISE_PERIOD) as f32,
            (camera.z as f64).rem_euclid(WAKE_NOISE_PERIOD) as f32,
            0.0,
            0.0,
        ];
        params.insert("wake_head".to_owned(), head.to_vec());
        params.insert("wake_bounds".to_owned(), bounds.to_vec());
        params.insert("wake_info".to_owned(), info.to_vec());
        params.insert("wake_points".to_owned(), points);
        params.insert("wake_anchor".to_owned(), anchor.to_vec());
    }
}
