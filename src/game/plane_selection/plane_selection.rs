use nalgebra::{ AbstractRotation, Point3, Quaternion, Rotation2, Unit, UnitQuaternion, Vector2, Vector3};
use std::{collections::HashMap, f64::consts::PI, time::{Duration, Instant}};

use crate::{app::App, engine::rendering::enviroment::environment::Environment, resources::apply_environment, transform::Transform, engine::ui::{button, ui_node::{UiNode, UiNodeContent}}, engine::utils::lerps::{lerp_quaternion, lerp_vector3}};

use crate::engine::input::input;
use crate::engine::scene_manager::scene::{FrameContext, Scene};

pub struct ListOfPlanes {
    list: Vec<String>,
    index: usize
}

pub struct GameLogic { // here we define the data we use on our script
    pub plane_list: ListOfPlanes,
    pub controller_simulation: Vector2<f32>
}

impl GameLogic {
    // this is called once
    pub fn new(app: &mut App) -> Self {

        app.ui.renderizable_elements.clear();

        app.camera.active_mut().camera.position = [0.0, 7.0, 50.0].into();

        let plane_list = ListOfPlanes { list: vec!["f16".to_string(), "f14".to_string()], index: 0 };


        Self {
            plane_list,
            controller_simulation: Vector2::new(0.0, 1.0)
        }
    }

    // this is called every frame
    pub fn update(&mut self, app: &mut App) {
        if let Some(plane) = app.renderizable_instances.get_mut(&self.plane_list.list[self.plane_list.index]) {
            if let Some(plane_model) = app.game_models.get_mut(&plane.instance.model) {
                if let Some(meshes) = plane_model.model.mesh_lists.get_mut("transparent") {
                    if let Some(afterburner) = meshes.get_mut("Afterburner") {
                        afterburner.change_transform(&app.renderer.queue, Transform::new(afterburner.transform.position, afterburner.transform.rotation, Vector3::new(0.0, 0.0, 0.0)));
                    }
                }
            }
        }

        // ui plane name

        // display plane
        let angle = 1.0 * app.time.delta_time * 3.0;
        let binding = Rotation2::new(angle);
        let rotation_matrix = binding.matrix();
        self.controller_simulation = rotation_matrix * self.controller_simulation;

        for plane in &self.plane_list.list {
            if let Some(plane) = app.renderizable_instances.get_mut(plane) {
                if let Some(plane_model) = app.game_models.get_mut(&plane.model_ref) {
                    if let Some(meshes) = plane_model.model.mesh_lists.get_mut("opaque") {
                        if let Some(aleron) = meshes.get_mut("left_aleron") {

                            let dependent = aleron.base_transform.rotation.clone() * *UnitQuaternion::from_axis_angle(&Vector3::x_axis() ,0.5 * -self.controller_simulation.x);
                            let aleron_rotation = lerp_quaternion(aleron.transform.rotation,  dependent, app.time.delta_time * 7.0);
                            let aleron_transform = Transform::new(aleron.transform.position, aleron_rotation, aleron.transform.scale);
                            aleron.change_transform(&app.renderer.queue, aleron_transform);
                        }
    
                        if let Some(aleron) = meshes.get_mut("right_aleron") {
                            let dependent = aleron.base_transform.rotation.clone() * *UnitQuaternion::from_axis_angle(&Vector3::x_axis() ,0.5 * self.controller_simulation.x);
                            let aleron_rotation = lerp_quaternion(aleron.transform.rotation,  dependent, app.time.delta_time * 7.0);
                            let aleron_transform = Transform::new(aleron.transform.position, aleron_rotation, aleron.transform.scale);
                            aleron.change_transform(&app.renderer.queue, aleron_transform);
                        }
    
                        if let Some(elevator) = meshes.get_mut("left_elevator") {
                            let elevator_rotation = lerp_quaternion(elevator.transform.rotation, *UnitQuaternion::from_axis_angle(&Vector3::x_axis() ,0.2 * -self.controller_simulation.y), app.time.delta_time * 7.0);
                            let elevator_transform = Transform::new(elevator.transform.position, elevator_rotation, elevator.transform.scale);
                            elevator.change_transform(&app.renderer.queue, elevator_transform);
                        }
    
                        if let Some(elevator) = meshes.get_mut("right_elevator") {
                            let elevator_rotation = lerp_quaternion(elevator.transform.rotation, *UnitQuaternion::from_axis_angle(&Vector3::x_axis() ,0.2 * -self.controller_simulation.y), app.time.delta_time * 7.0);
                            let elevator_transform = Transform::new(elevator.transform.position, elevator_rotation, elevator.transform.scale);
                            elevator.change_transform(&app.renderer.queue, elevator_transform);
                        }
                    }
                }

                let scale: Vector3<f32> = if self.plane_list.list[self.plane_list.index] == plane.instance.id {
                    plane.renderizable_transform.scale
                } else {
                    [0.0, 0.0, 0.0].into()
                };

                plane.instance.transform.scale = lerp_vector3(plane.instance.transform.scale, scale, app.time.delta_time * 7.0);
            }
        }

        self.camera_control(app, app.time.delta_time);
    }

    fn camera_control(&mut self, app: &mut App, delta_time: f32) {
        let new_position = Self::rotate_camera_position(app.camera.active().camera.position.coords, Vector3::zeros(), 40.0, Vector3::new(0.0, 1.0, 0.0), delta_time);

        let active = app.camera.active_mut();
        active.camera.position = Point3::new(new_position.x, new_position.y, new_position.z);
        active.camera.look_at([0.0, 0.0, 0.0].into());

        if input::is_action_just_pressed("ui_left") && self.plane_list.index > 0 {
            self.plane_list.index -= 1;
        }

        if input::is_action_just_pressed("ui_right") && self.plane_list.index < self.plane_list.list.len() - 1 {
            self.plane_list.index += 1;
        }
    }

    fn rotate_camera_position(base_position: Vector3<f32>, pivot: Vector3<f32>, rotation_speed: f32, rotation_axis: Vector3<f32>, delta_time: f32) -> Vector3<f32> {
        let angle = rotation_speed * delta_time;
        let rotation = UnitQuaternion::from_axis_angle(&Unit::new_normalize(rotation_axis), angle);
        let relative_position = base_position - pivot;
        let rotated_position = rotation.transform_vector(&relative_position);
    
        rotated_position + pivot
    }
}

impl Scene for GameLogic {
    fn update(&mut self, app: &mut App, ctx: &mut FrameContext) {
        let _ = ctx;
        self.update(app);
    }
}