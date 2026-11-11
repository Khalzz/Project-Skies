use nalgebra::Vector3;

use super::landing_gear::LandingGear;
use super::speed_brake::SpeedBrake;

/// The plane's mechanical systems - physics-side state, owned by
/// `AircraftPhysics` (their positions feed straight into forces). Landing
/// gear and the speed brake today. Flaps and variable-geometry wings can
/// sit here the same way, each as its own struct.
pub struct Airframe {
    pub landing_gear: LandingGear,
    pub speed_brake: SpeedBrake,
}

impl Airframe {
    /// `gear_down`: the gear is down and locked at spawn (a ground start).
    /// The speed brake starts stowed.
    pub fn new(gear_down: bool) -> Self {
        Self { landing_gear: LandingGear::new(gear_down), speed_brake: SpeedBrake::new() }
    }
}

/// Move `current` toward `target` by at most `rate_per_s * dt`, staying in
/// 0..1. Shared by the gear and the speed brake.
pub fn approach(current: f32, target: f32, rate_per_s: f32, dt: f32) -> f32 {
    let step = rate_per_s.max(0.0) * dt;
    (current + (target - current).clamp(-step, step)).clamp(0.0, 1.0)
}

/// Drag opposing `velocity` from an equivalent flat area `area` (m²) at
/// `air_density`. Zero below 1 m/s, so a parked jet isn't tugged.
pub fn dynamic_drag(velocity: Vector3<f32>, air_density: f32, area: f32) -> Vector3<f32> {
    let speed = velocity.magnitude();
    if speed < 1.0 || area <= 0.0 {
        return Vector3::zeros();
    }
    let dynamic_pressure = 0.5 * air_density * speed * speed;
    -velocity * (dynamic_pressure * area / speed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_opposes_the_velocity_and_grows_with_speed() {
        let slow = dynamic_drag(Vector3::z() * 100.0, 1.225, 1.0);
        let fast = dynamic_drag(Vector3::z() * 200.0, 1.225, 1.0);
        assert!(slow.z < 0.0);
        assert!(slow.x.abs() < 1e-3 && slow.y.abs() < 1e-3);
        // Dynamic pressure goes with speed squared, so twice the speed is
        // four times the drag.
        assert!((fast.z / slow.z - 4.0).abs() < 1e-2, "{} {}", fast.z, slow.z);
        assert_eq!(dynamic_drag(Vector3::z() * 0.2, 1.225, 1.0), Vector3::zeros());
    }
}
