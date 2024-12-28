use nalgebra::{ AbstractRotation, Point3, Quaternion, Rotation2, Unit, UnitQuaternion, Vector2, Vector3};
use std::{collections::HashMap, f64::consts::PI, time::{Duration, Instant}};
use sdl2::controller::GameController;
use glyphon::{cosmic_text::Align, Color};

use crate::{app::{App, AppState}, rendering::ui::UiContainer, transform::Transform, ui::{button, ui_node::{UiNode, UiNodeContent, UiNodeParameters, Visibility}, ui_transform::UiTransform}, utils::lerps::{lerp_quaternion, lerp_vector3}};

use super::controller::Controller;

pub struct ListOfPlanes {
    list: Vec<String>,
    index: usize
}

pub struct GameLogic { // here we define the data we use on our script
    pub controller: Controller,
    pub plane_list: ListOfPlanes,
    pub controller_simulation: Vector2<f32>
} 

impl GameLogic {
    // this is called once
    pub fn new(app: &mut App) -> Self {

        let plane_name = UiNode::new(
            UiTransform::new(0.0, 0.0, 50.0, 100.0, 0.0, false), 
            Visibility::new([0.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.29, 1.0]),
            UiNodeParameters::Text { text: "Plane Name", color: Color::rgba(0, 255, 75, 255), align: Align::Center, font_size: 20.0 }, 
            app,
        );

        app.ui.renderizable_elements.clear();
        app.ui.renderizable_elements.insert("static".to_owned(), UiContainer::Tagged(HashMap::new()));

        match app.ui.renderizable_elements.get_mut("static").unwrap() {
            UiContainer::Tagged(hash_map) => {
                hash_map.insert("plane_name".to_owned(), plane_name);
            },
            _ => {},
        }

        app.camera.camera.position = [0.0, 7.0, 50.0].into();

        let plane_list = ListOfPlanes { list: vec!["f16".to_string(), "f14".to_string()], index: 0 };

        Self {
            controller: Controller::new(0.3, 0.2),
            plane_list,
            controller_simulation: Vector2::new(0.0, 1.0)
        }
    }

    // this is called every frame
    pub fn update(&mut self, mut app_state: &mut AppState, mut event_pump: &mut sdl2::EventPump, app: &mut App, controller: &mut Option<GameController>) {
        if let Some(plane) = app.renderizable_instances.get_mut(&self.plane_list.list[self.plane_list.index]) {
            if let Some(plane_model) = app.game_models.get_mut(&plane.instance.model) {
                if let Some(meshes) = plane_model.model.mesh_lists.get_mut("transparent") {
                    if let Some(afterburner) = meshes.get_mut("Afterburner") {
                        afterburner.change_transform(&app.queue, Transform::new(afterburner.transform.position, afterburner.transform.rotation, Vector3::new(0.0, 0.0, 0.0)));
                    }
                }
            }
        }

        // ui plane name
        match app.ui.renderizable_elements.get_mut("static").unwrap() {
            UiContainer::Tagged(hash_map) => {
                if let Some(plane_name) = hash_map.get_mut("plane_name") {
                    match &mut plane_name.content {
                        UiNodeContent::Text(label) => {
                            label.set_text(&mut app.ui.text.font_system, &self.plane_list.list[self.plane_list.index], true);
                        },
                        _ => {}
                    }
                }
            },
            _=> {},
        }

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
                            aleron.change_transform(&app.queue, aleron_transform);
                        }
    
                        if let Some(aleron) = meshes.get_mut("right_aleron") {
                            let dependent = aleron.base_transform.rotation.clone() * *UnitQuaternion::from_axis_angle(&Vector3::x_axis() ,0.5 * self.controller_simulation.x);
                            let aleron_rotation = lerp_quaternion(aleron.transform.rotation,  dependent, app.time.delta_time * 7.0);
                            let aleron_transform = Transform::new(aleron.transform.position, aleron_rotation, aleron.transform.scale);
                            aleron.change_transform(&app.queue, aleron_transform);
                        }
    
                        if let Some(elevator) = meshes.get_mut("left_elevator") {
                            let elevator_rotation = lerp_quaternion(elevator.transform.rotation, *UnitQuaternion::from_axis_angle(&Vector3::x_axis() ,0.2 * -self.controller_simulation.y), app.time.delta_time * 7.0);
                            let elevator_transform = Transform::new(elevator.transform.position, elevator_rotation, elevator.transform.scale);
                            elevator.change_transform(&app.queue, elevator_transform);
                        }
    
                        if let Some(elevator) = meshes.get_mut("right_elevator") {
                            let elevator_rotation = lerp_quaternion(elevator.transform.rotation, *UnitQuaternion::from_axis_angle(&Vector3::x_axis() ,0.2 * -self.controller_simulation.y), app.time.delta_time * 7.0);
                            let elevator_transform = Transform::new(elevator.transform.position, elevator_rotation, elevator.transform.scale);
                            elevator.change_transform(&app.queue, elevator_transform);
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
        self.controller.update(&mut app_state, &mut event_pump, app, controller, app.time.delta_time);
    }

    fn camera_control(&mut self, app: &mut App, delta_time: f32) {
        let new_position = Self::rotate_camera_position(app.camera.camera.position.coords, Vector3::zeros(), 40.0, Vector3::new(0.0, 1.0, 0.0), delta_time);

        app.camera.camera.position = Point3::new(new_position.x, new_position.y, new_position.z);
        app.camera.camera.look_at([0.0, 0.0, 0.0].into());

        if self.controller.ui_left && self.plane_list.index > 0 {
            self.plane_list.index -= 1;
        }

        if self.controller.ui_right && self.plane_list.index < self.plane_list.list.len() - 1 {
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