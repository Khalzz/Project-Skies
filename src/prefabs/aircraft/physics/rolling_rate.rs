//! Simplified, illustrative roll-rate gain-scheduling model.
//!
//! NOT real aircraft FCS data -- a physically-motivated toy model for a
//! flight sim, tuned to produce a "hump" shaped roll-rate-vs-speed curve:
//! rises with airspeed at low speed (control-effectiveness limited),
//! peaks around a "corner" dynamic pressure, then tapers at high speed
//! (structural/load protection), and additionally falls off at high AoA.
//!
//! Key design choice: everything is driven by Equivalent Airspeed (EAS),
//! not Mach or raw TAS. EAS already encodes dynamic pressure, so a single
//! curve behaves correctly at every altitude -- no per-altitude tables
//! or corrections needed.

/// Standard-atmosphere air density (kg/m^3) at a given altitude (m).
/// Simplified troposphere-only model, valid to ~11,000 m.
pub fn isa_density(altitude_m: f32) -> f32 {
    const T0: f32 = 288.15; // K
    const P0: f32 = 101_325.0; // Pa
    const L: f32 = 0.0065; // K/m
    const R: f32 = 287.05; // J/(kg*K)
    const G: f32 = 9.80665; // m/s^2

    let alt = altitude_m.clamp(0.0, 11_000.0);
    let t = T0 - L * alt;
    let p = P0 * (t / T0).powf(G / (R * L));
    p / (R * t)
}

/// Equivalent airspeed (m/s) from true airspeed (m/s) and altitude (m).
/// EAS is the sea-level speed that would produce the same dynamic
/// pressure -- i.e. the speed that actually tracks structural load.
pub fn equivalent_airspeed(true_airspeed_ms: f32, altitude_m: f32) -> f32 {
    let rho = isa_density(altitude_m);
    let rho0 = isa_density(0.0);
    true_airspeed_ms * (rho / rho0).sqrt()
}

/// Tunable parameters for the roll-rate model. Expose these in a debug
/// UI / config asset to let designers reshape the curve without
/// touching code.
#[derive(Clone, Copy, Debug)]
pub struct RollRateParams {
    /// Peak achievable roll rate, deg/s, at the "corner" EAS.
    pub base_max_roll_rate_deg_s: f32,
    /// EAS (m/s) at which roll rate peaks.
    pub corner_eas_ms: f32,
    /// Shape exponent for the low-speed (control-effectiveness-limited) rise.
    pub low_speed_exponent: f32,
    /// Shape exponent for the high-speed (load-limited) taper.
    pub high_speed_exponent: f32,
    /// High-speed taper aggressiveness (higher = falls off faster past corner).
    pub high_speed_taper_gain: f32,
    /// AoA (deg) below which roll authority is unaffected.
    pub aoa_soft_start_deg: f32,
    /// AoA (deg) at which roll authority bottoms out (departure protection).
    pub aoa_limit_deg: f32,
    /// Floor multiplier so roll authority never hits exactly zero.
    pub aoa_floor: f32,
}

impl Default for RollRateParams {
    fn default() -> Self {
        Self {
            // This is the TARGET the force sim should be tuned to reach, not
            // a number to be edited down to match wherever the sim
            // currently falls short - see wing.rs's own roll_gain comment,
            // which still needs actual force-sim tuning (control_surface_area,
            // damping) to close the gap, not another edit here.
            base_max_roll_rate_deg_s: 324.0,
            corner_eas_ms: 400.0 * 0.514_444, // ~400 KEAS, in m/s
            // 1.0 (linear) rather than 0.5 (sqrt): steady roll rate for a
            // fixed stick deflection scales ~linearly with airspeed (the
            // dynamic pressure cancels between the aileron moment and the
            // roll-damping moment), and the sqrt gave far too much roll
            // authority at taxi speed - full stick at ~10 kt was ~50 deg/s.
            low_speed_exponent: 1.0,
            high_speed_exponent: 1.3,
            high_speed_taper_gain: 0.9,
            aoa_soft_start_deg: 15.0,
            aoa_limit_deg: 25.0,
            aoa_floor: 0.05,
        }
    }
}

