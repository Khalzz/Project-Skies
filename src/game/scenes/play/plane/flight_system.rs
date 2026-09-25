use nalgebra::{Vector3, clamp};
use crate::game::scenes::play::plane::utils;
use crate::game::scenes::play::plane::engine;

pub struct AoA {
    pub aoa_pitch: f32,
    pub aoa_yaw: f32,
}

pub struct FlightSystem {
    pub velocity: nalgebra::Vector3<f32>,
    pub local_velocity: nalgebra::Vector3<f32>,
    pub local_angular_velocity: nalgebra::Vector3<f32>,
    pub aoa: AoA,
    pub last_velocity: nalgebra::Vector3<f32>,
    pub g_force: f32,
    pub input: Vector3<f32>, // x = roll, y = pitch, z = yaw
    /// Actual spooled net thrust (N), lagging behind the commanded/target
    /// value (see `engine::target_thrust`). This is what's applied to the
    /// rigidbody, not the instantaneous throttle demand.
    pub current_thrust: f32,
}

impl FlightSystem {
    pub fn new() -> Self {
        Self {
            velocity: nalgebra::Vector3::new(0.0, 0.0, 0.0),
            local_velocity: nalgebra::Vector3::new(0.0, 0.0, 0.0),
            local_angular_velocity: nalgebra::Vector3::new(0.0, 0.0, 0.0),
            aoa: AoA {
                aoa_pitch: 0.0,
                aoa_yaw: 0.0,
            },
            last_velocity: nalgebra::Vector3::new(0.0, 0.0, 0.0),
            g_force: 0.0,
            input: Vector3::new(0.0, 0.0, 0.0),
            current_thrust: 0.0,
        }
    }

    /// Spool the engine toward the throttle/altitude-commanded thrust and
    /// return the resulting force in world space, along the body's forward
    /// axis (+Z local). Only needs read access to the rigidbody
    /// (`altitude_m`/`rotation` snapshotted by the caller) - applying the
    /// force is the caller's job (see `AircraftPhysics::fixed_update`).
    ///
    /// `dt` is the fixed physics step - this runs exactly once per step, so
    /// the spool lag advances in simulated time, in lockstep with the world.
    pub fn compute_thrust(&mut self, altitude_m: f32, rotation: nalgebra::UnitQuaternion<f32>, throttle: f32, dt: f32) -> Vector3<f32> {
        let target = engine::target_thrust(throttle, altitude_m);

        // First-order lag toward target. alpha = 1 - e^(-dt/tau) composes
        // correctly for any dt.
        let tau = if target > self.current_thrust {
            if throttle >= engine::AB_GATE {
                engine::SPOOL_TAU_AB
            } else {
                engine::SPOOL_TAU_UP
            }
        } else {
            engine::SPOOL_TAU_DOWN
        };
        let alpha = 1.0 - (-dt / tau).exp();
        self.current_thrust += (target - self.current_thrust) * alpha;

        let thrust_local = nalgebra::Vector3::new(0.0, 0.0, self.current_thrust);
        rotation * thrust_local
    }
}