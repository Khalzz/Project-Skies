//! Heat - what an infrared seeker sees. Everything hot is a source with a
//! heat value (in rough units: a jet's engine at idle is 1), and a seeker
//! sees it dimmer the farther away it is (`apparent`). The sun is the one
//! source at no distance: it always looks SUN_APPARENT bright.
//!
//! An IR missile homes on the brightest thing in its field of view - so a
//! hotter source near its target (the sun behind it, an afterburner beside
//! it, one day a flare) can pull it off (see play::weapon::missile).

/// A jet's engine heat: at idle, at full dry power (military), and at full
/// afterburner - its exhaust plume is far hotter, and far bigger.
pub const IDLE_HEAT: f32 = 1.0;
pub const MILITARY_HEAT: f32 = 3.0;
pub const AFTERBURNER_HEAT: f32 = 12.0;
/// A wreck on fire, engine or not.
pub const BURNING_WRECK_HEAT: f32 = 6.0;
/// How bright the sun looks to a seeker - as bright as a jet at military
/// power ~1.7 km away, at full afterburner ~3.5 km away.
pub const SUN_APPARENT: f32 = 1.0;
/// How wide an IR seeker sees round where it's looking (degrees) - what's
/// inside it competes with what it's tracking.
pub const SEEKER_FIELD_DEG: f32 = 2.5;
/// A source has to look this many times brighter than what the seeker's on
/// to pull it off - so two about as hot don't flick it back and forth.
pub const SWITCH_RATIO: f32 = 1.5;

/// An aircraft's heat: its engine - from the throttle lever (0..1) and how
/// lit its afterburner is (0..1) - or, burning, its fire.
pub fn aircraft_heat(throttle: f32, afterburner: f32, burning: bool) -> f32 {
    if burning {
        return BURNING_WRECK_HEAT;
    }
    IDLE_HEAT + (MILITARY_HEAT - IDLE_HEAT) * throttle.clamp(0.0, 1.0) + (AFTERBURNER_HEAT - MILITARY_HEAT) * afterburner.clamp(0.0, 1.0)
}

/// How bright a source of `heat` looks from `distance` m: falling off with
/// the square of the distance in km (no closer than 100 m - it doesn't
/// blow up to infinity on top of it).
pub fn apparent(heat: f32, distance: f32) -> f32 {
    let km = (distance / 1000.0).max(0.1);
    heat / (km * km)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_afterburner_outshines_a_dry_engine() {
        let idle = aircraft_heat(0.0, 0.0, false);
        let military = aircraft_heat(1.0, 0.0, false);
        let reheat = aircraft_heat(1.0, 1.0, false);
        assert!(idle < military && military < reheat);
        assert_eq!(aircraft_heat(0.0, 0.0, true), BURNING_WRECK_HEAT);
    }

    #[test]
    fn farther_looks_dimmer() {
        assert!(apparent(3.0, 1000.0) > apparent(3.0, 2000.0));
        assert!((apparent(3.0, 2000.0) - 0.75).abs() < 1e-5);
        // A jet at military power: brighter than the sun close in, dimmer far out.
        assert!(apparent(MILITARY_HEAT, 1000.0) > SUN_APPARENT);
        assert!(apparent(MILITARY_HEAT, 3000.0) < SUN_APPARENT);
    }
}
