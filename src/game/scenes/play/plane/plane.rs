use nalgebra::{UnitQuaternion, Vector3};

use crate::app::App;
use crate::engine::input::input;
use crate::engine::physics::physics_handler::{MetadataType, RenderMessage};
use crate::engine::rendering::camera::handler::SceneCameras;
use crate::engine::rendering::models::model::Model as LoadedModel;
use crate::engine::scene_manager::behavior::Behavior;
use crate::engine::scene_manager::node::Node;
use crate::engine::scene_manager::properties::Model as ModelProperty;
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

    pub fn apply_physics_feedback(&mut self, model: &mut LoadedModel, physics_message: &RenderMessage, gravity: Vector3<f32>, instance_scale: Vector3<f32>, queue: &wgpu::Queue, delta_time: f32) {
        self.instrumentation.flight_data.speedometer = physics_message.linvel.magnitude() * 1.94384;
        const SPEED_OF_SOUND_SEA_LEVEL_MS: f32 = 340.29;
        self.instrumentation.flight_data.mach = physics_message.linvel.magnitude() / SPEED_OF_SOUND_SEA_LEVEL_MS;
        self.instrumentation.flight_data.altimeter = physics_message.translation.y;
        let rotation = UnitQuaternion::from_quaternion(physics_message.rotation);
        let body_velocity = rotation.inverse() * physics_message.linvel;

        if body_velocity.magnitude() > 0.5 {
            self.instrumentation.flight_data.aoa_y = (-body_velocity.y).atan2(body_velocity.z).to_degrees();
            self.instrumentation.flight_data.aoa_x = body_velocity.x.atan2(body_velocity.z).to_degrees();
            self.instrumentation.flight_data.aoa = (body_velocity.z / body_velocity.magnitude()).clamp(-1.0, 1.0).acos().to_degrees();
        }

        let body_angvel = rotation.inverse() * physics_message.angvel;
        self.instrumentation.flight_data.roll_rate = body_angvel.z.to_degrees();
        self.instrumentation.flight_data.pitch_rate = body_angvel.x.to_degrees();
        self.instrumentation.flight_data.yaw_rate = body_angvel.y.to_degrees();

        self.instrumentation.velocity_sample_elapsed += delta_time;
        match &self.instrumentation.previous_velocity {
            Some(previous) if *previous != physics_message.linvel => {
                let acceleration = (physics_message.linvel - previous) / self.instrumentation.velocity_sample_elapsed;
                // Felt acceleration = total acceleration minus gravity (pilot doesn't feel gravity).
                let felt_acceleration = acceleration - gravity;
                let plane_up = UnitQuaternion::from_quaternion(physics_message.rotation) * Vector3::y_axis();
                let target_g = felt_acceleration.dot(&plane_up) / 9.81;
                self.instrumentation.flight_data.g_meter = lerp(self.instrumentation.flight_data.g_meter, target_g, delta_time * 10.0);
                self.instrumentation.previous_velocity = Some(physics_message.linvel);
                self.instrumentation.velocity_sample_elapsed = 0.0;
            }
            None => {
                self.instrumentation.previous_velocity = Some(physics_message.linvel);
                self.instrumentation.velocity_sample_elapsed = 0.0;
            }
            _ => {}
        }

        let wheel_data = match physics_message.metadata.get("wheels") {
            Some(MetadataType::Wheels(w)) => Some(w),
            _ => None,
        };

        self.airframe.landing_gear.place_wheel_meshes(model, wheel_data, instance_scale, queue);

        if let Some(MetadataType::Wings(wings)) = physics_message.metadata.get("wings") {
            if let Some(elevator_wing) = wings.iter().find(|w| w.label == "Right elevator wing") {
                self.controls.debug_simulated_elevator_control_input = Some(elevator_wing.control_input);
            }

            for wing in wings {
                self.instrumentation.wing_lift_forces.insert(wing.label.clone(), wing.last_lift_force);
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
}