/// 0..1 multiplier from EAS: rises to 1.0 at corner_eas, tapers above it.
fn speed_taper(eas_ms: f32, p: &RollRateParams) -> f32 {
    let ratio = eas_ms / p.corner_eas_ms;

    if ratio <= 1.0 {
        ratio.max(0.0).powf(p.low_speed_exponent)
    } else {
        let over = ratio - 1.0;
        1.0 / (1.0 + p.high_speed_taper_gain * over.powf(p.high_speed_exponent))
    }
    .clamp(0.0, 1.0)
}

/// 0..1 multiplier from angle of attack: full authority below the soft
/// start, ramps down to `aoa_floor` at the AoA limit.
fn aoa_taper(aoa_deg: f32, p: &RollRateParams) -> f32 {
    if aoa_deg <= p.aoa_soft_start_deg {
        1.0
    } else {
        let span = (p.aoa_limit_deg - p.aoa_soft_start_deg).max(f32::EPSILON);
        let ramp = ((aoa_deg - p.aoa_soft_start_deg) / span).clamp(0.0, 1.0);
        (1.0 - ramp).max(p.aoa_floor)
    }
}

/// Main entry point: max achievable roll rate (deg/s) for the current
/// flight condition. Call this once per aircraft per frame (or whenever
/// speed/altitude/AoA changes meaningfully) -- it's cheap.
pub fn max_roll_rate_deg_s(
    true_airspeed_ms: f32,
    altitude_m: f32,
    aoa_deg: f32,
    params: &RollRateParams,
) -> f32 {
    let eas = equivalent_airspeed(true_airspeed_ms, altitude_m);
    params.base_max_roll_rate_deg_s * speed_taper(eas, params) * aoa_taper(aoa_deg, params)
}

/// Optional: turn stick input (-1.0..1.0) into an actual roll rate,
/// applying the max-rate ceiling. Extend this with a rate-of-change
/// limiter (roll acceleration) if you want the "spool up" feel discussed
/// separately, rather than an instant snap to max rate.
pub fn commanded_roll_rate_deg_s(
    stick_input: f32,
    true_airspeed_ms: f32,
    altitude_m: f32,
    aoa_deg: f32,
    params: &RollRateParams,
) -> f32 {
    let max_rate = max_roll_rate_deg_s(true_airspeed_ms, altitude_m, aoa_deg, params);
    stick_input.clamp(-1.0, 1.0) * max_rate
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peaks_near_corner_speed() {
        let p = RollRateParams::default();
        let low = max_roll_rate_deg_s(100.0 * 0.514444, 0.0, 4.0, &p);
        let corner = max_roll_rate_deg_s(400.0 * 0.514444, 0.0, 4.0, &p);
        let high = max_roll_rate_deg_s(700.0 * 0.514444, 0.0, 4.0, &p);
        assert!(corner > low);
        assert!(corner > high);
    }

    #[test]
    fn high_aoa_reduces_roll_rate() {
        let p = RollRateParams::default();
        let normal = max_roll_rate_deg_s(300.0 * 0.514444, 3000.0, 5.0, &p);
        let high_aoa = max_roll_rate_deg_s(300.0 * 0.514444, 3000.0, 28.0, &p);
        assert!(high_aoa < normal);
    }

    #[test]
    fn eas_collapses_altitude_curves() {
        // Same EAS at different altitudes/TAS should give (nearly) the
        // same roll rate -- this is the whole point of driving the model
        // off EAS instead of raw airspeed or Mach.
        let p = RollRateParams::default();
        let target_eas = 300.0 * 0.514444;

        let rho0 = isa_density(0.0);
        let alt_m = 6000.0;
        let rho_alt = isa_density(alt_m);
        let tas_at_alt = target_eas * (rho0 / rho_alt).sqrt();

        let rate_sea_level = max_roll_rate_deg_s(target_eas, 0.0, 4.0, &p);
        let rate_at_altitude = max_roll_rate_deg_s(tas_at_alt, alt_m, 4.0, &p);

        assert!((rate_sea_level - rate_at_altitude).abs() < 1.0);
    }
}