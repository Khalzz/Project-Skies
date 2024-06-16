use std::{f64::consts::PI, time::{Duration, Instant}};

use cgmath::{InnerSpace, Point3, Quaternion, Rad, Rotation, Rotation3, Vector3};
use glyphon::Color;
use sdl2::controller::GameController;
use tokio::io::ReadBuf;
use crate::{app::{App, AppState, InstanceData}, primitive::rectangle::RectPos, ui::button::{self, Button, ButtonConfig}, utils::lerps::{lerp, lerp_quaternion, lerp_vector3}};

use super::controller::Controller;

pub enum CameraState {
    Normal,
    Front,
}

pub struct CameraData {
    camera_state: CameraState,
    target: Point3<f32>,
    position: Point3<f32>,
    mod_yaw: f32,
    mod_pitch: f32,
    base_position: Vector3<f32>,
    pub look_at: Option<Vector3<f32>>
}

pub struct AltitudeUi {
    pub altitude: f32,
    alert_state: bool,
    time_alert: f32
}

pub struct Ui {
    altitude: Button
}

pub struct PlaneSystems {
    bandits: Vec<Vector3<f32>>
}

pub struct GameLogic { // here we define the data we use on our script
    fps: u32,
    last_frame: Instant,
    frame_count: u32,
    frame_timer: Duration,
    pub start_time: Instant,
    pub controller: Controller,
    pub velocity: Vector3<f32>,
    rotation: Vector3<f32>,
    pub camera_data: CameraData,
    pub altitude: AltitudeUi,
    pub plane_systems: PlaneSystems
} 

