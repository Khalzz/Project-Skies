//! F-16-style yaw fly-by-wire.
//!
//! On the real jet the pedals don't move the rudder directly either - the
//! flight control computer's yaw channel:
//! - turns the pedals into a SIDESLIP command, scaled down as dynamic
//!   pressure rises, so full pedal at Mach 1 gives a few degrees of skid,
//!   not the airframe's full-rudder skid;
//! - damps yaw with a yaw damper (yaw-rate feedback, washed out so a steady
//!   turn isn't fought);
//! - adds rudder when rolling at angle of attack (the aileron-rudder
//!   interconnect, ARI), so the roll stays coordinated instead of the nose
//!   swinging the wrong way (adverse yaw).
//!
//! This loop turns that into the -1..1 rudder command the fin's rudder
//! deflects by, through the real fin forces. Switchable (`YAW_FLCS`): off,
//! the pedals go straight to the rudder - the raw airframe.
//!
//! Signs (from the physics): a positive rudder input pushes the tail toward
//! +X, swinging the nose right - which reads as positive sideslip (the air
//! from the +X/left side, `atan2(v.x, v.z)`) and a negative yaw rate.

use std::sync::atomic::AtomicBool;

use rapier3d::prelude::RigidBody;

use super::rolling_rate::isa_density;

/// On: pedals command sideslip (this loop). Off: pedals straight to the
/// rudder. Set from the F7 "Wing Surfaces" window.
pub static YAW_FLCS: AtomicBool = AtomicBool::new(true);

/// Most sideslip full pedal commands, at or below Q_REF (deg).
const MAX_SIDESLIP_DEG: f32 = 10.0;
/// Above this dynamic pressure (Pa, ~250 kt at sea level) the sideslip full
/// pedal commands shrinks as Q_REF / q ...
const Q_REF: f32 = 10_000.0;
/// ... but never below this (deg).
const MIN_MAX_SIDESLIP_DEG: f32 = 2.0;

/// Rudder per degree of commanded sideslip, before feedback - about the
/// inverse of how much sideslip full rudder holds on the raw airframe
/// (~12 deg), so the command lands close straight away.
const KFF: f32 = 1.0 / 12.0;
/// Rudder per degree of sideslip error.
const KP: f32 = 0.05;
/// Rudder per degree-second of accumulated sideslip error.
const KI: f32 = 0.03;
/// Most of the command the integrator may hold, either way.
const MAX_INTEGRAL_COMMAND: f32 = 0.4;
/// Yaw damper: rudder per deg/s of (washed-out) yaw rate.
const K_YAW_DAMPER: f32 = 0.02;
/// Washout: yaw rate held this long (s) stops being damped - a steady turn
/// isn't fought, only yawing that comes and goes.
const WASHOUT_SECONDS: f32 = 1.0;
/// ARI: rudder per unit of roll command, at full ARI_FULL_AOA_DEG and above.
const ARI_GAIN: f32 = 0.3;
const ARI_FULL_AOA_DEG: f32 = 20.0;
/// Below this airspeed (m/s) the loop is inert - pedals straight through.
const MIN_ACTIVE_SPEED: f32 = 20.0;

pub struct YawFlcs {
    /// Accumulated sideslip error (deg s).
    integral: f32,
    /// Yaw rate's slow average - what the washout takes away (deg/s).
    yaw_rate_average: f32,
    /// Last tick's commanded and measured sideslip (deg).
    pub last_command_deg: f32,
    pub last_measured_deg: f32,
}

impl YawFlcs {
    pub fn new() -> Self {
        Self { integral: 0.0, yaw_rate_average: 0.0, last_command_deg: 0.0, last_measured_deg: 0.0 }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// The -1..1 rudder command this tick, from the `pedals` (-1..1, trim
    /// included) and this tick's `roll_command` (the roll FLCS's output,
    /// for the ARI). `dt` is the fixed physics step.
    pub fn update(&mut self, rigidbody: &RigidBody, pedals: f32, roll_command: f32, dt: f32) -> f32 {
        let rotation = *rigidbody.rotation();
        let body_velocity = rotation.inverse() * rigidbody.linvel();
        let airspeed = body_velocity.magnitude();

        if airspeed < MIN_ACTIVE_SPEED {
            self.reset();
            return pedals.clamp(-1.0, 1.0);
        }

        let sideslip = body_velocity.x.atan2(body_velocity.z).to_degrees();
        let angle_of_attack = (-body_velocity.y).atan2(body_velocity.z).to_degrees();
        let yaw_rate = (rotation.inverse() * rigidbody.angvel()).y.to_degrees();
        self.last_measured_deg = sideslip;

        // Pedals -> sideslip, less of it the faster the jet goes.
        let q = 0.5 * isa_density(rigidbody.translation().y) * airspeed * airspeed;
        let max_sideslip = (MAX_SIDESLIP_DEG * (Q_REF / q.max(1.0)).min(1.0)).max(MIN_MAX_SIDESLIP_DEG);
        let command = pedals.clamp(-1.0, 1.0) * max_sideslip;
        self.last_command_deg = command;

        // Yaw damper on the washed-out yaw rate. A nose-right yaw is a
        // negative rate, and a positive rudder input swings it right - so a
        // positive gain on the rate pushes back against it.
        let ease = 1.0 - (-dt / WASHOUT_SECONDS).exp();
        self.yaw_rate_average += (yaw_rate - self.yaw_rate_average) * ease;
        let damper = K_YAW_DAMPER * (yaw_rate - self.yaw_rate_average);

        // ARI: rolling right (positive roll command) yaws the nose left
        // (adverse yaw) - right rudder (positive) counters it, more of it
        // the higher the angle of attack.
        let ari = ARI_GAIN * roll_command * (angle_of_attack / ARI_FULL_AOA_DEG).clamp(0.0, 1.0);

        let error = command - sideslip;
        let feed_forward = command * KFF;
        let proportional = error * KP;
        let integral_command = self.integral * KI;

        // Only integrate while there's rudder left - no winding up against
        // full travel.
        let unsaturated = (feed_forward + proportional + integral_command + damper + ari).abs() < 1.0;
        if unsaturated {
            self.integral += error * dt;
            let limit = MAX_INTEGRAL_COMMAND / KI;
            self.integral = self.integral.clamp(-limit, limit);
        }

        (feed_forward + proportional + self.integral * KI + damper + ari).clamp(-1.0, 1.0)
    }
}
