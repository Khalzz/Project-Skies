use crate::engine::input::input;

/// How far pilot trim has shifted each axis's neutral position - its own
/// struct rather than three loose fields on `PlaneControls`, since it's a
/// genuinely separate concept from the instantaneous stick/pedal input
/// `elevator`/`aileron`/`rudder` hold (trim persists, drifts slowly, and gets
/// added on top of whatever the stick is doing right now - see
/// `wing_manager.rs`'s own use of both together).
#[derive(Clone)]
pub struct Trim {
    pub pitch: f32,
    pub roll: f32,
    pub yaw: f32,
}

impl Trim {
    fn new() -> Self {
        // Pitch starts at -0.3 (requested directly, not derived) - roll/yaw
        // still default untrimmed (0.0), adjustable by hand (I/K/J/L/U/O,
        // see Trim::update below) same as before.
        Self { pitch: -0.16, roll: 0.0, yaw: 0.0 }
    }

    pub fn update(&mut self, delta_time: f32) {
        let trim_speed = 0.5;
        if input::is_action_pressed("trim_pitch_up") {
            self.pitch = (self.pitch + trim_speed * delta_time).clamp(-1.0, 1.0);
        }
        if input::is_action_pressed("trim_pitch_down") {
            self.pitch = (self.pitch - trim_speed * delta_time).clamp(-1.0, 1.0);
        }
        if input::is_action_pressed("trim_roll_left") {
            self.roll = (self.roll - trim_speed * delta_time).clamp(-1.0, 1.0);
        }
        if input::is_action_pressed("trim_roll_right") {
            self.roll = (self.roll + trim_speed * delta_time).clamp(-1.0, 1.0);
        }
        if input::is_action_pressed("trim_yaw_left") {
            self.yaw = (self.yaw - trim_speed * delta_time).clamp(-1.0, 1.0);
        }
        if input::is_action_pressed("trim_yaw_right") {
            self.yaw = (self.yaw + trim_speed * delta_time).clamp(-1.0, 1.0);
        }
    }
}

/// The pilot's controls - owned by `Plane` (main thread, fed from input),
/// sent every frame as this aircraft's physics input
/// (`node.set_physics_input`), read by `AircraftPhysics` via
/// `ctx.input::<PlaneControls>()`. Only pilot intent lives here - anything
/// the physics side computes (g, gear position, ...) comes back the other
/// way, in `AircraftState`.
#[derive(Clone)]
pub struct PlaneControls {
    pub throttle: f32,
    pub elevator: f32,
    pub aileron: f32,
    pub rudder: f32,
    pub trim: Trim,
    // Mirrors `Plane::fcs.pitch_autotrim` - when set, the pitch FLCS
    // (WingManager::pitch_flcs, see PitchFlcs) interprets `elevator` below
    // as a normal-g COMMAND, not a deflection.
    pub fly_by_wire_pitch_autotrim: bool,
    /// 0..1 toe-brake pressure on the main wheels (held input).
    pub wheel_brake: f32,
    /// Full brake on the main wheels regardless of `wheel_brake` - set at
    /// spawn so the jet doesn't creep away on idle thrust, released by the
    /// pilot ("toggle_parking_brake").
    pub parking_brake: bool,
}

impl PlaneControls {
    pub fn new() -> Self {
        Self { throttle: 0.0, elevator: 0.0, aileron: 0.0, rudder: 0.0, trim: Trim::new(), fly_by_wire_pitch_autotrim: false, wheel_brake: 0.0, parking_brake: true }
    }
}