impl GameLogic {
    // this is called once
    pub fn new(app: &mut App, speed: f64) -> Self {
        // UI ELEMENTS AND LIST
        let altitude = button::Button::new(
            button::ButtonConfig {
                rect_pos: RectPos { top: 10, left: 10, bottom: 50, right: 200 },
                fill_color: [0.0, 0.0, 0.0, 0.0],
                fill_color_active: [0.0, 0.0, 0.0, 0.0],
                border_color: [0.0, 1.0, 0.29411764705882354, 1.0],
                border_color_active: [0.0, 1.0, 0.29411764705882354, 1.0],
                text: "ALT:",
                text_color: Color::rgba(0, 255, 75, 255),
                text_color_active: Color::rgba(0, 255, 75, 000),
            },
            &mut app.text.font_system,
        );

        let speed = button::Button::new(
            button::ButtonConfig {
                rect_pos: RectPos { top: 60, left: 10, bottom: 100, right: 200 },
                fill_color: [0.0, 0.0, 0.0, 0.0],
                fill_color_active: [0.0, 0.0, 0.0, 0.0],
                border_color: [0.0, 1.0, 0.29411764705882354, 1.0],
                border_color_active: [0.0, 1.0, 0.29411764705882354, 1.0],
                text: "SPEED:",
                text_color: Color::rgba(0, 255, 75, 255),
                text_color_active: Color::rgba(0, 255, 75, 000),
            },
            &mut app.text.font_system,
        );

        let altitude_alert = button::Button::new(
            button::ButtonConfig {
                rect_pos: RectPos { top: app.config.height / 2 + 100, left: app.config.width / 2 - 70, bottom: app.config.height / 2 + 140, right: app.config.width / 2 + 70 },
                fill_color: [0.0, 0.0, 0.0, 0.0],
                fill_color_active: [0.0, 0.0, 0.0, 0.0],
                border_color: [1.0, 0.0, 0.0, 1.0],
                border_color_active: [1.0, 0.0, 0.0, 1.0],
                text: "Altitude",
                text_color: Color::rgba(255, 0, 0, 255),
                text_color_active: Color::rgba(0, 0, 75, 000),
            },
            &mut app.text.font_system,
        );

        let compass: Button = button::Button::new(
            button::ButtonConfig {
                rect_pos: RectPos { top: 10, left: app.config.width / 2 - 50, bottom: 50, right: app.config.width / 2 + 50 },
                fill_color: [0.0, 0.0, 0.0, 0.0],
                fill_color_active: [0.0, 0.0, 0.0, 0.0],
                border_color: [0.0, 1.0, 0.0, 1.0],
                border_color_active: [0.0, 1.0, 0.0, 1.0],
                text: "90°",
                text_color: Color::rgba(0, 255, 0, 255),
                text_color_active: Color::rgba(0, 0, 75, 000),
            },
            &mut app.text.font_system,
        );

        let framerate: Button = button::Button::new(
            button::ButtonConfig {
                rect_pos: RectPos { top: 10, left: app.config.width - 110, bottom: 50, right: app.config.width - 10 },
                fill_color: [0.0, 0.0, 0.0, 0.0],
                fill_color_active: [0.0, 0.0, 0.0, 0.0],
                border_color: [0.0, 1.0, 0.0, 1.0],
                border_color_active: [0.0, 1.0, 0.0, 1.0],
                text: "90 fps",
                text_color: Color::rgba(0, 255, 0, 255),
                text_color_active: Color::rgba(0, 0, 75, 000),
            },
            &mut app.text.font_system,
        );

        let throttle: Button = button::Button::new(
            button::ButtonConfig {
                rect_pos: RectPos { top: 10, left: app.config.width - 15, bottom: app.config.height - 10, right: app.config.width - 10 },
                fill_color: [0.0, 1.0, 0.0, 1.0],
                fill_color_active: [0.0, 0.0, 0.0, 0.0],
                border_color: [0.0, 1.0, 0.0, 1.0],
                border_color_active: [0.0, 1.0, 0.0, 1.0],
                text: "",
                text_color: Color::rgba(0, 255, 0, 255),
                text_color_active: Color::rgba(0, 0, 75, 000),
            },
            &mut app.text.font_system,
        );

        app.components = vec![];
        app.components.push(altitude);
        app.components.push(speed);
        app.components.push(compass);
        app.components.push(altitude_alert);
        app.components.push(framerate);
        app.components.push(throttle);

        let velocity = Vector3::new(0.0, 0.0, 500.0);
        let rotation = Vector3::new(0.0, 0.0, 0.0);

        let camera_data = CameraData { 
            camera_state: CameraState::Normal, 
            target: Point3::new(0.0, 0.0, 0.0), 
            position: Point3::new(0.0, 0.0, 0.0), 
            mod_yaw: 0.0, 
            mod_pitch: 0.0, 
            base_position: Vector3::new(0.0, 13.0, -35.0), 
            look_at: None 
        };

        let plane_systems = PlaneSystems {
            bandits: vec![app.renderizable_instances.get("tower").unwrap().instance.position, app.renderizable_instances.get("crane").unwrap().instance.position, app.renderizable_instances.get("tower2").unwrap().instance.position],
        };

        Self {
            fps: 0,
            last_frame: Instant::now(),
            start_time: Instant::now(),
            frame_count: 0,
            frame_timer: Duration::new(0, 0),
            controller: Controller::new(0.3, 0.1),
            velocity,
            rotation,
            camera_data,
            altitude: AltitudeUi { altitude: 0.0, alert_state: false, time_alert: 0.0 },
            plane_systems
        }
    }

    // this is called every frame
    pub fn update(&mut self, mut app_state: &mut AppState, mut event_pump: &mut sdl2::EventPump, app: &mut App, controller: &Option<GameController>) {
        let delta_time_duration = self.delta_time();
        let delta_time = delta_time_duration.as_secs_f32();
        self.display_framerate(delta_time_duration, app);
        self.plane_movement(app, delta_time);
        self.camera_control(app, delta_time);
        self.ui_control(app, delta_time);
        Self::event_handler(self, &mut app_state, &mut event_pump, app, controller);
    }

    fn event_handler(&mut self, app_state: &mut AppState, event_pump: &mut sdl2::EventPump, app: &mut App, controller: &Option<GameController>) {
        if app.throttling.last_controller_update.elapsed() >= app.throttling.controller_update_interval {
            for event in event_pump.poll_iter() {
                self.controller.update(event, app, app_state, controller)
            }
            app.throttling.last_controller_update = Instant::now();
        }
    }

