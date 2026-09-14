use nalgebra::{UnitQuaternion, Vector3};

use crate::app::App;
use crate::engine::input::input;
use crate::engine::physics::physics_handler::{MetadataType, RenderMessage};
use crate::engine::rendering::camera::handler::SceneCameras;
use crate::engine::rendering::models::model::Model as LoadedModel;
use crate::engine::scene_manager::behavior::Behavior;
use crate::engine::scene_manager::node::Node;
use crate::engine::scene_manager::properties::{Model as ModelProperty, Transform3D};
use crate::engine::utils::lerps::lerp;
use crate::transform::Transform;

use super::afterburner::Afterburner;
use super::airframe::Airframe;
use super::control_surfaces::{ControlInput, ControlSurface, SurfaceBase};
use super::controls::PlaneControls;
use super::fcs::Fcs;
use super::instrumentation::Instrumentation;

pub struct Plane {
    pub controls: PlaneControls,
    pub input_locked: bool,
    control_surfaces: Vec<ControlSurface>,
    afterburner: Afterburner,
    pub airframe: Airframe,
    pub instrumentation: Instrumentation,
    pub fcs: Fcs,
}

impl Plane {
    // Cap on App::aero_debug_trail/App::wing_lift_trail (see those fields'
    // own doc comments) - oldest sample drops once this is exceeded, so the
    // F7 overlay reads as a "recent history" trail rather than growing
    // forever while it's on.
    const AERO_DEBUG_TRAIL_CAPACITY: usize = 500;

