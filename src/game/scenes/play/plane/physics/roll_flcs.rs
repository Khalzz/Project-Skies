//! F-16-style roll-rate command fly-by-wire.
//!
//! On the real jet the stick doesn't move the flaperons - it commands a ROLL
//! RATE, and the flight control computer moves the flaperons and the
//! differential stabilators to hold it, up to a scheduled limit. The airframe
//! on its own can roll faster than that limit; the FLCS is what caps it.
//!
//! Here: the stick's roll rate comes from `rolling_rate::
//! commanded_roll_rate_deg_s` (the schedule - speed and AoA limited), and
//! this loop turns it into the -1..1 roll command the wings deflect by
//! (`Wing::roll_input`), through the real wing forces. Where the airframe
//! can't reach the command (low speed, high AoA) it saturates and the jet
//! rolls as fast as it physically can.
//!
//! Switchable (`ROLL_FLCS`): off, the stick goes straight to the surfaces -
//! full stick is full travel at any speed - which shows what the airframe
//! itself can do.

use std::sync::atomic::AtomicBool;

use rapier3d::prelude::RigidBody;

use super::rolling_rate::{commanded_roll_rate_deg_s, equivalent_airspeed, RollRateParams};

/// On: stick commands a roll rate (this loop). Off: stick straight to the
/// surfaces. Set from the F7 "Wing Surfaces" window - an atomic since that's
/// on the main thread and this runs on the physics thread.
pub static ROLL_FLCS: AtomicBool = AtomicBool::new(true);

/// Feed-forward: roll command per deg/s commanded at the corner speed -
/// about the inverse of how fast full surface travel rolls the airframe
/// there (~420 deg/s measured at 400 kt, FLCS off), so the command lands
/// close before any feedback. Scaled with speed (`authority_schedule`).
const KFF: f32 = 1.0 / 420.0;
/// Roll command per deg/s of roll-rate error.
const KP: f32 = 0.003;
/// Roll command per deg of accumulated roll-rate error - trims out what the
/// feed-forward misses.
const KI: f32 = 0.006;
/// Most of the command the integrator may hold, either way.
const MAX_INTEGRAL_COMMAND: f32 = 0.5;
/// Below this airspeed (m/s) the loop is inert and the stick goes straight
/// through - nothing to fly yet.
const MIN_ACTIVE_SPEED: f32 = 20.0;

pub struct RollFlcs {
    /// Accumulated roll-rate error (deg).
    integral: f32,
    /// Last tick's commanded and measured roll rate (deg/s).
    pub last_command_deg_s: f32,
    pub last_measured_deg_s: f32,
}

impl RollFlcs {
    pub fn new() -> Self {
        Self { integral: 0.0, last_command_deg_s: 0.0, last_measured_deg_s: 0.0 }
    }

    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// The -1..1 roll command for the wings this tick, from the stick's
    /// `aileron` (-1..1). + = rolling right (a positive body roll rate - the
    /// +X/left wing rising, same sign `commanded_roll_rate_deg_s` and the
    /// kinematic roll use). `dt` is the fixed physics step.
    pub fn update(&mut self, rigidbody: &RigidBody, aileron: f32, dt: f32) -> f32 {
        let rotation = *rigidbody.rotation();
        let linvel = *rigidbody.linvel();
        let airspeed = linvel.magnitude();
        let altitude = rigidbody.translation().y;
        let measured = (rotation.inverse() * rigidbody.angvel()).z.to_degrees();
        self.last_measured_deg_s = measured;

        if airspeed < MIN_ACTIVE_SPEED {
            self.integral = 0.0;
            self.last_command_deg_s = 0.0;
            return aileron.clamp(-1.0, 1.0);
        }

        // Same AoA the kinematic roll's schedule uses.
        let body_velocity = rotation.inverse() * linvel;
        let aoa_deg = (-body_velocity.y).atan2(body_velocity.z).to_degrees().abs();
        let params = RollRateParams::default();
        let command = commanded_roll_rate_deg_s(aileron, airspeed, altitude, aoa_deg, &params);
        self.last_command_deg_s = command;

        // How much roll a unit of command buys grows ~linearly with airspeed
        // (the surfaces' moment scales with dynamic pressure, the wings' own
        // roll damping with dynamic pressure / airspeed) - so the gains scale
        // the other way, to keep the loop equally responsive at any speed.
        let authority_schedule = (params.corner_eas_ms / equivalent_airspeed(airspeed, altitude).max(1.0)).clamp(0.5, 4.0);
        let gain_schedule = authority_schedule.min(2.0);

        let error = command - measured;
        let feed_forward = command * KFF * authority_schedule;
        let proportional = error * KP * gain_schedule;
        let integral_command = self.integral * KI * gain_schedule;

        // Only integrate while there's authority left to use - otherwise it
        // winds up while pinned against full travel.
        let unsaturated = (feed_forward + proportional + integral_command).abs() < 1.0;
        if unsaturated {
            self.integral += error * dt;
            let limit = MAX_INTEGRAL_COMMAND / (KI * gain_schedule);
            self.integral = self.integral.clamp(-limit, limit);
        }

        (feed_forward + proportional + self.integral * KI * gain_schedule).clamp(-1.0, 1.0)
    }
}
