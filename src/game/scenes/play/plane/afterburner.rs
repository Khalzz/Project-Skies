use rand::{rngs::ThreadRng, Rng};

use crate::engine::utils::lerps::lerp;

use super::engine;

/// The afterburner's own little animation state - flame scale with a bit of
/// random jitter (real afterburner flicker), lerped rather than snapping.
/// Its own struct for the same reason `Trim` is: it's a distinct piece of
/// behavior, not just a loose f32 sitting alongside unrelated fields. Owns
/// its own `ThreadRng` rather than being handed one each call - nothing else
/// ever needs random numbers, so there's no reason for `Plane` to carry one
/// just to pass through here.
///
/// Driven off `engine::afterburner_activation`, not raw throttle, so the
/// flame only blooms once the lever is past the AB gate - matching the
/// dry/afterburner split in the thrust model (`engine::target_thrust`).
pub struct Afterburner {
    pub value: f32,
    rng: ThreadRng,
}

impl Afterburner {
    pub fn new() -> Self {
        Self { value: 0.0, rng: rand::thread_rng() }
    }

    pub fn update(&mut self, throttle: f32, delta_time: f32) {
        let activation = engine::afterburner_activation(throttle);
        if activation > 0.0 {
            // Jitter kept small - `activation` only spans the narrow AB band
            // of the lever, so full-swing flicker read as a strobe.
            let target = (activation + self.rng.gen_range(-0.15..0.15)).max(0.0);
            self.value = lerp(self.value, target, delta_time * 12.0);
        } else {
            self.value = lerp(self.value, 0.0, delta_time * 3.0);
        }
    }
}
