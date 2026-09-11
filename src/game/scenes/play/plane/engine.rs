//! Simplified F110-GE-129-class engine model: throttle -> thrust with a
//! dry/afterburner split and an altitude lapse. First-order spool lag is
//! applied on top of this in `FlightSystem::update_thrust` (it needs state);
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

use super::physics::rolling_rate::isa_density;

/// Flight-idle net thrust at sea level, N.
pub const IDLE_THRUST_SL: f32 = 4_000.0;
/// Military (max dry) net thrust at sea level, N. F110-GE-129 ~= 76 kN.
pub const MIL_THRUST_SL: f32 = 76_000.0;
/// Max afterburner net thrust at sea level, N. F110-GE-129 ~= 129 kN.
pub const MAX_THRUST_SL: f32 = 129_000.0;

/// Throttle fraction where the afterburner lights. Below this the lever spans
/// idle -> mil; above it, mil -> max AB. On the real jet the AB detent sits
/// near the top of the quadrant.
pub const AB_GATE: f32 = 0.85;

/// Spool time constants (seconds) for the first-order lag toward target
/// thrust, applied in `FlightSystem::update_thrust`.
pub const SPOOL_TAU_UP: f32 = 1.4; // idle -> mil, core spool-up
pub const SPOOL_TAU_AB: f32 = 0.5; // inside the AB range, fuel/nozzle light-off
pub const SPOOL_TAU_DOWN: f32 = 0.8; // throttle chop

/// Steady-state (fully spooled) net thrust for a throttle setting at a given
/// altitude (m, this game's Y).
pub fn target_thrust(throttle: f32, altitude_m: f32) -> f32 {
    let t = throttle.clamp(0.0, 1.0);
    let sea_level = if t <= AB_GATE {
        // idle -> military across the lower (dry) lever range
        let f = t / AB_GATE;
        IDLE_THRUST_SL + (MIL_THRUST_SL - IDLE_THRUST_SL) * f
    } else {
        // military -> full afterburner across the top of the lever
        let f = (t - AB_GATE) / (1.0 - AB_GATE);
        MIL_THRUST_SL + (MAX_THRUST_SL - MIL_THRUST_SL) * f
    };
    sea_level * altitude_lapse(altitude_m)
}

/// Thrust multiplier vs sea level. Turbofan net thrust falls roughly as
/// (rho/rho0)^0.7 - the usual first-cut approximation.
pub fn altitude_lapse(altitude_m: f32) -> f32 {
    let rho0 = isa_density(0.0);
    let rho = isa_density(altitude_m);
    (rho / rho0).powf(0.7)
}

/// How lit the afterburner is, 0..1, from throttle alone (0 at/below the
/// gate, 1 at full lever). Drives the visual flame so it only blooms in the
/// AB range, matching `target_thrust`'s split.
pub fn afterburner_activation(throttle: f32) -> f32 {
    let t = throttle.clamp(0.0, 1.0);
    if t <= AB_GATE {
        0.0
    } else {
        (t - AB_GATE) / (1.0 - AB_GATE)
    }
}
