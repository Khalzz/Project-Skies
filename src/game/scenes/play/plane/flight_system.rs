use std::time::Instant;

use nalgebra::{Vector3, clamp};
use rapier3d::prelude::RigidBody;
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
    /// Wall clock of the last `update_thrust` call, so the spool lag advances
    /// in real time regardless of how fast the physics thread's spin loop
    /// calls this (it isn't rate-limited, and the `delta_time` it passes is
    /// the last physics *step*'s dt, stale on iterations that don't step).
    last_thrust_update: Instant,
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
            last_thrust_update: Instant::now(),
        }
    }

    /// Spool the engine toward the throttle/altitude-commanded thrust and
    /// return the resulting force in world space, along the body's forward
    /// axis (+Z local) - the pure-computation half of the old
    /// `update_thrust`, split out so it only needs read access to the
    /// rigidbody (`altitude_m`/`rotation` snapshotted by the caller) instead
    /// of a live `&mut RigidBody`. That's what lets `AircraftUnit::compute`
    /// (see physics_logic.rs) call this from inside a rayon `par_iter_mut`
    /// pass over every aircraft at once - each aircraft only mutates its own
    /// `FlightSystem` (disjoint, safe in parallel) and never touches the
    /// shared `RigidBodySet` here. Applying the returned force back onto the
    /// rigidbody still happens serially afterward, in
    /// `AircraftUnit::apply`.
    ///
    /// `_delta_time` (the physics step dt) is ignored on purpose - it's stale
    /// on spin-loop iterations that don't advance a physics step. The spool
    /// lag is integrated against a real wall clock instead.
    pub fn compute_thrust(&mut self, altitude_m: f32, rotation: nalgebra::UnitQuaternion<f32>, throttle: f32) -> Vector3<f32> {
        let now = Instant::now();
        // Clamp so a pause/resume or a long hitch can't dump a huge dt into
        // the lag (which would let it snap straight to target).
        let dt = (now - self.last_thrust_update).as_secs_f32().min(0.1);
        self.last_thrust_update = now;

        let target = engine::target_thrust(throttle, altitude_m);

        // First-order lag toward target. alpha = 1 - e^(-dt/tau) composes
        // correctly for any dt, so the spin-loop call rate doesn't matter.
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

    /// Thin wrapper kept for any caller that still wants the old
    /// compute-and-apply-in-one-call shape - snapshots what `compute_thrust`
    /// needs off `rigidbody`, then applies the resulting force to it
    /// directly. `AircraftUnit` no longer uses this (see `compute_thrust`'s
    /// own doc comment); it's the non-parallel fallback.
    pub fn update_thrust(&mut self, rigidbody: &mut RigidBody, _delta_time: f32, throttle: f32) {
        let thrust_world = self.compute_thrust(rigidbody.translation().y, *rigidbody.rotation(), throttle);
        rigidbody.add_force(thrust_world, true);
    }
}