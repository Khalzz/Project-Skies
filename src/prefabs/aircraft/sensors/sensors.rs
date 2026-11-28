/*
    What the aircraft measures about itself each physics step: the air data its probes read and what its gyros and
    accelerometers feel. The control system and the HUD read these, never the physics directly, so a failing or
    noisy sensor can be added here later.
*/

use nalgebra::{UnitQuaternion, Vector3};
use rapier3d::prelude::RigidBody;

use crate::prefabs::world::atmosphere::Atmosphere;

const GRAVITY: f32 = 9.80665;
const INERTIAL_FILTER: f32 = 0.06; // seconds the gyros and accelerometers are smoothed over

// The standard day the altimeter (on 29.92 inHg) and the airspeed indicator are calibrated to
const STANDARD_PRESSURE: f32 = 101_325.0; // Pa
const STANDARD_TEMPERATURE: f32 = 288.15; // K
const STANDARD_SPEED_OF_SOUND: f32 = 340.294; // m/s
const TROPOPAUSE: (f32, f32) = (11_000.0, 216.65); // m, K
const LAPSE_RATE: f32 = 0.0065; // K/m
const GAS_CONSTANT: f32 = 287.05;

#[derive(Clone, Copy)]
pub struct Sensors {
    pub airspeed: f32,            // m/s, true
    pub calibrated_airspeed: f32, // m/s: what the pitot-static airspeed indicator reads
    pub mach: f32,
    pub altitude: f32,         // m above the sea
    pub pressure_altitude: f32, // m: what the altimeter reads, set to 29.92 inHg
    pub vertical_speed: f32,   // m/s, + climbing
    // The inertial navigation system: where the jet points and how it moves over the ground
    pub attitude: UnitQuaternion<f32>, // the jet's frame in the world
    pub velocity: Vector3<f32>,        // m/s, in the world
    pub pitch: f32,                    // degrees, + nose up
    pub bank: f32,                     // degrees, + right wing down
    pub heading: f32,                  // degrees, 0 north (-Z), 90 east (+X)
    pub alpha: f32,            // degrees, + with the air coming from below the nose
    pub beta: f32,             // degrees, + with the air coming from the right
    pub dynamic_pressure: f32, // Pa
    pub rates: Vector3<f32>,   // rad/s: pitch (nose up), roll (right wing down), yaw (nose right)
    pub load_factor: f32,      // g, 1 in level flight
    pub lateral_g: f32,        // g felt toward the right
    pub longitudinal_g: f32,   // g felt forward: + speeding up
    pub weight_on_wheels: bool,
    last_velocity: Option<Vector3<f32>>,
}

impl Default for Sensors {
    fn default() -> Self {
        Sensors {
            airspeed: 0.0,
            calibrated_airspeed: 0.0,
            mach: 0.0,
            altitude: 0.0,
            pressure_altitude: 0.0,
            vertical_speed: 0.0,
            attitude: UnitQuaternion::identity(),
            velocity: Vector3::zeros(),
            pitch: 0.0,
            bank: 0.0,
            heading: 0.0,
            alpha: 0.0,
            beta: 0.0,
            dynamic_pressure: 0.0,
            rates: Vector3::zeros(),
            load_factor: 1.0,
            lateral_g: 0.0,
            longitudinal_g: 0.0,
            weight_on_wheels: false,
            last_velocity: None,
        }
    }
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
        self.calibrated_airspeed = calibrated_airspeed(self.mach, air_data.pressure);
        self.pressure_altitude = pressure_altitude(air_data.pressure);

        // The INS
        self.attitude = rotation;
        self.velocity = velocity;
        self.vertical_speed = velocity.y;
        let forward = rotation * -Vector3::z();
        let (right, up) = (rotation * Vector3::x(), rotation * Vector3::y());
        self.pitch = forward.y.clamp(-1.0, 1.0).asin().to_degrees();
        self.bank = (-right.y).atan2(up.y).to_degrees();
        self.heading = forward.x.atan2(-forward.z).to_degrees().rem_euclid(360.0);

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
        let lateral_g = felt.dot(&(rotation * Vector3::x())) / GRAVITY;
        let longitudinal_g = felt.dot(&(rotation * -Vector3::z())) / GRAVITY;
        if self.last_velocity.is_some() {
            self.lateral_g += (lateral_g - self.lateral_g) * smoothing;
            self.longitudinal_g += (longitudinal_g - self.longitudinal_g) * smoothing;
        }
        self.load_factor = match self.last_velocity {
            Some(_) => self.load_factor + (load_factor - self.load_factor) * smoothing,
            None => 1.0,
        };
        self.last_velocity = Some(velocity);
    }
}

// The impact pressure a pitot probe reads over the static pressure, at `mach`: isentropic below Mach 1,
// behind the probe's own shock (Rayleigh's pitot formula) above it
fn impact_ratio(mach: f32) -> f32 {
    if mach <= 1.0 {
        (1.0 + 0.2 * mach * mach).powf(3.5) - 1.0
    } else {
        166.921_58 * mach.powi(7) / (7.0 * mach * mach - 1.0).powf(2.5) - 1.0
    }
}

/*
    Calibrated airspeed: the speed that would make the same impact pressure on a standard day at sea level.
    The indicator only feels qc = p·ratio(M), so it reads lower than the true airspeed the higher the jet goes.
    Solved for the sea level Mach giving that qc over p₀ (the ratio only grows, so halving finds it).
*/
pub fn calibrated_airspeed(mach: f32, static_pressure: f32) -> f32 {
    let wanted = static_pressure * impact_ratio(mach) / STANDARD_PRESSURE;
    let (mut low, mut high) = (0.0_f32, 4.0_f32);
    for _ in 0..30 {
        let middle = (low + high) / 2.0;
        if impact_ratio(middle) < wanted {
            low = middle;
        } else {
            high = middle;
        }
    }
    (low + high) / 2.0 * STANDARD_SPEED_OF_SOUND
}

// Pressure altitude: the standard atmosphere's height for that static pressure
pub fn pressure_altitude(static_pressure: f32) -> f32 {
    let exponent = GAS_CONSTANT * LAPSE_RATE / GRAVITY;
    let (tropopause, tropopause_temperature) = TROPOPAUSE;
    let tropopause_pressure =
        STANDARD_PRESSURE * (tropopause_temperature / STANDARD_TEMPERATURE).powf(1.0 / exponent);
    if static_pressure >= tropopause_pressure {
        STANDARD_TEMPERATURE / LAPSE_RATE * (1.0 - (static_pressure / STANDARD_PRESSURE).powf(exponent))
    } else {
        tropopause
            + GAS_CONSTANT * tropopause_temperature / GRAVITY * (tropopause_pressure / static_pressure).ln()
    }
}