    fn delta_time(&mut self) -> Duration {
        let current_time = Instant::now();
        let delta_time = current_time.duration_since(self.last_frame); // this is our Time.deltatime
        self.last_frame = current_time;
        return delta_time
    }

    fn display_framerate(&mut self, delta_time: Duration, app: &mut App) {
        self.frame_count += 1;
        self.frame_timer += delta_time;

        if self.frame_timer >= Duration::from_secs(1) {
            self.fps = self.frame_count;
            self.frame_count = 0;
            self.frame_timer -= Duration::from_secs(1); // Remove one second from the timer
        }

        let fps_text = format!("FPS: {}", self.fps);
        app.components[4].text.set_text(&mut app.text.font_system, &fps_text);
    }

    fn map_to_range(x: f64, in_min: f64, in_max: f64, out_min: f64, out_max: f64) -> f64 {
        (x - in_min) * (out_max - out_min) / (in_max - in_min) + out_min
    }

    fn plane_movement (&mut self, app: &mut App, delta_time: f32) {
        let plane = app.renderizable_instances.get_mut("f14").unwrap();

        // plane.model.meshes[1].transform.rotation = lerp_quaternion(plane.model.meshes[1].transform.rotation,Quaternion::from_angle_x(Rad(0.2) * self.controller.x), delta_time * 5.0);
        let mut angle = 0.0;
        let mut high_g_mode = false;

        if self.velocity.z < 1800.0 {
            high_g_mode = true;
            angle = 0.5
        }
        plane.model.meshes[3].transform.rotation = lerp_quaternion(plane.model.meshes[3].transform.rotation,Quaternion::from_angle_y(Rad(-angle)), delta_time);
        plane.model.meshes[2].transform.rotation = lerp_quaternion(plane.model.meshes[2].transform.rotation,Quaternion::from_angle_y(Rad(angle)), delta_time);
        plane.model.meshes[3].update_transform(&app.queue);
        plane.model.meshes[2].update_transform(&app.queue);


        let throttle = if self.controller.power > 0.1 {
            app.camera.projection.fovy = lerp(app.camera.projection.fovy, 70.0, delta_time);
            if self.velocity.z < 2485.0 {
                self.velocity.z += 200.0 * delta_time;
            }
            1.0 
        } else if self.controller.power < -0.1 {
            app.camera.projection.fovy = lerp(app.camera.projection.fovy, 45.0, delta_time);
            if self.velocity.z > 0.0 {
                self.velocity.z -= 400.0 * delta_time;
            }
            -1.0 
        } else {
            app.camera.projection.fovy = lerp(app.camera.projection.fovy, 60.0, delta_time);
            if self.velocity.z > 0.0 {
                self.velocity.z -= 2.0 * delta_time;
            }
            0.0
        };

        let x = if self.controller.y > 0.2 { 
            if high_g_mode {
                0.8
            } else {
                0.4
            }
        } else if self.controller.y < -0.2 {
            if high_g_mode {
                -1.0
            } else {
                -0.7
            }
        } else { 0.0 };

        let y = 0.5 * -self.controller.yaw;
        let z = 5.5 * self.controller.x;

        if self.controller.x < 0.2 && self.controller.x > -0.2 {
            self.rotation.z = lerp(self.rotation.z,z, delta_time * 5.0);
        } else {
            self.rotation.z = lerp(self.rotation.z,z, delta_time * 2.0);
        }
        self.rotation.x = lerp(self.rotation.x, x, delta_time * 3.0);
        self.rotation.y = lerp(self.rotation.y, y, delta_time);

        let amount_x = cgmath::Quaternion::from_angle_x(cgmath::Rad(self.rotation.x) * delta_time);
        let amount_y = cgmath::Quaternion::from_angle_y(cgmath::Rad(self.rotation.y) * delta_time);
        let amount_z = cgmath::Quaternion::from_angle_z(cgmath::Rad(self.rotation.z) * delta_time);
        
        app.camera.camera.position.y = self.controller.ry;
        plane.instance.rotation = plane.instance.rotation * (amount_x * amount_z * amount_y);
    }

