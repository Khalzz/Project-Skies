use std::{collections::HashMap, f64::consts::PI, hash::Hash, time::{Duration, Instant}};

use cgmath::{num_traits::{real::Real, Float}, point3, Deg, EuclideanSpace, Euler, InnerSpace, Matrix2, One, Point3, Quaternion, Rad, Rotation, Rotation3, Vector2, Vector3, Zero};
use glyphon::{cosmic_text::rustybuzz::ttf_parser::ankr::Point, Color};
use image::imageops::flip_horizontal;
use rand::{rngs::ThreadRng, Rng};
use sdl2::controller::GameController;
use tokio::task;
use crate::{app::{App, AppState}, primitive::rectangle::RectPos, rendering::instance_management::ModelDataInstance, resources, transform::Transform, ui::button::{self, Button, ButtonConfig}, utils::lerps::{lerp, lerp_euler, lerp_quaternion, lerp_vector3}};
use serde::{de, Deserialize, Serialize};
use serde_json;

use super::controller::Controller;

pub enum CameraState {
    Normal,
    Cockpit,
    Far
}

pub struct Bandit {
    tag: String,
    locked: bool,
}

pub struct CameraData {
    camera_state: CameraState,
    target: Point3<f32>,
    position: Point3<f32>,
    mod_yaw: f32,
    mod_pitch: f32,
    mod_pos_x: f32,
    mod_pos_y: f32,
    base_position: Vector3<f32>,
    pub look_at: Option<Vector3<f32>>,
    pub next_look_at: Option<Vector3<f32>>,
    pub mod_vector: Vector3<f32>,
    pub mod_up: Vector3<f32>
}

pub struct BlinkingAlert {
    alert_state: bool,
    time_alert: f32
}

pub struct PlaneSystems {
    bandits: Vec<Bandit>,
    stall: bool,
    pub altitude: f32,
}

pub struct ListOfPlanes {
    list: Vec<String>,
    index: usize
}

pub struct GameLogic { // here we define the data we use on our script
    fps: u32,
    last_frame: Instant,
    pub start_time: Instant,
    frame_count: u32,
    frame_timer: Duration,
    pub controller: Controller,
    pub plane_list: ListOfPlanes,
    pub controller_simulation: Vector2<f32>
} 

impl GameLogic {
    // this is called once
    pub fn new(app: &mut App) -> Self {
        let plane_name = button::Button::new(
            button::ButtonConfig {
                rect_pos: RectPos { top: app.config.height / 2 + 300, left: app.config.width / 2 - 100 , bottom: app.config.height / 2 + 415, right: app.config.width / 2 + 100 },
                fill_color: [0.0, 0.0, 0.0, 0.0],
                fill_color_active: [0.0, 0.0, 0.0, 0.0],
                border_color: [0.0, 0.0, 0.0, 0.0],
                border_color_active: [0.0, 0.0, 0.0, 0.0],
                text: "Plane Name",
                text_color: Color::rgba(0, 255, 75, 255),
                text_color_active: Color::rgba(0, 255, 75, 000),
                rotation: Quaternion::zero()
            },
            &mut app.ui.text.font_system,
        );

        app.components.clear();
        app.components.insert("plane_name".to_owned(), plane_name);

        app.camera.camera.position = [0.0, 7.0, 50.0].into();

        let plane_list = ListOfPlanes { list: vec!["f16".to_string(), "f14".to_string()], index: 0 };

        Self {
            fps: 0,
            last_frame: Instant::now(),
            start_time: Instant::now(),
            frame_count: 0,
            frame_timer: Duration::new(0, 0),
            controller: Controller::new(0.3, 0.2),
            plane_list,
            controller_simulation: Vector2::new(0.0, 1.0)
        }
    }

