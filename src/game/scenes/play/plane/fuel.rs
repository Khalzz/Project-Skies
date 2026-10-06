//! Internal fuel. The rigidbody mass in `data.ron` is the jet with these
//! tanks full; what's been burned comes off that mass. Drop tanks add
//! through `Fuel::add`, up to `capacity_kg`.

use serde::Deserialize;

/// A plane's internal fuel - `data.ron`'s `fuel: ( ... )` (optional: none).
#[derive(Clone, Debug, Deserialize)]
pub struct FuelSpec {
    /// The tanks, full (kg).
    pub capacity_kg: f32,
    /// Burn at idle, at military power (the afterburner gate), and at full
    /// afterburner (kg/s). The lever blends idle -> military below the gate
    /// and military -> afterburner above it, the same split as the thrust.
    pub idle_kg_s: f32,
    pub military_kg_s: f32,
    pub afterburner_kg_s: f32,
}

impl Default for FuelSpec {
    fn default() -> Self {
        Self { capacity_kg: 0.0, idle_kg_s: 0.0, military_kg_s: 0.0, afterburner_kg_s: 0.0 }
    }
}

impl FuelSpec {
    /// kg/s at `throttle` (0..1). `gate` is where the afterburner lights.
    pub fn burn_kg_s(&self, throttle: f32, gate: f32) -> f32 {
        let throttle = throttle.clamp(0.0, 1.0);
        let gate = gate.clamp(0.01, 1.0);
        if throttle <= gate {
            let along = throttle / gate;
            self.idle_kg_s + (self.military_kg_s - self.idle_kg_s) * along
        } else {
            let along = (throttle - gate) / (1.0 - gate);
            self.military_kg_s + (self.afterburner_kg_s - self.military_kg_s) * along
        }
    }
}

/// What's in the tanks right now.
#[derive(Clone, Debug)]
pub struct Fuel {
    pub kg: f32,
    pub capacity_kg: f32,
    idle_kg_s: f32,
    military_kg_s: f32,
    afterburner_kg_s: f32,
    /// Last whole kilogram sent to the physics body. Starts at the full
    /// tanks, so spawn doesn't send a report.
    reported_kg: f32,
    /// The tanks ran dry - the engine is cut until fuel is added.
    pub starved: bool,
}

impl Default for Fuel {
    fn default() -> Self {
        Self::from_spec(&FuelSpec::default())
    }
}

impl Fuel {
    /// Tanks full.
    pub fn from_spec(spec: &FuelSpec) -> Self {
        Self {
            kg: spec.capacity_kg.max(0.0),
            capacity_kg: spec.capacity_kg.max(0.0),
            idle_kg_s: spec.idle_kg_s.max(0.0),
            military_kg_s: spec.military_kg_s.max(0.0),
            afterburner_kg_s: spec.afterburner_kg_s.max(0.0),
            // Already the spawned weight. The body is told only once a
            // whole kilogram has actually changed.
            reported_kg: spec.capacity_kg.max(0.0).round(),
            starved: false,
        }
    }

    /// A spec edited while flying: the rates and the maximum follow it, and
    /// what's in the tanks stays, clamped to the new maximum.
    pub fn apply_spec(&mut self, spec: &FuelSpec) {
        self.capacity_kg = spec.capacity_kg.max(0.0);
        self.idle_kg_s = spec.idle_kg_s.max(0.0);
        self.military_kg_s = spec.military_kg_s.max(0.0);
        self.afterburner_kg_s = spec.afterburner_kg_s.max(0.0);
        self.kg = self.kg.clamp(0.0, self.capacity_kg);
        self.starved = self.capacity_kg > 0.0 && self.kg <= 0.0;
    }

    pub fn has_tanks(&self) -> bool {
        self.capacity_kg > 0.0
    }

    /// kg/s at `throttle`. `gate` is where the afterburner lights.
    pub fn burn_kg_s(&self, throttle: f32, gate: f32) -> f32 {
        FuelSpec {
            capacity_kg: self.capacity_kg,
            idle_kg_s: self.idle_kg_s,
            military_kg_s: self.military_kg_s,
            afterburner_kg_s: self.afterburner_kg_s,
        }
        .burn_kg_s(throttle, gate)
    }

    /// Puts `kg` in the tanks, up to the maximum. Back to flying on the
    /// engine once there's any.
    pub fn add(&mut self, kg: f32) {
        if self.capacity_kg <= 0.0 || kg <= 0.0 {
            return;
        }
        self.kg = (self.kg + kg).min(self.capacity_kg);
        if self.kg > 0.0 {
            self.starved = false;
        }
    }

    /// The body's mass (kg) with this much fuel. `full_mass` is the jet with
    /// the tanks full (its rigidbody mass, stores included).
    pub fn mass(&self, full_mass: f32) -> f32 {
        (full_mass - (self.capacity_kg - self.kg)).max(1.0)
    }

    /// Whole kilograms changed since the last report - the physics body is
    /// told then, not every gram.
    pub fn take_report(&mut self) -> Option<f32> {
        let reported = self.kg.round();
        if reported == self.reported_kg {
            return None;
        }
        self.reported_kg = reported;
        Some(self.kg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> FuelSpec {
        FuelSpec { capacity_kg: 3_240.0, idle_kg_s: 0.18, military_kg_s: 1.6, afterburner_kg_s: 7.0 }
    }

    #[test]
    fn burn_follows_the_throttle() {
        let spec = spec();
        assert!((spec.burn_kg_s(0.0, 0.85) - 0.18).abs() < 1e-4);
        assert!((spec.burn_kg_s(0.85, 0.85) - 1.6).abs() < 1e-4);
        assert!((spec.burn_kg_s(1.0, 0.85) - 7.0).abs() < 1e-4);
        let mid_dry = spec.burn_kg_s(0.425, 0.85);
        assert!(mid_dry > 0.18 && mid_dry < 1.6, "{mid_dry}");
    }

    #[test]
    fn full_tanks_weigh_the_published_mass_and_empty_weighs_less() {
        let mut fuel = Fuel::from_spec(&spec());
        assert!(fuel.take_report().is_none(), "a full spawn must not retell the body its weight");
        assert!((fuel.mass(11_900.0) - 11_900.0).abs() < 1e-3);
        let mut empty = fuel;
        empty.kg = 0.0;
        assert!((empty.mass(11_900.0) - (11_900.0 - 3_240.0)).abs() < 1e-3);
    }

    #[test]
    fn added_fuel_stops_at_the_maximum() {
        let mut fuel = Fuel::from_spec(&spec());
        fuel.kg = 0.0;
        fuel.starved = true;
        fuel.add(1_000.0);
        assert!((fuel.kg - 1_000.0).abs() < 1e-3);
        assert!(!fuel.starved);
        fuel.add(10_000.0);
        assert!((fuel.kg - 3_240.0).abs() < 1e-3);
    }
}