    fn camera_control(&mut self, app: &mut App, delta_time: f32) {
        let plane = &app.renderizable_instances.get_mut("f14").unwrap().instance;

        match self.camera_data.camera_state {
            CameraState::Normal => {
                let base_target_vector = Vector3::new(0.0, 0.0, 100.0);
                if self.controller.rx.abs() > self.controller.rs_deathzone || self.controller.ry.abs() > self.controller.rs_deathzone {
                    self.camera_data.base_position = lerp_vector3(self.camera_data.base_position, Vector3::new(0.0, 0.0, -60.0), delta_time * 5.0);
                    self.camera_data.mod_yaw = -self.controller.rx * std::f32::consts::PI;
                    self.camera_data.mod_pitch = -self.controller.ry * (std::f32::consts::PI / 2.1); // Limit pitch to -90 to 90 degrees
                } else {
                    self.camera_data.base_position = Vector3::new(0.0, 13.0, -40.0);
                    self.camera_data.mod_yaw = 0.0;
                    self.camera_data.mod_pitch = 0.0;
                }

                let rotation_mod = Quaternion::from_axis_angle(Vector3::unit_y(), Rad(self.camera_data.mod_yaw)) * Quaternion::from_axis_angle(Vector3::unit_x(), Rad(self.camera_data.mod_pitch));
                self.camera_data.position = Point3::new(plane.position.x, plane.position.y, plane.position.z) + (plane.rotation * rotation_mod * self.camera_data.base_position);
                self.camera_data.target = Point3::new(plane.position.x, plane.position.y, plane.position.z) + (plane.rotation * rotation_mod * base_target_vector);
                app.camera.camera.position = self.camera_data.position;
                app.camera.camera.look_at(self.camera_data.target);
            },
            CameraState::Front => {
                self.camera_data.position = Point3::new(plane.position.x, plane.position.y, plane.position.z) + (plane.rotation * Vector3::new(0.0, 1.0, 0.0));
                self.camera_data.target = Point3::new(plane.position.x, plane.position.y, plane.position.z) + (plane.rotation * Vector3::new(0.0, 0.0, 10.0));

                app.camera.camera.position = self.camera_data.position;
                let rotation_view = plane.rotation * Vector3::new(-self.controller.rx, self.controller.ry * 10.0, 0.0) * 30.0;
                let edited = self.camera_data.target + rotation_view;
                app.camera.camera.look_at((edited.x, edited.y, edited.z).into());
            },
        }

        match self.camera_data.look_at {
            Some(look_at) => {
                if self.controller.fix_view {
                    app.camera.camera.look_at(Point3::new(look_at.x, look_at.y, look_at.z));
                    let pos = plane.position + Quaternion::between_vectors(Vector3::unit_z(), (look_at - plane.position).normalize()) * (Vector3::new(0.0, 0.0, -60.0));
                    let final_pos = pos + (plane.rotation * Vector3::new(0.0, 20.0, 0.0));
                    app.camera.camera.position = (final_pos.x, final_pos.y, final_pos.z).into();
                }
            },
            None => {},
        }
    }

