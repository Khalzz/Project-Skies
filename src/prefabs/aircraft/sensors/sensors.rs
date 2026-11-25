/*
    What the aircraft measures about itself each physics step: the air data its probes read and what its gyros and
    accelerometers feel. The control system and the HUD read these, never the physics directly, so a failing or
    noisy sensor can be added here later.
*/

use nalgebra::Vector3;
use rapier3d::prelude::RigidBody;

use crate::prefabs::world::atmosphere::Atmosphere;

const GRAVITY: f32 = 9.80665;
const INERTIAL_FILTER: f32 = 0.06; // seconds the gyros and accelerometers are smoothed over

#[derive(Clone, Copy, Default)]
pub struct Sensors {
    pub airspeed: f32,         // m/s
    pub mach: f32,
    pub altitude: f32,         // m above the sea
    pub alpha: f32,            // degrees, + with the air coming from below the nose
    pub beta: f32,             // degrees, + with the air coming from the right
    pub dynamic_pressure: f32, // Pa
    pub rates: Vector3<f32>,   // rad/s: pitch (nose up), roll (right wing down), yaw (nose right)
    pub load_factor: f32,      // g, 1 in level flight
    pub weight_on_wheels: bool,
    last_velocity: Option<Vector3<f32>>,
}

impl Sensors {
    pub fn measure(&mut self, body: &RigidBody, atmosphere: &Atmosphere, weight_on_wheels: bool, dt: f32) {
        self.weight_on_wheels = weight_on_wheels;
        let rotation = *body.rotation();
        let center = body.center_of_mass().coords;
        let velocity = *body.linvel();
        let air_data = atmosphere.air_at(center.y);

        // The air past the jet, in its own frame (nose -Z, +X right, +Y up)
        let air = rotation.inverse() * (atmosphere.wind_at(center) - velocity);
        self.airspeed = air.norm();
        self.alpha = air.y.atan2(air.z).to_degrees();
        self.beta = if self.airspeed > 1e-3 {
            (-air.x / self.airspeed).clamp(-1.0, 1.0).asin().to_degrees()
        } else {
            0.0
        };
        self.mach = self.airspeed / air_data.speed_of_sound;
        self.dynamic_pressure = 0.5 * air_data.density * self.airspeed * self.airspeed;
        self.altitude = center.y - atmosphere.sea_level;

        // The gear's jolts reach the gyros and accelerometers too: smoothed, like real ones are
        let smoothing = 1.0 - (-dt / INERTIAL_FILTER).exp();
        let spin = rotation.inverse() * body.angvel();
        let rates = Vector3::new(spin.x, -spin.z, -spin.y);
        self.rates += (rates - self.rates) * smoothing;

        // What an accelerometer feels: the acceleration minus gravity, along the jet's up
        let acceleration = self
            .last_velocity
            .map_or(Vector3::zeros(), |last| (velocity - last) / dt.max(1e-6));
        let felt = acceleration + Vector3::y() * GRAVITY;
        let load_factor = felt.dot(&(rotation * Vector3::y())) / GRAVITY;
        self.load_factor = match self.last_velocity {
            Some(_) => self.load_factor + (load_factor - self.load_factor) * smoothing,
            None => 1.0,
        };
        self.last_velocity = Some(velocity);
    }
}