    pub fn new() -> Self {
        Self {
            controls: PlaneControls::new(),
            input_locked: false,
            control_surfaces: Self::default_control_surfaces(),
            afterburner: Afterburner::new(),
            airframe: Airframe::new(),
            instrumentation: Instrumentation::new(),
            fcs: Fcs::new(),
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

    // Renders all the feedback information of the physiscs systems torwards the utilizable rendering in the main thread
    pub fn apply_physics_feedback(&mut self, model: &mut LoadedModel, physics_message: &RenderMessage, instance_scale: Vector3<f32>, queue: &wgpu::Queue, delta_time: f32) {
        self.instrumentation.update(physics_message, delta_time);

        let wheel_data = match physics_message.metadata.get("wheels") {
            Some(MetadataType::Wheels(w)) => Some(w),
            _ => None,
        };
        self.airframe.landing_gear.place_wheel_meshes(model, wheel_data, instance_scale, queue);

        if let Some(MetadataType::Wings(wings)) = physics_message.metadata.get("wings") {
            if let Some(elevator_wing) = wings.iter().find(|w| w.label == "Right elevator wing") {
                self.controls.debug_simulated_elevator_control_input = Some(elevator_wing.control_input);
            }
        }
    }
}

impl Behavior for Plane {
    fn update(&mut self, node: &mut Node, _cameras: &mut SceneCameras, app: &mut App, delta_time: f32) {
        if !self.input_locked {
            const STICK_INPUT_LERP_SPEED: f32 = 6.0;
            let target_elevator = input::get_axis("pitch_down", "pitch_up");
            let target_aileron = input::get_axis("roll_left", "roll_right");
            let target_rudder = input::get_axis("rudder_left", "rudder_right");

            let target_elevator = if self.fcs.pitch_autotrim {
                target_elevator
            } else {
                target_elevator + self.controls.trim.pitch
            };

            let target_aileron = target_aileron + self.controls.trim.roll;

            self.controls.elevator = lerp(self.controls.elevator, target_elevator, delta_time * STICK_INPUT_LERP_SPEED);
            self.controls.aileron = lerp(self.controls.aileron, target_aileron, delta_time * STICK_INPUT_LERP_SPEED);
            self.controls.rudder = lerp(self.controls.rudder, target_rudder, delta_time * STICK_INPUT_LERP_SPEED);

            const THROTTLE_RATE: f32 = 0.5;
            self.controls.throttle = (self.controls.throttle + input::get_axis("throttle_down", "throttle_up") * THROTTLE_RATE * delta_time).clamp(0.0, 1.0);
            self.controls.trim.update(delta_time);

            if input::is_action_just_pressed("toggle_fbw_pitch") {
                self.fcs.pitch_autotrim = !self.fcs.pitch_autotrim;
                println!(
                    "Pitch fly-by-wire (g-command): {}",
                    if self.fcs.pitch_autotrim { "ENGAGED - stick commands g" } else { "OFF - manual + trim" }
                );
            }
            self.controls.fly_by_wire_pitch_autotrim = self.fcs.pitch_autotrim;
            self.controls.g_meter = self.instrumentation.flight_data.g_meter;

            if input::is_action_just_pressed("toggle_landing_gear") {
                self.airframe.landing_gear.request_toggle();
            }
            if input::is_action_just_pressed("landing_gear_up") {
                self.airframe.landing_gear.set_commanded_down(false);
            }
            if input::is_action_just_pressed("landing_gear_down") {
                self.airframe.landing_gear.set_commanded_down(true);
            }
            self.airframe.landing_gear.tick(delta_time);
            self.controls.gear_deploy = self.airframe.landing_gear.deploy;
        }

        let Some(model_ref) = node.get_property::<ModelProperty>().map(|model| model.model_ref.clone()) else { return };
        let Some(model_instance) = app.game_models.get_mut(&model_ref) else { return };

        self.afterburner.update(self.controls.throttle, delta_time);

        for surface in &mut self.control_surfaces {
            surface.apply(&mut model_instance.model, &self.controls, delta_time, &app.renderer.queue);
        }

        if let Some(meshes) = model_instance.model.mesh_lists.get_mut("transparent") {
            if let Some(afterburner_mesh) = meshes.get_mut("Afterburner") {
                let new_transform = Transform::new(afterburner_mesh.transform.position, afterburner_mesh.transform.rotation, Vector3::new(1.0, 1.0, self.afterburner.value));
                afterburner_mesh.change_transform(&app.renderer.queue, new_transform);
            }
        }
    }

    // Reacts to this node's own physics results - the generic replacement
    // for what used to be `play::scene::GameLogic::update` hand-picking the
    // "player" node by name and calling `apply_physics_feedback` on it
    // directly (see that fn's own doc comment). `physics_message` is `None`
    // on any frame the physics thread hasn't reported back for this node yet
    // (e.g. before the first tick, or a node with no Physics property at
    // all) - nothing to react to yet, so this is a no-op then.
    //
    // The F7 aero-debug-trail/wing-lift-trail bookkeeping lives here too,
    // right after apply_physics_feedback, rather than back in GameLogic::
    // update where it used to sit - SceneBehaviour::update (where GameLogic::
    // update runs) executes *before* nodes.fixed_update every frame, so if
    // apply_physics_feedback moved here but this stayed there, it would read
    // last frame's instrumentation instead of this frame's.
    fn fixed_update(&mut self, node: &mut Node, _cameras: &mut SceneCameras, app: &mut App, delta_time: f32, physics_message: Option<&RenderMessage>) {
        let Some(physics_message) = physics_message else { return };

        let scale = node.get_property::<Transform3D>().map(|transform| transform.scale).unwrap_or(Vector3::new(1.0, 1.0, 1.0));
        let Some(model_ref) = node.get_property::<ModelProperty>().map(|model| model.model_ref.clone()) else { return };
        let Some(model_instance) = app.game_models.get_mut(&model_ref) else { return };

        self.apply_physics_feedback(&mut model_instance.model, physics_message, scale, &app.renderer.queue, delta_time);

        if app.show_aero_debug_overlay {
            let speed_ms = physics_message.linvel.magnitude();
            let aoa_y = self.instrumentation.flight_data.aoa_y;
            let speed_in_knots = speed_ms * 1.94384; // 1 m/s = 1.94384 knots
            app.aero_debug_trail.push_back((speed_in_knots, aoa_y, self.instrumentation.flight_data.roll_rate));
            if app.aero_debug_trail.len() > Self::AERO_DEBUG_TRAIL_CAPACITY {
                app.aero_debug_trail.pop_front();
            }

            // See Instrumentation::wing_lift_forces' own doc comment.
            let main_wing_lift_y = self.instrumentation.wing_lift_forces.get("Left wing").map(|f| f.y).unwrap_or(0.0);
            let elevator_wing_lift_y = self.instrumentation.wing_lift_forces.get("Right elevator wing").map(|f| f.y).unwrap_or(0.0);
            let next_index = app.wing_lift_trail.back().map(|(i, _, _)| i + 1.0).unwrap_or(0.0);
            app.wing_lift_trail.push_back((next_index, main_wing_lift_y, elevator_wing_lift_y));
            if app.wing_lift_trail.len() > Self::AERO_DEBUG_TRAIL_CAPACITY {
                app.wing_lift_trail.pop_front();
            }
        }
    }
}
