//! Simplified turbofan model: throttle -> thrust with a
//! dry/afterburner split and an altitude lapse. First-order spool lag is
//! applied on top of this in `FlightSystem::compute_thrust` (it needs state);
//! everything here is a pure steady-state function.
//!
//! NOT a real engine deck - an illustrative model tuned so the F-16 stops
//! accelerating like a dragster:
//!  - the lever spans idle -> military over most of its travel, and only the
//!    top slice is the afterburner range (mil -> max), instead of the old
//!    "half stick = half of full-AB thrust" linear map;
//!  - net thrust falls with air density, so it can't hold full sea-level
//!    thrust at altitude / while climbing.
//!
//! Mach ram effects are deliberately left out - on a real F110 they'd raise
//! thrust with speed at low altitude, which works against the thing this is
//! trying to fix.

use serde::Deserialize;

use super::physics::rolling_rate::isa_density;

/// A plane's engine - `data.ron`'s `engine: ( ... )`. Thrust in newtons, at
/// sea level (it falls with altitude - see `altitude_lapse_exponent`);
/// times in seconds.
#[derive(Debug, Clone, Deserialize)]
pub struct EngineSpec {
    /// Flight idle.
    pub idle_thrust: f32,
    /// Military power - the most without afterburner.
    pub military_thrust: f32,
    /// Full afterburner. Same as `military_thrust` for an engine without one.
    pub max_thrust: f32,
    /// Throttle fraction where the afterburner lights: below it the lever
    /// spans idle -> military, above it military -> full afterburner (on
    /// the real jet the AB detent sits near the top of the quadrant). 1.0 =
    /// no afterburner.
    pub afterburner_gate: f32,
    /// How quickly thrust follows the lever (first-order time constants):
    /// spooling up below the gate, lighting up inside the afterburner
    /// range, and dropping on a throttle chop.
    pub spool_up_seconds: f32,
    pub spool_afterburner_seconds: f32,
    pub spool_down_seconds: f32,
    /// Thrust falls as (air density / sea level)^this - 0.7 is the usual
    /// turbofan first cut. Optional.
    #[serde(default = "default_lapse_exponent")]
    pub altitude_lapse_exponent: f32,
}

fn default_lapse_exponent() -> f32 {
    0.7
}

impl EngineSpec {
    /// Steady-state (fully spooled) net thrust for a throttle setting at a
    /// given altitude (m, this game's Y).
    pub fn target_thrust(&self, throttle: f32, altitude_m: f32) -> f32 {
        let t = throttle.clamp(0.0, 1.0);
        let gate = self.afterburner_gate.clamp(0.01, 1.0);
        let sea_level = if t <= gate {
            // idle -> military across the lower (dry) lever range
            let f = t / gate;
            self.idle_thrust + (self.military_thrust - self.idle_thrust) * f
        } else {
            // military -> full afterburner across the top of the lever
            let f = (t - gate) / (1.0 - gate);
            self.military_thrust + (self.max_thrust - self.military_thrust) * f
        };
        sea_level * self.altitude_lapse(altitude_m)
    }

    /// Thrust multiplier vs sea level.
    pub fn altitude_lapse(&self, altitude_m: f32) -> f32 {
        let rho0 = isa_density(0.0);
        let rho = isa_density(altitude_m);
        (rho / rho0).powf(self.altitude_lapse_exponent)
    }

    /// How lit the afterburner is, 0..1, from throttle alone (0 at/below the
    /// gate, 1 at full lever). Drives the visual flame so it only blooms in
    /// the AB range, matching `target_thrust`'s split.
    pub fn afterburner_activation(&self, throttle: f32) -> f32 {
        let t = throttle.clamp(0.0, 1.0);
        if t <= self.afterburner_gate || self.afterburner_gate >= 1.0 {
            0.0
        } else {
            (t - self.afterburner_gate) / (1.0 - self.afterburner_gate)
        }
    }

    /// The time constant for moving from `current` toward `target` thrust
    /// with the lever at `throttle`.
    pub fn spool_seconds(&self, current: f32, target: f32, throttle: f32) -> f32 {
        if target > current {
            if throttle >= self.afterburner_gate { self.spool_afterburner_seconds } else { self.spool_up_seconds }
        } else {
            self.spool_down_seconds
        }
    }
}
