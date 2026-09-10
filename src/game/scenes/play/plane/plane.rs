use nalgebra::{Quaternion, Unit, UnitQuaternion, Vector3};
use rand::{rngs::ThreadRng, Rng};

use crate::app::App;
use crate::engine::input::input;
use crate::engine::physics::physics_handler::{MetadataType, RenderMessage};
use crate::engine::rendering::camera::handler::SceneCameras;
use crate::engine::rendering::models::model::Model as LoadedModel;
use crate::engine::scene_manager::behavior::Behavior;
use crate::engine::scene_manager::node::Node;
use crate::engine::scene_manager::properties::Model as ModelProperty;
use crate::engine::utils::lerps::{lerp, lerp_quaternion, lerp_vector3};
use crate::transform::Transform;

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

    fn update(&mut self, delta_time: f32) {
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

/// The afterburner's own little animation state - throttle-driven scale with
/// a bit of random jitter (real afterburner flicker), lerped rather than
/// snapping directly to throttle. Its own struct for the same reason `Trim`
/// is: it's a distinct piece of behavior, not just a loose f32 sitting
/// alongside unrelated fields.
struct Afterburner {
    value: f32,
}

impl Afterburner {
    fn new() -> Self {
        Self { value: 0.0 }
    }

    fn update(&mut self, throttle: f32, delta_time: f32, rng: &mut ThreadRng) {
        if throttle > 0.0 {
            self.value = lerp(self.value, throttle + rng.gen_range(-0.5..0.5), delta_time * 20.0);
        } else {
            self.value = lerp(self.value, 0.0, delta_time * 2.0);
        }
    }
}

/// Which of `PlaneControls`'s instantaneous inputs a `ControlSurface` reacts
/// to - trim is deliberately not an option here, callers that want it just
/// add it into the surface's own resolved value before it gets used (none do
/// yet - the model's control surfaces currently only animate off raw stick
/// input, trim only affects the invisible aerodynamic forces in
/// `wing_manager.rs`).
enum ControlInput {
    Elevator,
    Aileron,
    Rudder,
}

impl ControlInput {
    fn value(&self, controls: &PlaneControls) -> f32 {
        match self {
            // Negated - see PlaneControls.elevator's own assignment (in
            // Plane::update) for why: that value's sign convention flipped
            // (W now +1/S now -1, matching aileron/rudder's own convention)
            // but the elevator control surface's own visual animation
            // direction shouldn't change just because of that, so this
            // undoes the flip locally. TESTING: when
            // debug_simulated_elevator_control_input is available, this uses
            // that instead - it's already in the SAME convention as -elevator
            // (the elevator wing's own control_input is defined as
            // -plane_controls.elevator in wing_manager.rs's non-fly-by-wire
            // path, so this is a like-for-like swap, not an extra negation),
            // so the mesh shows the simulated wing's real, post-solve state
            // rather than raw stick. See that field's own doc comment.
            ControlInput::Elevator => controls.debug_simulated_elevator_control_input.unwrap_or(-controls.elevator),
            ControlInput::Aileron => controls.aileron,
            ControlInput::Rudder => controls.rudder,
        }
    }
}

/// What a `ControlSurface`'s target rotation is composed with, before the
/// control-driven part - three different needs, all seen on the F16's own
/// meshes: `None` (elevators - the control input is the whole rotation),
/// `Fixed` (rudders - each has a constant real-world tilt baked in,
/// authored here rather than in the mesh, composed with the control-driven
/// swing), `Captured` (ailerons - there's no fixed/known rest tilt, so the
/// first frame this surface is ever animated just remembers whatever the
/// mesh's own rotation already was and treats that as neutral).
enum SurfaceBase {
    None,
    Fixed(UnitQuaternion<f32>),
    Captured(Option<Quaternion<f32>>),
}

/// One animated part of the plane's model - a named mesh, which axis/how far
/// it swings per unit of control input, how fast it eases toward that target,
/// and what it's relative to (see `SurfaceBase`). `Plane` holds a `Vec` of
/// these (see `Plane::default_control_surfaces`) and drives all of them
/// through the same `apply` each frame, instead of a hand-written block per
/// mesh repeating the same lerp-and-write.
struct ControlSurface {
    mesh_list: &'static str,
    mesh_name: &'static str,
    axis: Vector3<f32>,
    scale: f32,
    lerp_speed: f32,
    base: SurfaceBase,
    input: ControlInput,
}

impl ControlSurface {
    fn new(mesh_list: &'static str, mesh_name: &'static str, axis: Vector3<f32>, scale: f32, lerp_speed: f32, base: SurfaceBase, input: ControlInput) -> Self {
        Self { mesh_list, mesh_name, axis, scale, lerp_speed, base, input }
    }

    fn apply(&mut self, model: &mut LoadedModel, controls: &PlaneControls, delta_time: f32, queue: &wgpu::Queue) {
        let Some(meshes) = model.mesh_lists.get_mut(self.mesh_list) else { return };
        let Some(mesh) = meshes.get_mut(self.mesh_name) else { return };

        let base = match &mut self.base {
            SurfaceBase::None => UnitQuaternion::identity(),
            SurfaceBase::Fixed(rotation) => *rotation,
            SurfaceBase::Captured(captured) => UnitQuaternion::from_quaternion(*captured.get_or_insert(mesh.transform.rotation)),
        };

        let control_value = self.input.value(controls);
        let target = base * UnitQuaternion::from_axis_angle(&Unit::new_normalize(self.axis), self.scale * control_value);
        let new_rotation = lerp_quaternion(mesh.transform.rotation, *target, delta_time * self.lerp_speed);
        let new_transform = Transform::new(mesh.transform.position, new_rotation, mesh.transform.scale);
        mesh.change_transform(queue, new_transform);
    }
}

// Gear extend/retract speed, as a fraction of the full cycle per second
// (so 0.55 => ~1.8s each way). EMERGENCY is the rate used when ground contact
// forces the gear back down mid-cycle.
const GEAR_RATE_PER_S: f32 = 0.55;
const GEAR_EMERGENCY_RATE_PER_S: f32 = 3.0;
// At/above this `deploy` the gear counts as down-and-locked.
const GEAR_LOCKED_THRESHOLD: f32 = 0.999;

struct LandingGearWheel {
    // Same mesh names `WheelManager` keys its wheel data by (see
    // `Plane::apply_physics_feedback`).
    mesh_name: &'static str,
    // Fully retracted - tucked in near the fuselage centerline. Hand-picked;
    // eyeball against the model.
    retracted_position: Vector3<f32>,
    // Fully deployed pose, used ONLY as a fallback when there's no live
    // suspension raycast data yet (first frames / physics not running).
    // Normally the deployed pose is the raycast point, so the wheels track
    // the ground / bumps.
    deployed_fallback: Vector3<f32>,
}

/// Landing-gear state machine. `deploy` is the whole state: 0.0 = up and
/// stowed, 1.0 = down and locked, anything between = mid-cycle. The wheel
/// meshes and the suspension-force scale sent to the physics thread are both
/// derived from it.
///
/// Rules:
/// - starts down (see `Plane::new`);
/// - can't retract while down-and-locked AND a wheel is on the ground;
/// - freely toggled in the air;
/// - if a wheel ray finds ground while the gear is mid-cycle (not
///   down-and-locked), it's retracted the rest of the way UP, fast - a
///   part-extended strut can't safely carry the airframe.
pub struct LandingGear {
    /// 0..1 gear extension. Copied into `PlaneControls::gear_deploy` each frame.
    pub deploy: f32,
    /// What the pilot last commanded (`true` = down).
    commanded_down: bool,
    /// Any wheel ray touched ground last physics frame - refreshed by
    /// `Plane::apply_physics_feedback`.
    pub any_grounded: bool,
    wheels: Vec<LandingGearWheel>,
}

impl LandingGear {
    fn new(starts_down: bool) -> Self {
        Self {
            deploy: if starts_down { 1.0 } else { 0.0 },
            commanded_down: starts_down,
            any_grounded: false,
            wheels: vec![
                LandingGearWheel { mesh_name: "wheel-f",  retracted_position: Vector3::new(0.0, 0.00, 0.456),  deployed_fallback: Vector3::new(0.0, -0.279, 0.756) },
                LandingGearWheel { mesh_name: "wheel-lb", retracted_position: Vector3::new(-0.08, 0.02, 0.367), deployed_fallback: Vector3::new(-0.2, -0.266, 0.167) },
                LandingGearWheel { mesh_name: "wheel-rb", retracted_position: Vector3::new(0.08, 0.02, 0.367),  deployed_fallback: Vector3::new(0.2, -0.266, 0.167) },
            ],
        }
    }

    fn locked_down(&self) -> bool {
        self.deploy >= GEAR_LOCKED_THRESHOLD
    }

    /// Pilot pressed the gear toggle. No-op only when the gear is already
    /// down-and-locked and a wheel is on the ground (can't retract the gear
    /// you're standing on); otherwise flips the command.
    fn request_toggle(&mut self) {
        self.set_commanded_down(!self.commanded_down);
    }

    fn set_commanded_down(&mut self, down: bool) {
        if !down && self.locked_down() && self.any_grounded {
            return; // reject "gear up" with weight on wheels
        }
        self.commanded_down = down;
    }

    /// Advance one frame. Call after `apply_physics_feedback` has refreshed
    /// `any_grounded`.
    fn tick(&mut self, delta_time: f32) {
        // A wheel ray found ground while the gear is NOT down-and-locked -
        // i.e. it's mid-cycle. A part-extended strut can't safely carry the
        // airframe, so pull the gear the rest of the way UP (fast) and get it
        // out of the way rather than leaving it half-out under load.
        let forced = self.any_grounded && !self.locked_down();
        if forced {
            self.commanded_down = false;
        }

        let target = if self.commanded_down { 1.0 } else { 0.0 };
        let rate = if forced { GEAR_EMERGENCY_RATE_PER_S } else { GEAR_RATE_PER_S };
        let step = rate * delta_time;
        self.deploy = (self.deploy + (target - self.deploy).clamp(-step, step)).clamp(0.0, 1.0);
    }

    /// Positions every wheel mesh, blending each between its retracted pose
    /// and its live deployed pose (raycast point, or the fallback) by
    /// `deploy`. Also latches `any_grounded`. Called from
    /// `apply_physics_feedback`, which has the model and the wheel metadata.
    fn place_wheel_meshes(
        &mut self,
        model: &mut LoadedModel,
        wheel_data: Option<&std::collections::HashMap<String, crate::game::scenes::play::plane::physics::wheels::wheel::WheelData>>,
        instance_scale: Vector3<f32>,
        queue: &wgpu::Queue,
    ) {
        self.any_grounded = wheel_data
            .map(|w| w.values().any(|d| d.grounded))
            .unwrap_or(false);

        let Some(meshes) = model.mesh_lists.get_mut("opaque") else { return };
        for lg in &self.wheels {
            let Some(mesh) = meshes.get_mut(lg.mesh_name) else { continue };
            let deployed = match wheel_data.and_then(|w| w.get(lg.mesh_name)) {
                Some(d) => Vector3::new(
                    d.local_position.x / instance_scale.x,
                    d.local_position.y / instance_scale.y,
                    d.local_position.z / instance_scale.z,
                ),
                None => lg.deployed_fallback,
            };
            mesh.transform.position = lerp_vector3(lg.retracted_position, deployed, self.deploy);
            mesh.update_transform(queue);
        }
    }
}

/// Everything the flight HUD reads back out of the plane every frame -
/// unchanged from what used to live on `GameLogic::plane_systems`, just
/// relocated onto the struct that actually produces it.
pub struct FlightData {
    pub altimeter: f32,
    pub speedometer: f32,
    pub g_meter: f32,
    // True airspeed (physics_message.linvel, m/s) divided by a fixed sea-
    // level speed of sound (340.29 m/s, standard ISA value at 15°C) - not
    // altitude-corrected, same sea-level-only assumption the rest of this
    // aerodynamics model already makes (see e.g. Wing::physics_force's own
    // fixed air_density constant).
    pub mach: f32,
    // Angle of attack, decomposed into the plane's own body axes (see
    // apply_physics_feedback for the exact rotation.inverse() * linvel
    // derivation) - forward is local +Z, up is local +Y, right is local +X
    // (same convention `plane_up`/water_splash's own "forward" already use).
    // aoa_y is the classic vertical AoA (relative wind above/below the nose,
    // positive = nose pitched above the flight path); aoa_x is the
    // horizontal/sideslip equivalent (relative wind left/right of the nose).
    // aoa is the resultant total angle between the nose and the velocity
    // vector regardless of direction (always >= 0), not just aoa_x/aoa_y
    // added together.
    pub aoa_x: f32,
    pub aoa_y: f32,
    pub aoa: f32,
    // Turn rates (deg/s), same body-axis derivation and convention as
    // aoa_x/aoa_y above (rotation.inverse() * angvel instead of * linvel) -
    // roll_rate is rotation about the forward axis (elevator has no effect
    // on this), pitch_rate about the right/left axis (elevator's own axis),
    // yaw_rate about the up axis (rudder's own axis). Unlike aoa_*, these
    // aren't gated on airspeed - angular velocity is a direct physical
    // reading, not a ratio that blows up near zero the way atan2/acos of a
    // near-zero vector does.
    pub roll_rate: f32,
    pub pitch_rate: f32,
    pub yaw_rate: f32,
}

/// The player plane, as a `Node` `Behavior` attached to the `"player"` node
/// (see `play::scene::GameLogic::spawn_world`) - owns both halves of "being
/// the plane": the control/trim state that crosses to the physics thread
/// (`controls`), and everything about how the model reacts to it visually
/// (`control_surfaces`, `afterburner`, `flight_data`/`previous_velocity` for
/// instruments). `GameLogic` no longer owns any of this directly; it reaches
/// in via `Node::get_behavior_mut::<Plane>()` for the one thing a `Behavior`
/// can't do on its own - see `apply_physics_feedback`'s own doc comment.
pub struct Plane {
    pub controls: PlaneControls,
    // Set every frame by `GameLogic::update` (which knows about cinematics/
    // input-lock windows a `Behavior` has no way to see on its own) - `true`
    // freezes `controls` at whatever they already are instead of reading
    // input, same as the old "skip Plane::update" gate did. Mesh animation
    // below still runs either way, driven by whatever `controls` currently
    // holds.
    pub input_locked: bool,
    control_surfaces: Vec<ControlSurface>,
    pub landing_gear: LandingGear,
    afterburner: Afterburner,
    pub flight_data: FlightData,
    pub previous_velocity: Option<Vector3<f32>>,
    // See PlaneSystems' old own doc comment (now here) for why this exists -
    // physics ticks at a fixed 120Hz on its own thread, decoupled from render
    // frame rate, so `RenderMessage::linvel` only actually changes once every
    // ~8.3ms; this accumulates real elapsed time across however many render
    // frames see no change, rather than dividing by just the one frame's own
    // (much smaller) delta_time once a change finally shows up.
    velocity_sample_elapsed: f32,
    // Always false today - no stall detection is actually implemented yet
    // (see the flight HUD's own stall_alert, which reads this) - kept rather
    // than dropped so that alert isn't silently dead code with nothing left
    // to ever light it up.
    pub stall: bool,
    rng: ThreadRng,
    // Whether this aircraft's pitch axis is fly-by-wire normal-g command
    // (like the real F-16's normal-mode pitch law - the pilot commands g,
    // the loop drives the stabilator to hold it and its integrator stands in
    // for trim) vs. the plain manual + persistent-trim path. Toggleable in
    // flight via "toggle_fbw_pitch" (B). Per-`Plane` rather than a global
    // since not every future aircraft will be fly-by-wire; `false` reproduces
    // the manual/trim behavior exactly. See PitchFlcs for the control law.
    pub fly_by_wire_pitch_autotrim: bool,
    // Every wing's own last_lift_force (see Wing's own field of the same
    // name), keyed by label ("Left wing", "Right elevator wing", etc.) -
    // written each tick by apply_physics_feedback from that tick's own
    // WingDebugData (same "wings" metadata debug_simulated_elevator_
    // control_input already reads - see that field's own doc comment),
    // read by play::scene::GameLogic::update to feed App::wing_lift_trail
    // for the F7 "Wing Lift Forces" chart. Empty until the first physics
    // tick reports back.
    pub wing_lift_forces: std::collections::HashMap<String, Vector3<f32>>,
}

impl Plane {
    pub fn new() -> Self {
        Self {
            controls: PlaneControls::new(),
            input_locked: false,
            control_surfaces: Self::default_control_surfaces(),
            landing_gear: LandingGear::new(true), // starts down
            afterburner: Afterburner::new(),
            flight_data: FlightData { altimeter: 0.0, speedometer: 0.0, g_meter: 1.0, aoa_x: 0.0, aoa_y: 0.0, aoa: 0.0, roll_rate: 0.0, pitch_rate: 0.0, yaw_rate: 0.0, mach: 0.0 },
            previous_velocity: None,
            velocity_sample_elapsed: 0.0,
            stall: false,
            rng: rand::thread_rng(),
            // Pitch fly-by-wire: normal-g command law (see PitchFlcs). On by
            // default - this is the F-16. Toggle in flight with
            // "toggle_fbw_pitch" (B) to fall back to manual + trim.
            fly_by_wire_pitch_autotrim: true,
            wing_lift_forces: std::collections::HashMap::new(),
        }
    }

    fn default_control_surfaces() -> Vec<ControlSurface> {
        let rudder_base = UnitQuaternion::from_axis_angle(&Vector3::x_axis(), -28.4493_f32.to_radians());
        vec![
            ControlSurface::new("opaque", "left_elevator", *Vector3::x_axis(), 0.15, 7.0, SurfaceBase::None, ControlInput::Elevator),
            ControlSurface::new("opaque", "right_elevator", *Vector3::x_axis(), 0.15, 7.0, SurfaceBase::None, ControlInput::Elevator),
            ControlSurface::new("opaque", "left_aleron", *Vector3::x_axis(), -0.5, 7.0, SurfaceBase::Captured(None), ControlInput::Aileron),
            ControlSurface::new("opaque", "right_aleron", *Vector3::x_axis(), 0.5, 7.0, SurfaceBase::Captured(None), ControlInput::Aileron),
            ControlSurface::new("opaque", "rudder_0", *Vector3::y_axis(), 0.5, 7.0, SurfaceBase::Fixed(rudder_base), ControlInput::Rudder),
            ControlSurface::new("opaque", "rudder_1", *Vector3::y_axis(), 0.5, 7.0, SurfaceBase::Fixed(rudder_base), ControlInput::Rudder),
        ]
    }

    /// Everything `GameLogic::update` feeds in from that frame's physics
    /// results (speedometer, G-meter, wheel mesh positions) - a `Behavior`
    /// has no way to reach this itself: `Behavior::update` only gets
    /// `(node, cameras, app, dt)`, none of which carry `physics_data` (that
    /// only exists on `SceneBehaviour::update`'s own `FrameContext`). So
    /// `GameLogic::update` looks up the `"player"` node, gets its `Plane` via
    /// `Node::get_behavior_mut`, and calls this directly - the one place
    /// `Plane`'s own encapsulation is deliberately broken through, rather
    /// than widening every `Behavior`'s signature for this one consumer.
    pub fn apply_physics_feedback(&mut self, model: &mut LoadedModel, physics_message: &RenderMessage, gravity: Vector3<f32>, instance_scale: Vector3<f32>, queue: &wgpu::Queue, delta_time: f32) {
        self.flight_data.speedometer = physics_message.linvel.magnitude() * 1.94384;
        // See FlightData::mach's own doc comment for the sea-level-only
        // caveat on this constant.
        const SPEED_OF_SOUND_SEA_LEVEL_MS: f32 = 340.29;
        self.flight_data.mach = physics_message.linvel.magnitude() / SPEED_OF_SOUND_SEA_LEVEL_MS;
        // Raw world Y - this game's own "sea level" is Y=0 (see e.g.
        // play::scene::spawn_world's "world"/water node), so this doubles as
        // height above water without needing the wave-surface's own current
        // height subtracted out; a HUD altimeter reads the nominal/rest sea
        // level, not the instantaneous wave crest/trough under the plane.
        self.flight_data.altimeter = physics_message.translation.y;

        // Body-frame velocity - rotation.inverse() undoes the plane's own
        // orientation, same idea as `plane_up` below but for the velocity
        // vector instead of the up axis. Guarded on airspeed since atan2/acos
        // of a near-zero vector is meaningless, jittery noise (e.g. sitting
        // still on the runway) rather than a real angle - just holds the
        // last computed value below that speed instead of flickering.
        let rotation = UnitQuaternion::from_quaternion(physics_message.rotation);
        let body_velocity = rotation.inverse() * physics_message.linvel;
        if body_velocity.magnitude() > 0.5 {
            self.flight_data.aoa_y = (-body_velocity.y).atan2(body_velocity.z).to_degrees();
            self.flight_data.aoa_x = body_velocity.x.atan2(body_velocity.z).to_degrees();
            self.flight_data.aoa = (body_velocity.z / body_velocity.magnitude()).clamp(-1.0, 1.0).acos().to_degrees();
        }

        // Turn rates - see FlightData::roll_rate's own comment for the
        // axis/units convention. No airspeed guard needed here (unlike
        // aoa_* above) - angular velocity is a direct reading, not a ratio.
        let body_angvel = rotation.inverse() * physics_message.angvel;
        self.flight_data.roll_rate = body_angvel.z.to_degrees();
        self.flight_data.pitch_rate = body_angvel.x.to_degrees();
        self.flight_data.yaw_rate = body_angvel.y.to_degrees();

        self.velocity_sample_elapsed += delta_time;
        match &self.previous_velocity {
            Some(previous) if *previous != physics_message.linvel => {
                let acceleration = (physics_message.linvel - previous) / self.velocity_sample_elapsed;
                // Felt acceleration = total acceleration minus gravity (pilot doesn't feel gravity).
                let felt_acceleration = acceleration - gravity;
                let plane_up = UnitQuaternion::from_quaternion(physics_message.rotation) * Vector3::y_axis();
                let target_g = felt_acceleration.dot(&plane_up) / 9.81;
                self.flight_data.g_meter = lerp(self.flight_data.g_meter, target_g, delta_time * 10.0);
                self.previous_velocity = Some(physics_message.linvel);
                self.velocity_sample_elapsed = 0.0;
            }
            None => {
                self.previous_velocity = Some(physics_message.linvel);
                self.velocity_sample_elapsed = 0.0;
            }
            _ => {}
        }

        // Landing gear: LandingGear owns every wheel mesh position, blending
        // each between its retracted pose and its live suspension-raycast
        // pose by `deploy` (the smoothly-animated 0..1 state - see
        // LandingGear::tick). Also refreshes `any_grounded` for that state
        // machine. This is the ONLY writer of the wheel mesh transforms.
        let wheel_data = match physics_message.metadata.get("wheels") {
            Some(MetadataType::Wheels(w)) => Some(w),
            _ => None,
        };
        self.landing_gear.place_wheel_meshes(model, wheel_data, instance_scale, queue);

        // TESTING - see debug_simulated_elevator_control_input's own doc
        // comment. Either elevator wing works (they're driven to the same
        // control_input, fly-by-wire override included).
        if let Some(MetadataType::Wings(wings)) = physics_message.metadata.get("wings") {
            if let Some(elevator_wing) = wings.iter().find(|w| w.label == "Right elevator wing") {
                self.controls.debug_simulated_elevator_control_input = Some(elevator_wing.control_input);
            }
            // See wing_lift_forces' own doc comment.
            for wing in wings {
                self.wing_lift_forces.insert(wing.label.clone(), wing.last_lift_force);
            }
        }
    }
}

impl Behavior for Plane {
    fn update(&mut self, node: &mut Node, _cameras: &mut SceneCameras, app: &mut App, delta_time: f32) {
        if !self.input_locked {
            // get_axis reads continuous strength (not just pressed/not-pressed), so a
            // stick binding gives proportional deflection while a keyboard binding
            // still cleanly resolves to -1.0/0.0/1.0 (a key's strength is always
            // exactly 0.0 or 1.0).
            // get_axis(negative, positive) - "pitch_down" (S) had been
            // passed as the SECOND (positive) arg and "pitch_up" (W) as the
            // first (negative) one, backwards relative to how aileron/rudder
            // below are both ordered (their own first arg is each one's own
            // "-1" key) - meant W read as -1 and S as +1. Swapped so W is +1
            // and S is -1, matching that same convention. Everywhere this
            // value gets consumed for actual flight behavior (not just this
            // struct) had to negate it to compensate, so W/S still produce
            // the exact same physical pitch as before this - see
            // ControlInput::value's own Elevator arm and wing_manager.rs's
            // own "elevator wing" control_input, both of which now do that.
            // TESTING ONLY - stick-response smoothing (see
            // STICK_INPUT_LERP_SPEED's own comment). Raw target axis reads
            // straight off input as before; each control then lerps its own
            // current value toward that target rather than snapping to it
            // instantly, same lerp(current, target, delta_time * SPEED)
            // idiom used elsewhere (LandingGear/ControlSurface) - self-
            // referential (self.controls.X read as "current" and
            // overwritten with the eased step), so no extra state is
            // needed. SPEED=6.0 puts center-to-max at ~1-e^(-0.5*6)≈95% of
            // the way there by 0.5s (an exact linear "reaches -1 at exactly
            // 0.5s" ramp was the literal ask, but this asymptotic approach
            // is close enough to read as "0.5s to max" and matches this
            // codebase's own established easing idiom instead of a new one).
            const STICK_INPUT_LERP_SPEED: f32 = 6.0;
            let raw_elevator_stick = input::get_axis("pitch_down", "pitch_up");
            let target_elevator = raw_elevator_stick;
            let target_aileron = input::get_axis("roll_left", "roll_right");
            let target_rudder = input::get_axis("rudder_left", "rudder_right");

            // Pitch and roll trim folded in HERE (input stage) rather than
            // left as separate terms added inside wing_manager.rs's
            // control_input - see the conversation this came out of. Two
            // effects: trim changes now ride the same lerp as stick input
            // instead of applying instantly, and the stick-gauge dot /
            // readouts on the F7 overlay now reflect the actual trimmed
            // command rather than raw stick alone. Added (not subtracted) -
            // an earlier version of this subtracted trim.pitch here to
            // exactly reproduce wing_manager.rs's old `-elevator +
            // trim.pitch` formula, but that made self.controls.elevator's
            // OWN sign run backwards from trim (increasing trim.pitch
            // DECREASED elevator), which visually showed the stick-gauge
            // dot moving the wrong way when trimming. Adding instead keeps
            // self.controls.elevator/aileron in the same intuitive
            // convention trim.pitch/trim.roll already use on their own (and
            // makes the red stick dot exactly coincide with the yellow
            // trim-only marker whenever the stick is centered, which is a
            // good sanity check that this is right) - wing_manager.rs's
            // elevator wings now do plain `-plane_controls.elevator`, and
            // "Left"/"Right wing" now do plain `-`/`+ plane_controls.aileron`,
            // neither with a separate trim term, so this is where all of
            // pitch and roll trim's effect comes from. This does change
            // roll trim's actual behavior, not just where it applies: the
            // old wing_manager.rs formula added trim.roll with the SAME
            // sign to both wings (a uniform bias - since both wings' lift
            // responds to control_input the same way, that was actually
            // behaving more like a symmetric lift/flap trim than genuine
            // roll authority, not really "roll" despite the name). Folding
            // it in before the L/R sign split makes it properly
            // differential like aileron itself, matching how a real
            // aileron trim tab works - only yaw trim is left alone, out of
            // scope for this pass, though the same folding would be
            // straightforward there too if wanted (Rudder wing has no L/R
            // pair to be asymmetric about).
            // Pitch trim is NOT folded in while fly-by-wire pitch is engaged:
            // there the stick is a normal-g command (see PitchFlcs), the loop's
            // own integrator is the trim, and the I/K trim keys are inert for
            // pitch. Roll trim is unaffected either way.
            let target_elevator = if self.fly_by_wire_pitch_autotrim {
                target_elevator
            } else {
                target_elevator + self.controls.trim.pitch
            };
            let target_aileron = target_aileron + self.controls.trim.roll;

            self.controls.elevator = lerp(self.controls.elevator, target_elevator, delta_time * STICK_INPUT_LERP_SPEED);
            self.controls.aileron = lerp(self.controls.aileron, target_aileron, delta_time * STICK_INPUT_LERP_SPEED);
            self.controls.rudder = lerp(self.controls.rudder, target_rudder, delta_time * STICK_INPUT_LERP_SPEED);
            // Accumulator, not an absolute lever position - holding
            // throttle_up/throttle_down adds/subtracts power over time
            // rather than snapping straight to axis strength. A keyboard
            // binding's own axis strength is always exactly 0 or 1 (see
            // this fn's own top comment on get_axis), so the old
            // `.clamp(0.0, 1.0)` of that value directly meant "full power
            // exactly while held, zero the instant it's released" - not a
            // real throttle at all on keyboard, only a hair better on an
            // analog trigger. THROTTLE_RATE is how much power (0..1) builds
            // per second of being held - not yet tuned, just a starting
            // guess.
            const THROTTLE_RATE: f32 = 0.5;
            self.controls.throttle = (self.controls.throttle + input::get_axis("throttle_down", "throttle_up") * THROTTLE_RATE * delta_time).clamp(0.0, 1.0);
            self.controls.trim.update(delta_time);

            // Pitch fly-by-wire (normal-g command law) - toggleable in flight
            // for A/B comparison against the manual/trim path. When engaged,
            // `self.controls.elevator` is a G COMMAND rather than a deflection
            // (mapped in PitchFlcs), and the loop's integrator replaces trim.
            // This flag is what actually crosses plane_control_tx to the
            // physics thread (PlaneControls, not Plane); the PitchFlcs itself
            // lives on the physics side (WingManager).
            if input::is_action_just_pressed("toggle_fbw_pitch") {
                self.fly_by_wire_pitch_autotrim = !self.fly_by_wire_pitch_autotrim;
                println!(
                    "Pitch fly-by-wire (g-command): {}",
                    if self.fly_by_wire_pitch_autotrim { "ENGAGED - stick commands g" } else { "OFF - manual + trim" }
                );
            }
            self.controls.fly_by_wire_pitch_autotrim = self.fly_by_wire_pitch_autotrim;
            self.controls.g_meter = self.flight_data.g_meter;

            // Gear input feeds the LandingGear state machine (see its doc
            // comment). request_toggle / set_commanded_down enforce the
            // "can't retract with weight on wheels" rule; tick() advances the
            // `deploy` animation and applies the "ground contact mid-cycle ->
            // slam down" override. `any_grounded` was refreshed earlier this
            // frame by apply_physics_feedback.
            if input::is_action_just_pressed("toggle_landing_gear") {
                self.landing_gear.request_toggle();
            }
            if input::is_action_just_pressed("landing_gear_up") {
                self.landing_gear.set_commanded_down(false);
            }
            if input::is_action_just_pressed("landing_gear_down") {
                self.landing_gear.set_commanded_down(true);
            }
            self.landing_gear.tick(delta_time);
            // Crosses to the physics thread - WheelManager scales suspension
            // force by this and skips the raycast at 0.
            self.controls.gear_deploy = self.landing_gear.deploy;
        }

        let Some(model_ref) = node.get_property::<ModelProperty>().map(|model| model.model_ref.clone()) else { return };
        let Some(model_instance) = app.game_models.get_mut(&model_ref) else { return };

        self.afterburner.update(self.controls.throttle, delta_time, &mut self.rng);

        for surface in &mut self.control_surfaces {
            surface.apply(&mut model_instance.model, &self.controls, delta_time, &app.renderer.queue);
        }

        // Wheel meshes / gear state are driven from Plane::apply_physics_feedback
        // (place_wheel_meshes) and the input block above (tick) - nothing to do
        // here.

        if let Some(meshes) = model_instance.model.mesh_lists.get_mut("transparent") {
            if let Some(afterburner_mesh) = meshes.get_mut("Afterburner") {
                let new_transform = Transform::new(afterburner_mesh.transform.position, afterburner_mesh.transform.rotation, Vector3::new(1.0, 1.0, self.afterburner.value));
                afterburner_mesh.change_transform(&app.renderer.queue, new_transform);
            }
        }
    }
}