    fn ui_control(&mut self, app: &mut App, delta_time: f32) {
        if app.throttling.last_ui_update.elapsed() >= app.throttling.ui_update_interval {
            app.components[0].text.set_text(&mut app.text.font_system, &format!("ALT: {}", self.altitude.altitude));
            app.components[1].text.set_text(&mut app.text.font_system, &format!("SPEED: {}", self.velocity.z.round()));

            let rotation = Self::map_to_range(app.camera.camera.yaw.0.into(), -PI, PI, 0.0, 360.0).round();
            let text_compass = if rotation >= 355.0 || rotation <= 5.0 {
                "N".to_owned()
            } else if rotation >= 175.0 && rotation <= 185.0{
                "S".to_owned()
            } else if rotation >= 85.0 && rotation <= 95.0 {
                "E".to_owned()
            } else if rotation >= 265.0 && rotation <= 275.0 {
                "O".to_owned()
            } else {
                rotation.round().to_string() + "°"
            };

            app.components[2].text.set_text(&mut app.text.font_system, &format!("{}", text_compass).to_string());
            app.throttling.last_ui_update = Instant::now();
        }

        self.altitude.time_alert += delta_time;
        if self.altitude.altitude < 1000.0 {
            if self.altitude.alert_state == false {
                if self.altitude.time_alert > 0.5 {
                    self.altitude.time_alert = 0.0;
                    self.altitude.alert_state = true;
                }
            } else {
                if self.altitude.time_alert > 0.5 {
                    self.altitude.time_alert = 0.0;
                    self.altitude.alert_state = false;
                }
            }
        } else {
            self.altitude.time_alert = 0.0;
            self.altitude.alert_state = false
        }

        if self.altitude.alert_state {
            app.components[3].rectangle.border_color = [1.0, 0.0, 0.0, 1.0];
            app.components[3].text.color = Color::rgba(255, 0, 0, 255);
        } else {
            app.components[3].rectangle.border_color = [0.0, 0.0, 0.0, 0.0];
            app.components[3].text.color = Color::rgba(0, 0, 0, 0);
        }

        app.components[5].rectangle.position.top = (app.config.height / 2) - ((app.config.height as f32 / 2.0 * self.controller.power) - 100.0) as u32;
        app.components[5].rectangle.position.bottom = (app.config.height / 2) + ((app.config.height as f32 / 2.0 * -self.controller.power) - 100.0) as u32;

        self.targeting_system(app);
    }

    fn targeting_system(&mut self, app: &mut App) {
        self.plane_systems.bandits = vec![app.renderizable_instances.get("tower").unwrap().instance.position, app.renderizable_instances.get("crane").unwrap().instance.position, app.renderizable_instances.get("tower2").unwrap().instance.position];

        if self.plane_systems.bandits.len() != app.dynamic_ui_components.len() {
            app.dynamic_ui_components = vec![];

            for _ in self.plane_systems.bandits.iter() {
                let crosshair: Button = Button::new(
                    ButtonConfig {
                        rect_pos: RectPos { top: 50, left: 50, bottom: 50, right: 50 },
                        fill_color: [0.0, 0.0, 0.0, 0.0],
                        fill_color_active: [0.0, 0.0, 0.0, 0.0],
                        border_color: [0.0, 1.0, 0.0, 1.0],
                        border_color_active: [0.0, 1.0, 0.0, 1.0],
                        text: "",
                        text_color: Color::rgba(0, 255, 0, 255),
                        text_color_active: Color::rgba(0, 0, 75, 000),
                    },
                    &mut app.text.font_system,
                );
                app.dynamic_ui_components.push(crosshair);
            }
        }

        for (i, bandit) in self.plane_systems.bandits.iter().enumerate() {
            let lock_position = app.camera.world_to_screen(Point3::new(bandit.x, bandit.y, bandit.z), app.config.width, app.config.height);
            match lock_position {
                Some(lock_pos) => {
                    app.dynamic_ui_components[i].rectangle.border_color = [0.0, 1.0, 0.0, 1.0];
                    app.dynamic_ui_components[i].rectangle.position.left = (lock_pos.x() - 20) as u32;
                    app.dynamic_ui_components[i].rectangle.position.right = (lock_pos.x() + 20)  as u32;
                    app.dynamic_ui_components[i].rectangle.position.top = (lock_pos.y() - 20) as u32;
                    app.dynamic_ui_components[i].rectangle.position.bottom = (lock_pos.y() + 20) as u32;
                },
                None => {
                    app.dynamic_ui_components[i].rectangle.border_color = [0.0, 1.0, 0.0, 0.0];
                },
            }
        }
    }
}