    // this is called every frame
    pub fn update(&mut self, mut app_state: &mut AppState, mut event_pump: &mut sdl2::EventPump, app: &mut App, controller: &mut Option<GameController>) {
        let delta_time_duration = self.delta_time();
        let delta_time = delta_time_duration.as_secs_f32();


        if let Some(plane) = app.renderizable_instances.get_mut(&self.plane_list.list[self.plane_list.index]) {
            if let Some(plane_model) = app.game_models.get_mut(&plane.instance.model) {
                if let Some(afterburner) = plane_model.model.meshes.get_mut("Afterburner") {
                    afterburner.change_transform(&app.queue, Transform::new(afterburner.transform.position, afterburner.transform.rotation, Vector3::new(0.0, 0.0, 0.0)));
                }
            }
        }

        // ui plane name
        if let Some(plane_name) = app.components.get_mut("plane_name") {
            plane_name.text.set_text(&mut app.ui.text.font_system, &self.plane_list.list[self.plane_list.index], true);
        }

        // display plane
        let angle = Rad(1.0 * delta_time * 3.0);
        let rotation_matrix = Matrix2::from_angle(angle);
        self.controller_simulation = rotation_matrix * self.controller_simulation;


        for plane in &self.plane_list.list {
            if let Some(plane) = app.renderizable_instances.get_mut(plane) {
                if let Some(plane_model) = app.game_models.get_mut(&plane.model_ref) {
                    if let Some(aleron) = plane_model.model.meshes.get_mut("left_aleron") {
                        let dependent = aleron.base_transform.rotation.clone() * Quaternion::from_angle_x(Rad(0.5 * -self.controller_simulation.x));
                        let aleron_rotation = lerp_quaternion(aleron.transform.rotation,  dependent, delta_time * 7.0);
                        let aleron_transform = Transform::new(aleron.transform.position, aleron_rotation, aleron.transform.scale);
                        aleron.change_transform(&app.queue, aleron_transform);
                    }

                    if let Some(aleron) = plane_model.model.meshes.get_mut("right_aleron") {
                        let dependent = aleron.base_transform.rotation.clone() * Quaternion::from_angle_x(Rad(0.5 * self.controller_simulation.x));
                        let aleron_rotation = lerp_quaternion(aleron.transform.rotation,  dependent, delta_time * 7.0);
                        let aleron_transform = Transform::new(aleron.transform.position, aleron_rotation, aleron.transform.scale);
                        aleron.change_transform(&app.queue, aleron_transform);
                    }

                    if let Some(elevator) = plane_model.model.meshes.get_mut("left_elevator") {
                        let elevator_rotation = lerp_quaternion(elevator.transform.rotation, Quaternion::from_angle_x(Rad(0.2 * -self.controller_simulation.y)), delta_time * 7.0);
                        let elevator_transform = Transform::new(elevator.transform.position, elevator_rotation, elevator.transform.scale);
                        elevator.change_transform(&app.queue, elevator_transform);
                    }

                    if let Some(elevator) = plane_model.model.meshes.get_mut("right_elevator") {
                        let elevator_rotation = lerp_quaternion(elevator.transform.rotation, Quaternion::from_angle_x(Rad(0.2 * -self.controller_simulation.y)), delta_time * 7.0);
                        let elevator_transform = Transform::new(elevator.transform.position, elevator_rotation, elevator.transform.scale);
                        elevator.change_transform(&app.queue, elevator_transform);
                    }
                }

                let scale: Vector3<f32> = if self.plane_list.list[self.plane_list.index] == plane.instance.id {
                    plane.renderizable_transform.scale
                } else {
                    [0.0, 0.0, 0.0].into()
                };

                plane.instance.transform.scale = lerp_vector3(plane.instance.transform.scale, scale, delta_time * 7.0);
            }
        }

        self.camera_control(app, delta_time);
        self.controller.update(&mut app_state, &mut event_pump, app, controller, delta_time);
    }

    fn get_oscillating_value(&self, speed: f32) -> f32 {
        let elapsed_time = self.start_time.elapsed().as_secs_f32();
        (elapsed_time * speed * PI as f32).sin()
    }

    fn delta_time(&mut self) -> Duration {
        let current_time = Instant::now();
        let delta_time = current_time.duration_since(self.last_frame); // this is our Time.deltatime
        self.last_frame = current_time;
        return delta_time
    }

    fn camera_control(&mut self, app: &mut App, delta_time: f32) {
        let new_position = Self::rotate_camera_position(app.camera.camera.position.to_vec(), Vector3::zero(), 40.0, Vector3::new(0.0, 1.0, 0.0), delta_time);

        app.camera.camera.position = point3(new_position.x, new_position.y, new_position.z);
        app.camera.camera.look_at([0.0, 0.0, 0.0].into());

        if self.controller.ui_left && self.plane_list.index > 0 {
            self.plane_list.index -= 1;
        }

        if self.controller.ui_right && self.plane_list.index < self.plane_list.list.len() - 1 {
            self.plane_list.index += 1;
        }
    }

    fn rotate_camera_position(base_position: Vector3<f32>, pivot: Vector3<f32>, rotation_speed: f32, rotation_axis: Vector3<f32>, delta_time: f32) -> Vector3<f32> {
        let angle = Deg(rotation_speed * delta_time);        
        let rotation = Quaternion::from_axis_angle(rotation_axis, angle);
        let relative_position = base_position - pivot;
        let rotated_position = rotation.rotate_vector(relative_position);

        rotated_position + pivot
    }
}