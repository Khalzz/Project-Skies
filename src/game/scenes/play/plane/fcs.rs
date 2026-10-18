/// The plane's own fly-by-wire flight-control-law state. Separate from
/// `PitchFlcs` (the physics-side struct in `WingManager` that actually runs
/// the g-command control law) the same way `PlaneControls` is separate from
/// the rigidbody it drives - this is what the pilot/toggle sees and sets,
/// not the control law's own internals. Its own struct rather than a bare
/// bool on `Plane` so more modes (roll/yaw autotrim, an AoA limiter, ...)
/// have somewhere to go later without adding more loose fields to `Plane`.
pub struct Fcs {
    // Whether this aircraft's pitch axis is fly-by-wire normal-g command
    // (like the real F-16's normal-mode pitch law - the pilot commands g,
    // the loop drives the stabilator to hold it and its integrator stands in
    // for trim) vs. the plain manual + persistent-trim path. Toggleable in
    // flight via "toggle_fbw_pitch" (B). Per-`Fcs` rather than a global
    // since not every future aircraft will be fly-by-wire; `false` reproduces
    // the manual/trim behavior exactly. See PitchFlcs for the control law.
    pub pitch_autotrim: bool,
}

impl Fcs {
    pub fn new() -> Self {
        // Pitch fly-by-wire: normal-g command law (see PitchFlcs). On by
        // default - this is the F-16. Toggle in flight with
        // "toggle_fbw_pitch" (B) to fall back to manual + trim.
        Self { pitch_autotrim: true }
    }
}
