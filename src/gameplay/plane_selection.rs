use std::{collections::HashMap, f64::consts::PI, hash::Hash, time::{Duration, Instant}};

use cgmath::{num_traits::{real::Real, Float}, Deg, Euler, InnerSpace, One, Point3, Quaternion, Rad, Rotation, Rotation3, Vector2, Vector3, Zero};
use glyphon::{cosmic_text::rustybuzz::ttf_parser::ankr::Point, Color};
use image::imageops::flip_horizontal;
use rand::{rngs::ThreadRng, Rng};
use sdl2::controller::GameController;
use tokio::task;
use crate::{app::{App, AppState}, primitive::rectangle::RectPos, rendering::instance_management::ModelDataInstance, resources, transform::Transform, ui::button::{self, Button, ButtonConfig}, utils::lerps::{lerp, lerp_euler, lerp_quaternion, lerp_vector3}};
use serde::{Deserialize, Serialize};
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
    frame_count: u32,
    frame_timer: Duration,
    pub controller: Controller,
    pub camera_data: CameraData,
    pub plane_list: ListOfPlanes
} 

#[derive(Debug, Deserialize)]

struct PlaneModelData {
    name: String,
    model: String,
    description: String,
}

impl GameLogic {
    // this is called once
    pub fn new(app: &mut App) -> Self {
        // UI ELEMENTS AND LIST

        // planes to show:
        /* 
            1. pass the json values to a list readable
            2. set all the models and load them
            3. show the loaded one
        */
        

        let plane_name = button::Button::new(
            button::ButtonConfig {
                rect_pos: RectPos { top: app.config.height / 2 + 300, left: app.config.width / 2 - 100 , bottom: app.config.height / 2 + 415, right: app.config.width / 2 + 100 },
                fill_color: [0.0, 0.0, 0.0, 0.0],
                fill_color_active: [0.0, 0.0, 0.0, 0.0],
                border_color: [0.0, 1.0, 0.29411764705882354, 1.0],
                border_color_active: [0.0, 1.0, 0.29411764705882354, 1.0],
                text: "ALT:",
                text_color: Color::rgba(0, 255, 75, 255),
                text_color_active: Color::rgba(0, 255, 75, 000),
                rotation: Quaternion::zero()
            },
            &mut app.ui.text.font_system,
        );

        app.components.clear();
        app.components.insert("plane_name".to_owned(), plane_name);
        
        let camera_data = CameraData { 
            camera_state: CameraState::Normal, 
            target: Point3::new(0.0, 0.0, 0.0), 
            position: Point3::new(0.0, 0.0, 0.0), 
            mod_yaw: 0.0, 
            mod_pitch: 0.0, 
            mod_pos_x: 0.0,
            mod_pos_y: 0.0,
            base_position: Vector3::new(10.0, 0.0, 0.0), 
            look_at: None,
            next_look_at: None,
            mod_vector: Vector3::new(0.0, 0.0, 0.0),
            mod_up: Vector3::zero()
        };

        let plane_list = ListOfPlanes { list: vec!["f16".to_string(), "f14".to_string()], index: 0 };

        Self {
            fps: 0,
            last_frame: Instant::now(),
            frame_count: 0,
            frame_timer: Duration::new(0, 0),
            controller: Controller::new(0.3, 0.2),
            camera_data,
            plane_list
        }
    }

    // this is called every frame
    pub fn update(&mut self, mut app_state: &mut AppState, mut event_pump: &mut sdl2::EventPump, app: &mut App, controller: &mut Option<GameController>) {
        let delta_time_duration = self.delta_time();
        let delta_time = delta_time_duration.as_secs_f32();

        let rotation_speed_degrees_per_second = 40.0; // Rotate 90 degrees per second
        let rotation_increment = rotation_speed_degrees_per_second * delta_time;
        let y_rotation_quat = Quaternion::from_angle_y(Deg(rotation_increment));

        if self.controller.ui_left && self.plane_list.index > 0 {
            self.plane_list.index -= 1;
        }
        if self.controller.ui_right && self.plane_list.index < self.plane_list.list.len() - 1 {
            self.plane_list.index += 1;
        }

        if let Some(f16) = app.renderizable_instances.get_mut("f16") {
            if self.plane_list.list[self.plane_list.index] == "f16" {
                f16.instance.transform.scale = lerp_vector3(f16.instance.transform.scale, [14.0, 14.0, 14.0].into(), delta_time * 7.0);
            } else {
                f16.instance.transform.scale = lerp_vector3(f16.instance.transform.scale, [0.0, 0.0, 0.0].into(), delta_time * 7.0);
            }
            f16.instance.transform.rotation = y_rotation_quat * f16.instance.transform.rotation;
        };

        if let Some(f14) = app.renderizable_instances.get_mut("f14") {
            if self.plane_list.list[self.plane_list.index] == "f14" {
                f14.instance.transform.scale = lerp_vector3(f14.instance.transform.scale, [19.0, 19.0, 19.0].into(), delta_time * 7.0);
            } else {
                f14.instance.transform.scale = lerp_vector3(f14.instance.transform.scale, [0.0, 0.0, 0.0].into(), delta_time * 7.0);
            }
            f14.instance.transform.rotation = y_rotation_quat * f14.instance.transform.rotation;
        };

        self.camera_control(app, delta_time);
        self.controller.update(&mut app_state, &mut event_pump, app, controller, delta_time);
    }


    fn delta_time(&mut self) -> Duration {
        let current_time = Instant::now();
        let delta_time = current_time.duration_since(self.last_frame); // this is our Time.deltatime
        self.last_frame = current_time;
        return delta_time
    }
    

    fn camera_control(&mut self, app: &mut App, delta_time: f32) {
        // MAKE THE CAMERA ROTATE OR STAY ON THE SAME POSITION
        app.camera.camera.position = [0.0, 5.0, 50.0].into();
        app.camera.camera.look_at([0.0, 0.0, 0.0].into());
        println!("{}", app.camera.camera.position.x);



    }
}