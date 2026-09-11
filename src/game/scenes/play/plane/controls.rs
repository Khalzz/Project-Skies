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

/// Sent across the physics thread's `plane_control_tx` channel every frame
/// (see `Plane`'s own doc comment) - the physics-facing half of a plane's
/// state, as opposed to `Plane` itself, which also owns the rendering side
/// (mesh animation) the physics thread has no business touching.
#[derive(Clone)]
pub struct PlaneControls {
    pub throttle: f32,
    pub elevator: f32,
    pub aileron: f32,
    pub rudder: f32,
    pub trim: Trim,
    // Mirrors Plane::fly_by_wire_pitch_autotrim (see that field's own doc
    // comment) - PlaneControls, not Plane itself, is what actually crosses
    // the plane_control_tx channel to the physics thread, where the pitch
    // FLCS (WingManager::pitch_flcs, see PitchFlcs) lives - that's where the
    // rigidbody and wing/airfoil data actually are. When set, `elevator`
    // below is interpreted by that loop as a normal-g COMMAND, not a
    // deflection.
    pub fly_by_wire_pitch_autotrim: bool,
    // The SAME g_meter shown on the F7 debug overlay ("G: {:.1}") - computed
    // in Plane::apply_physics_feedback from real velocity-delta sampling,
    // copied here each frame for any physics-side consumer that wants the
    // exact HUD number. NOTE the pitch FLCS does NOT use this - it computes
    // its own g fresh from linvel deltas on the physics thread to avoid this
    // field's one-render-frame channel lag (see PitchFlcs::update).
    pub g_meter: f32,
    // Mirrors `Plane::landing_gear.deploy` (0 = up/stowed, 1 = down/locked) -
    // crosses to the physics thread so WheelManager::update can scale the
    // suspension force by it (a part-extended strut can't hold the airframe)
    // and skip the raycast entirely at 0.
    pub gear_deploy: f32,
    // TESTING ONLY - the opposite direction from every other field here:
    // written by Plane::apply_physics_feedback from that tick's own
    // WingDebugData (see that fn's own comment), read by ControlInput::value
    // to make the elevator control-surface MESH animate off what the
    // simulated elevator wing's control_input actually is, instead of raw
    // player input - so a fly-by-wire solve override (or anything else that
    // makes the simulated wing diverge from the stick) is visible in real
    // time on the model itself. Never sent to the physics thread (nothing
    // there reads it) and never set anywhere PlaneControls gets built fresh
    // (Plane::new/PlaneControls::new), only mutated in place afterward -
    // None until the first physics tick reports back.
    pub debug_simulated_elevator_control_input: Option<f32>,
}

impl PlaneControls {
    pub fn new() -> Self {
        Self { throttle: 0.0, elevator: 0.0, aileron: 0.0, rudder: 0.0, trim: Trim::new(), fly_by_wire_pitch_autotrim: false, g_meter: 1.0, gear_deploy: 1.0, debug_simulated_elevator_control_input: None }
    }
}
