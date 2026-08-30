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
        Self { pitch: 0.29, roll: 0.0, yaw: 0.0 }
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
}

impl PlaneControls {
    pub fn new() -> Self {
        Self { throttle: 0.0, elevator: 0.0, aileron: 0.0, rudder: 0.0, trim: Trim::new() }
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
            ControlInput::Elevator => controls.elevator,
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

// How fast the gear eases toward its target position each frame - same
// asymptotic lerp-toward-target pattern `ControlSurface::apply` uses for its
// own lerp_speed, not a fixed-duration timer.
const LANDING_GEAR_LERP_SPEED: f32 = 1.5;

struct LandingGearWheel {
    // Reuses the exact mesh names `WheelManager`'s physics side already
    // keys `RenderMessage`'s wheel data by (see
    // `Plane::apply_physics_feedback`) - same meshes, just also driven by
    // this whenever the gear is closed instead of always by live suspension.
    mesh_name: &'static str,
    // Gear down - this wheel's authored rest-pose translation in
    // res/F16/f16.gltf, i.e. where it sat before this feature existed.
    open_position: Vector3<f32>,
    // Gear up - tucked in near the fuselage centerline. Hand-picked, eyeball/
    // adjust these in-engine against the actual model rather than trusting
    // them blindly.
    closed_position: Vector3<f32>,
}

/// The landing gear's own state - just which position each wheel mesh should
/// snap to right now, no in-between: `closed` picks `open_position` or
/// `closed_position` directly, no lerp/timer.
pub struct LandingGear {
    // The commanded state - which position `update` writes each frame.
    pub closed: bool,
    wheels: Vec<LandingGearWheel>,
}

impl LandingGear {
    // `starts_closed` is the "define whether the plane starts with the gear
    // up or down" knob - set here, at construction (see `Plane::new`), since
    // "player" currently spawns already airborne at 1000m rather than on a
    // runway.
    fn new(starts_closed: bool) -> Self {
        Self {
            closed: starts_closed,
            wheels: vec![
                LandingGearWheel { mesh_name: "wheel-f", open_position: Vector3::new(0.0, -0.279, 0.756), closed_position: Vector3::new(0.0, 0.00, 0.456) },
                LandingGearWheel { mesh_name: "wheel-lb", open_position: Vector3::new(0.2, -0.266, 0.167), closed_position: Vector3::new(-0.08, 0.02, 0.367) },
                LandingGearWheel { mesh_name: "wheel-rb", open_position: Vector3::new(-0.2, -0.266, 0.167), closed_position: Vector3::new(0.08, 0.02, 0.367) },
            ],
        }
    }

    fn toggle(&mut self) {
        self.closed = !self.closed;
    }

    fn update(&mut self, model: &mut LoadedModel, delta_time: f32, queue: &wgpu::Queue) {
        let Some(meshes) = model.mesh_lists.get_mut("opaque") else { return };
        for wheel in &self.wheels {
            let Some(mesh) = meshes.get_mut(wheel.mesh_name) else { continue };
            let target = if self.closed { wheel.closed_position } else { wheel.open_position };
            mesh.transform.position = lerp_vector3(mesh.transform.position, target, delta_time * LANDING_GEAR_LERP_SPEED);
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
}

impl Plane {
    pub fn new() -> Self {
        Self {
            controls: PlaneControls::new(),
            input_locked: false,
            control_surfaces: Self::default_control_surfaces(),
            landing_gear: LandingGear::new(true),
            afterburner: Afterburner::new(),
            flight_data: FlightData { altimeter: 0.0, speedometer: 0.0, g_meter: 1.0 },
            previous_velocity: None,
            velocity_sample_elapsed: 0.0,
            stall: false,
            rng: rand::thread_rng(),
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
        // Raw world Y - this game's own "sea level" is Y=0 (see e.g.
        // play::scene::spawn_world's "world"/water node), so this doubles as
        // height above water without needing the wave-surface's own current
        // height subtracted out; a HUD altimeter reads the nominal/rest sea
        // level, not the instantaneous wave crest/trough under the plane.
        self.flight_data.altimeter = physics_message.translation.y;

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

        if let Some(meshes) = model.mesh_lists.get_mut("opaque") {
            if let Some(MetadataType::Wheels(wheels)) = physics_message.metadata.get("wheels") {
                for (index, wheel) in wheels.iter() {
                    if let Some(wheel_mesh) = meshes.get_mut(index.as_str()) {
                        let local_position = &wheel.local_position;
                        wheel_mesh.transform.position = Vector3::new(local_position.x / instance_scale.x, local_position.y / instance_scale.y, local_position.z / instance_scale.z);
                        wheel_mesh.update_transform(queue);
                    }
                }
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
            self.controls.elevator = input::get_axis("pitch_up", "pitch_down");
            self.controls.aileron = input::get_axis("roll_left", "roll_right");
            self.controls.rudder = input::get_axis("rudder_left", "rudder_right");
            // Absolute position, not a ramp: a trigger's deflection directly sets
            // throttle, like a real lever.
            self.controls.throttle = input::get_axis("throttle_down", "throttle_up").clamp(0.0, 1.0);
            self.controls.trim.update(delta_time);

            if input::is_action_just_pressed("toggle_landing_gear") {
                self.landing_gear.toggle();
            }
            if input::is_action_just_pressed("landing_gear_up") {
                self.landing_gear.closed = true;
            }
            if input::is_action_just_pressed("landing_gear_down") {
                self.landing_gear.closed = false;
            }
        }

        let Some(model_ref) = node.get_property::<ModelProperty>().map(|model| model.model_ref.clone()) else { return };
        let Some(model_instance) = app.game_models.get_mut(&model_ref) else { return };

        self.afterburner.update(self.controls.throttle, delta_time, &mut self.rng);

        for surface in &mut self.control_surfaces {
            surface.apply(&mut model_instance.model, &self.controls, delta_time, &app.renderer.queue);
        }

        self.landing_gear.update(&mut model_instance.model, delta_time, &app.renderer.queue);

        if let Some(meshes) = model_instance.model.mesh_lists.get_mut("transparent") {
            if let Some(afterburner_mesh) = meshes.get_mut("Afterburner") {
                let new_transform = Transform::new(afterburner_mesh.transform.position, afterburner_mesh.transform.rotation, Vector3::new(1.0, 1.0, self.afterburner.value));
                afterburner_mesh.change_transform(&app.renderer.queue, new_transform);
            }
        }
    }
}
