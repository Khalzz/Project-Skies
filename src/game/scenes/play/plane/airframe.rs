use super::landing_gear::LandingGear;

/// The plane's mechanical systems - landing gear today, a natural home for
/// speed brakes/flaps/variable-geometry wings/etc. later (see the
/// conversation this came out of). Deliberately just a plain grouping, not a
/// shared "mechanism" abstraction - add the next system as its own concrete
/// struct/field here; only worth factoring out the shared ramp-toward-target
/// logic (`LandingGear::tick`'s own clamp-step math) once a second one
/// actually exists and the duplication is real, not ahead of time.
pub struct Airframe {
    pub landing_gear: LandingGear,
}

impl Airframe {
    pub fn new() -> Self {
        Self { landing_gear: LandingGear::new(true) } // starts down
    }
}
