use std::collections::HashMap;

use nalgebra::Vector3;

use super::flight_data::FlightData;

/// Everything read back out of the plane for HUD/debug display - written
/// each tick by `Plane::apply_physics_feedback`, read by the flight HUD
/// (`play::ui`), the F3/F7 debug panels (`render_pass.rs`), and
/// `play::scene::GameLogic` for the aero debug trail. Grouped together since
/// none of it is control logic - it's purely "what does the plane currently
/// show/report," as opposed to `Fcs` (what the plane is doing) or `controls`
/// (what's commanding it).
pub struct Instrumentation {
    pub flight_data: FlightData,
    pub previous_velocity: Option<Vector3<f32>>,
    // See PlaneSystems' old own doc comment (now here) for why this exists -
    // physics ticks at a fixed 120Hz on its own thread, decoupled from render
    // frame rate, so `RenderMessage::linvel` only actually changes once every
    // ~8.3ms; this accumulates real elapsed time across however many render
    // frames see no change, rather than dividing by just the one frame's own
    // (much smaller) delta_time once a change finally shows up.
    pub velocity_sample_elapsed: f32,
    // Every wing's own last_lift_force (see Wing's own field of the same
    // name), keyed by label ("Left wing", "Right elevator wing", etc.) -
    // written each tick by apply_physics_feedback from that tick's own
    // WingDebugData (same "wings" metadata debug_simulated_elevator_
    // control_input already reads - see that field's own doc comment),
    // read by play::scene::GameLogic::update to feed App::wing_lift_trail
    // for the F7 "Wing Lift Forces" chart. Empty until the first physics
    // tick reports back.
    pub wing_lift_forces: HashMap<String, Vector3<f32>>,
    // Always false today - no stall detection is actually implemented yet
    // (see the flight HUD's own stall_alert, which reads this) - kept rather
    // than dropped so that alert isn't silently dead code with nothing left
    // to ever light it up.
    pub stall: bool,
}

impl Instrumentation {
    pub fn new() -> Self {
        Self {
            flight_data: FlightData { altimeter: 0.0, speedometer: 0.0, g_meter: 1.0, aoa_x: 0.0, aoa_y: 0.0, aoa: 0.0, roll_rate: 0.0, pitch_rate: 0.0, yaw_rate: 0.0, mach: 0.0 },
            previous_velocity: None,
            velocity_sample_elapsed: 0.0,
            wing_lift_forces: HashMap::new(),
            stall: false,
        }
    }
}
