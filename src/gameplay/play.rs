use std::{f64::consts::PI, time::{Duration, Instant}};

use cgmath::{Deg, ElementWise, InnerSpace, Point3, Quaternion, Rad, Rotation, Rotation3, Vector2, Vector3};
use glyphon::Color;
use rand::{rngs::ThreadRng, Rng};
use sdl2::controller::GameController;
use crate::{app::{self, App, AppState, Instance}, primitive::rectangle::RectPos, transform::Transform, ui::button::{self, Button, ButtonConfig}, utils::lerps::{lerp, lerp_quaternion, lerp_vector3}};

use super::controller::Controller;

pub enum CameraState {
    Normal,
    Front,
    Cockpit
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
    pub bandit_index: usize
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
    bandits: Vec<Bandit>
}

pub struct GameLogic { // here we define the data we use on our script
    fps: u32,
    last_frame: Instant,
    frame_count: u32,
    frame_timer: Duration,
    pub start_time: Instant,
    pub controller: Controller,
    pub velocity: Vector3<f32>,
    pub max_speed: f32,
    rotation: Vector3<f32>,
    pub camera_data: CameraData,
    pub altitude: AltitudeUi,
    pub plane_systems: PlaneSystems,
    rng: ThreadRng
} 

impl GameLogic {
    // this is called once
    pub fn new(app: &mut App) -> Self {
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
            &mut app.ui.text.font_system,
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
            &mut app.ui.text.font_system,
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
            &mut app.ui.text.font_system,
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
            &mut app.ui.text.font_system,
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
            &mut app.ui.text.font_system,
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
            &mut app.ui.text.font_system,
        );

        let crosshair = button::Button::new(
            button::ButtonConfig {
                rect_pos: RectPos { top: 10, left:10, bottom: 10, right:10 },
                fill_color: [0.0, 0.0, 0.0, 0.0],
                fill_color_active: [0.0, 0.0, 0.0, 0.0],
                border_color: [0.0, 1.0, 0.0, 1.0],
                border_color_active: [0.0, 1.0, 0.0, 1.0],
                text: "",
                text_color: Color::rgba(0, 255, 0, 255),
                text_color_active: Color::rgba(0, 0, 75, 000),
            },
            &mut app.ui.text.font_system,
        );

        app.components.clear();
        app.components.insert("altitude".to_owned(),altitude);
        app.components.insert("speed".to_owned(),speed);
        app.components.insert("compass".to_owned(),compass);
        app.components.insert("altitude_alert".to_owned(),altitude_alert);
        app.components.insert("framerate".to_owned(),framerate);
        app.components.insert("throttle".to_owned(),throttle);
        
        // app.components.push(crosshair);
        app.dynamic_ui_components.get_mut("dynamic_static").unwrap().clear();
        app.dynamic_ui_components.get_mut("dynamic_static").unwrap().push(crosshair);

        let velocity = Vector3::new(0.0, 0.0, 1300.0);
        let rotation = Vector3::new(0.0, 0.0, 0.0);

        let camera_data = CameraData { 
            camera_state: CameraState::Normal, 
            target: Point3::new(0.0, 0.0, 0.0), 
            position: Point3::new(0.0, 0.0, 0.0), 
            mod_yaw: 0.0, 
            mod_pitch: 0.0, 
            mod_pos_x: 0.0,
            mod_pos_y: 0.0,
            base_position: Vector3::new(0.0, 13.0, -35.0), 
            look_at: None,
            next_look_at: None,
            bandit_index: 0
        };

        let tower = Bandit {
            tag: "tower".to_owned(),
            locked: true,
        };

        let tower2 = Bandit {
            tag: "tower2".to_owned(),
            locked: false,
        };

        let crane = Bandit {
            tag: "crane".to_owned(),
            locked: false,
        };

        let plane_systems = PlaneSystems {
            bandits: vec![tower, tower2, crane],
        };

        let rng = rand::thread_rng();

        Self {
            fps: 0,
            last_frame: Instant::now(),
            start_time: Instant::now(),
            frame_count: 0,
            frame_timer: Duration::new(0, 0),
            controller: Controller::new(0.3, 0.2),
            velocity,
            rotation,
            max_speed: 2485.0,
            camera_data,
            altitude: AltitudeUi { altitude: 0.0, alert_state: false, time_alert: 0.0 },
            plane_systems,
            rng
        }
    }

    // this is called every frame
    pub fn update(&mut self, mut app_state: &mut AppState, mut event_pump: &mut sdl2::EventPump, app: &mut App, controller: &mut Option<GameController>) {
        let delta_time_duration = self.delta_time();
        let delta_time = delta_time_duration.as_secs_f32();
        self.display_framerate(delta_time_duration, app);
        self.plane_movement(app, delta_time, controller);
        self.camera_control(app, delta_time);
        self.ui_control(app, delta_time);
        self.controller.update(&mut app_state, &mut event_pump, app, controller, delta_time);
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
        app.components.get_mut("framerate").unwrap().text.set_text(&mut app.ui.text.font_system, &fps_text, true);
    }

    fn plane_movement (&mut self, app: &mut App, delta_time: f32, controller: &mut Option<GameController>) {
        let plane = app.renderizable_instances.get_mut("f14").unwrap();
        let mut angle = 0.8;

        if self.velocity.z < 1000.0 {
            angle = 0.0;
        }
        
        // elevators
        let l_elevator = plane.model.meshes.get_mut("left_elevator").unwrap();
        let l_elevator_rotation = lerp_quaternion(l_elevator.transform.rotation, Quaternion::from_angle_x(Rad(0.15 * (-self.controller.y + -self.controller.x))), delta_time * 7.0);
        let l_elevator_transform = Transform::new(l_elevator.transform.position, l_elevator_rotation, l_elevator.transform.scale);
        l_elevator.change_transform(&app.queue, l_elevator_transform);

        let r_elevator = plane.model.meshes.get_mut("right_elevator").unwrap();
        let r_elevator_rotation = lerp_quaternion(r_elevator.transform.rotation, Quaternion::from_angle_x(Rad(0.15 * (-self.controller.y + self.controller.x))), delta_time * 7.0);
        let r_elevator_transform = Transform::new(r_elevator.transform.position, r_elevator_rotation, r_elevator.transform.scale);
        r_elevator.change_transform(&app.queue, r_elevator_transform);

        // wings
        let l_wing = plane.model.meshes.get_mut("left_wing").unwrap();
        let l_wing_rotation = lerp_quaternion(l_wing.transform.rotation,Quaternion::from_angle_y(Rad(angle)), delta_time);
        let l_wing_transform = Transform::new(l_wing.transform.position, l_wing_rotation, l_wing.transform.scale);
        l_wing.change_transform(&app.queue, l_wing_transform);

        let r_wing = plane.model.meshes.get_mut("right_wing").unwrap();
        let r_wing_rotation = lerp_quaternion(r_wing.transform.rotation,Quaternion::from_angle_y(Rad(-angle)), delta_time);
        let r_wing_transform = Transform::new(r_wing.transform.position, r_wing_rotation, r_wing.transform.scale);
        r_wing.change_transform(&app.queue, r_wing_transform);

        // rudders
        let l_rudder = plane.model.meshes.get_mut("left_rudder").unwrap();
        let l_rudder_rotation = lerp_quaternion(l_rudder.transform.rotation, Quaternion::from_angle_x(Deg(-28.4493)) * Quaternion::from_angle_y(Rad(0.5 * self.controller.yaw)), delta_time * 7.0);
        let l_rudder_transform = Transform::new(l_rudder.transform.position, l_rudder_rotation, l_rudder.transform.scale);
        l_rudder.change_transform(&app.queue, l_rudder_transform);

        let r_rudder = plane.model.meshes.get_mut("right_rudder").unwrap();
        let r_rudder_rotation = lerp_quaternion(r_rudder.transform.rotation, Quaternion::from_angle_x(Deg(-28.4493)) * Quaternion::from_angle_y(Rad(0.5 * self.controller.yaw)), delta_time * 7.0);
        let r_rudder_transform = Transform::new(r_rudder.transform.position, r_rudder_rotation, r_rudder.transform.scale);
        r_rudder.change_transform(&app.queue, r_rudder_transform);

        let random_x: f32 = self.rng.gen_range(-3.0..=3.0);
        let random_y: f32 = self.rng.gen_range(-3.0..=3.0);
        if self.controller.power > 0.1 {
            self.camera_data.mod_pos_x = lerp(self.camera_data.mod_pos_x, random_x * 0.5, delta_time * 7.0);
            self.camera_data.mod_pos_y = lerp(self.camera_data.mod_pos_y, random_y * 0.5, delta_time * 7.0);
            
            match controller {
                Some(control) => {
                    if control.has_rumble() {
                        control.set_rumble(u16::MAX / 4, u16::MAX / 4, 100).unwrap();
                    }
                },
                None => {},
            }
            
            app.camera.projection.fovy = lerp(app.camera.projection.fovy, 70.0, delta_time * 7.0);
            if self.velocity.z < self.max_speed {
                self.velocity.z += 200.0 * delta_time;
            }
        } else if self.controller.power < -0.1 {
            self.camera_data.mod_pos_x = lerp(self.camera_data.mod_pos_x, random_x, delta_time * 7.0);
            self.camera_data.mod_pos_y = lerp(self.camera_data.mod_pos_y, random_y, delta_time * 7.0);
            
            match controller {
                Some(control) => {
                    if control.has_rumble() {
                        control.set_rumble(u16::MAX / 2, u16::MAX / 2, 100).unwrap();
                    }
                },
                None => {},
            }
            
            app.camera.projection.fovy = lerp(app.camera.projection.fovy, 45.0, delta_time);
            if self.velocity.z > 0.0 {
                self.velocity.z -= 400.0 * delta_time;
            }
        } else {
            app.camera.projection.fovy = lerp(app.camera.projection.fovy, 60.0, delta_time);
            if self.velocity.z > 0.0 {
                self.velocity.z -= self.calculate_deceleration(150.0) * delta_time;
            }
        };
        
        let x = if self.controller.y > 0.2 { 
            Self::calculate_turning(self.velocity.z, self.max_speed, 0.3, 0.8)
        } else if self.controller.y < -0.2 {
            Self::calculate_turning(self.velocity.z, self.max_speed, -0.7, -1.1)
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
        
        let final_rotation = plane.instance.rotation * (amount_x * amount_z * amount_y);
        let final_position = plane.instance.position;
        
        plane.instance.rotation = final_rotation;
        self.cockpit(app,  final_position, final_rotation);
    
    }

    fn cockpit(&mut self, app: &mut App, position: Vector3<f32>, rotation: Quaternion<f32>) {
        // app.renderizable_instances.get_mut("visor").unwrap().instance = Instance { position: position + (rotation * Vector3::new(0.0, 0.0, 0.0)), rotation: rotation, scale: Vector3::new(19.0, 19.0, 19.0) }
    }

    fn camera_control(&mut self, app: &mut App, delta_time: f32) {
        let plane = &mut app.renderizable_instances.get_mut("f14").unwrap().instance;
        
        match self.camera_data.camera_state {
            CameraState::Normal => {
                app.camera.projection.znear = 0.1;

                plane.scale = Vector3 {x: 19.0, y: 19.0, z: 19.0};


                let base_target_vector = Vector3::new(0.0, 0.0, 100.0);
                if self.controller.rx.abs() > self.controller.rs_deathzone || self.controller.ry.abs() > self.controller.rs_deathzone {
                    self.camera_data.base_position = lerp_vector3(self.camera_data.base_position, Vector3::new(0.0, 0.0, -60.0), delta_time * 5.0);

                    self.camera_data.mod_yaw = lerp(self.camera_data.mod_yaw, -self.controller.rx * std::f32::consts::PI, delta_time * 10.0);
                    self.camera_data.mod_pitch = lerp(self.camera_data.mod_pitch, -self.controller.ry * (std::f32::consts::PI / 2.1), delta_time * 10.0);
                } else {
                    self.camera_data.base_position = Vector3::new(0.0, 13.0, -40.0);
                    self.camera_data.mod_yaw = lerp(self.camera_data.mod_yaw, 0.0, delta_time * 10.0);
                    self.camera_data.mod_pitch = lerp(self.camera_data.mod_pitch, 0.0, delta_time * 10.0);
                }

                let rotation_mod = Quaternion::from_axis_angle(Vector3::unit_y(), Rad(self.camera_data.mod_yaw)) * Quaternion::from_axis_angle(Vector3::unit_x(), Rad(self.camera_data.mod_pitch));
                self.camera_data.position = Point3::new(plane.position.x, plane.position.y, plane.position.z) + (plane.rotation * rotation_mod * self.camera_data.base_position);
                self.camera_data.target = Point3::new(plane.position.x, plane.position.y, plane.position.z) + (plane.rotation * rotation_mod * base_target_vector);
                self.camera_data.position.x = self.camera_data.position.x + self.camera_data.mod_pos_x;
                self.camera_data.position.y = self.camera_data.position.y + self.camera_data.mod_pos_y;
                app.camera.camera.position = self.camera_data.position;
                app.camera.camera.look_at(self.camera_data.target);
            },
            CameraState::Front => {
                plane.scale = Vector3 {x: 19.0, y: 19.0, z: 19.0};

                self.camera_data.position = Point3::new(plane.position.x, plane.position.y, plane.position.z) + (plane.rotation * Vector3::new(0.0, 15.0, 0.0));
                self.camera_data.target = Point3::new(plane.position.x, plane.position.y, plane.position.z) + (plane.rotation * Vector3::new(0.0, 0.0, 100.0));

                app.camera.camera.position = self.camera_data.position;
                let rotation_view = plane.rotation * Vector3::new(-self.controller.rx, self.controller.ry * 10.0, 0.0) * 30.0;
                let edited = self.camera_data.target + rotation_view;
                app.camera.camera.look_at((edited.x, edited.y, edited.z).into());
            },
            CameraState::Cockpit => {

                app.camera.camera.position = self.camera_data.position;
                let rotation_view = plane.rotation * Vector3::new(-self.controller.rx, self.controller.ry * 10.0, 0.0) * 30.0;
                let edited = self.camera_data.target + rotation_view;
                app.camera.camera.look_at((edited.x, edited.y, edited.z).into());

                // plane.scale = Vector3 {x: 0.0, y: 0.0, z: 0.0};


                let base_target_vector = Vector3::new(0.0, 0.0, 100.0);
                if self.controller.rx.abs() > self.controller.rs_deathzone || self.controller.ry.abs() > self.controller.rs_deathzone {
                    self.camera_data.mod_yaw = lerp(self.camera_data.mod_yaw, -self.controller.rx * std::f32::consts::PI, delta_time * 7.0);
                    self.camera_data.mod_pitch = lerp(self.camera_data.mod_pitch, -self.controller.ry * (std::f32::consts::PI / 2.3), delta_time * 7.0);
                } else {
                    self.camera_data.mod_yaw = lerp(self.camera_data.mod_yaw, 0.0, delta_time * 10.0);
                    self.camera_data.mod_pitch = lerp(self.camera_data.mod_pitch, 0.0, delta_time * 10.0);
                }

                let rotation_mod = Quaternion::from_axis_angle(Vector3::unit_y(), Rad(self.camera_data.mod_yaw)) * Quaternion::from_axis_angle(Vector3::unit_x(), Rad(self.camera_data.mod_pitch));
                self.camera_data.target = Point3::new(plane.position.x, plane.position.y, plane.position.z) + (plane.rotation * rotation_mod * base_target_vector);
                let x_val = if self.controller.rx.abs() > self.controller.rs_deathzone { self.controller.rx * -0.7 } else { 0.0 };
                app.camera.camera.position = Point3::new(plane.position.x, plane.position.y, plane.position.z) + (plane.rotation * Vector3::new(x_val, 3.2, 25.0));
                app.camera.camera.look_at(self.camera_data.target);
            },
        }
        
        self.calculate_lockable(app);
        if self.controller.change_camera.up {
            self.next_camera();
        }
    }

    fn calculate_lockable(&mut self, app: &mut App) {
        let plane = &app.renderizable_instances.get("f14").unwrap().instance;

        for lockable in &self.plane_systems.bandits {
            if lockable.locked && self.controller.fix_view.pressed && self.controller.fix_view.time_pressed > self.controller.fix_view_hold_window {
                match app.renderizable_instances.get(&lockable.tag) {
                    Some(look_at) => {
                        app.camera.camera.look_at(Point3::new(look_at.instance.position.x, look_at.instance.position.y, look_at.instance.position.z));

                        match self.camera_data.camera_state {
                            CameraState::Normal => {
                                let pos = plane.position + Quaternion::between_vectors(Vector3::unit_z(), (look_at.instance.position - plane.position).normalize()) * (Vector3::new(0.0, 0.0, -60.0));
                                let final_pos = pos + (plane.rotation * Vector3::new(0.0, 20.0, 0.0));
                                app.camera.camera.position = (final_pos.x, final_pos.y, final_pos.z).into();
                            },
                            CameraState::Front => {
                                let pos = plane.position + Quaternion::between_vectors(Vector3::unit_z(), (look_at.instance.position - plane.position).normalize()) * (Vector3::new(0.0, 0.0, -60.0));
                                let final_pos = pos + (plane.rotation * Vector3::new(0.0, 20.0, 0.0));
                                app.camera.camera.position = (final_pos.x, final_pos.y, final_pos.z).into();
                            },
                            CameraState::Cockpit => {},
                        }
                    },
                    None => {},
                }
            }
        }
    }

    fn ui_control(&mut self, app: &mut App, delta_time: f32) {
        if app.throttling.last_ui_update.elapsed() >= app.throttling.ui_update_interval {
            app.components.get_mut("altitude").unwrap().text.set_text(&mut app.ui.text.font_system, &format!("ALT: {}", self.altitude.altitude), true);
            app.components.get_mut("speed").unwrap().text.set_text(&mut app.ui.text.font_system, &format!("SPEED: {}", self.velocity.z.round()), true);

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

            app.components.get_mut("compass").unwrap().text.set_text(&mut app.ui.text.font_system, &format!("{}", text_compass).to_string(), true);
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
            app.components.get_mut("altitude_alert").unwrap().rectangle.border_color = [1.0, 0.0, 0.0, 1.0];
            app.components.get_mut("altitude_alert").unwrap().text.color = Color::rgba(255, 0, 0, 255);
        } else {
            app.components.get_mut("altitude_alert").unwrap().rectangle.border_color = [0.0, 0.0, 0.0, 0.0];
            app.components.get_mut("altitude_alert").unwrap().text.color = Color::rgba(0, 0, 0, 0);
        }

        app.components.get_mut("throttle").unwrap().rectangle.position.top = (app.config.height / 2) - ((app.config.height as f32 / 2.0 * self.controller.power) - 100.0) as u32;
        app.components.get_mut("throttle").unwrap().rectangle.position.bottom = (app.config.height / 2) + ((app.config.height as f32 / 2.0 * -self.controller.power) - 100.0) as u32;
        self.targeting_system(app);
    }

    fn targeting_system(&mut self, app: &mut App) {
        // we set the position of all the bandits
        let mut bandits_target = vec![];
        
        for markable in &self.plane_systems.bandits {
            match app.renderizable_instances.get(&markable.tag) {
                Some(bandit) => bandits_target.push(bandit.instance.position),
                None => continue,
            }
        }

        // make the buttons for each bandit ONLY when the length of the ui components and the length of the bandits its the same
        if self.plane_systems.bandits.len() != app.dynamic_ui_components.get("bandits").unwrap().len() {
            app.dynamic_ui_components.get_mut("bandits").unwrap().clear();

            let _ = &bandits_target.iter().for_each(|_target| {
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
                    &mut app.ui.text.font_system,
                );
                app.dynamic_ui_components.get_mut("bandits").unwrap().push(crosshair);
            });
        }

        /*
        // changing the locked target 
        let mut diff_vectors: f32 = 1000.0;
        if (Vector2::new((app.config.width / 2) as f32, (app.config.height / 2) as f32) - (Vector2::new(lock_pos.x as f32, lock_pos.y as f32))).magnitude() < diff_vectors {
            diff_vectors = (Vector2::new((app.config.width / 2) as f32, (app.config.height / 2) as f32) - Vector2::new(lock_pos.x as f32, lock_pos.y as f32)).magnitude();
            self.camera_data.next_look_at = Some(*bandit_position);
        }
        */

        let mut diff_magnitude = f32::MAX;
        let mut closest_index: usize = 0;
        // for each position of the bandit we set a screen position and move the button
        for (i, bandit_position) in bandits_target.iter().enumerate() {
            let target_position = app.camera.world_to_screen(Point3::new(bandit_position.x, bandit_position.y, bandit_position.z), app.config.width, app.config.height);

            match target_position {
                Some(lock_pos) => {
                    if !self.plane_systems.bandits[i].locked {
                        let screen_middle = Vector2::new((app.config.width / 2) as f32, (app.config.height / 2) as f32);
                        let target_pos = Vector2::new(lock_pos.x as f32, lock_pos.y as f32);
                        if (screen_middle - target_pos).magnitude() < diff_magnitude {
                            closest_index = i;
                            diff_magnitude = (screen_middle - target_pos).magnitude();
                            self.camera_data.next_look_at = Some(*bandit_position);
                        }
                        // search for the closest element to the middle of the screen
                    }

                    app.dynamic_ui_components.get_mut("bandits").unwrap()[i].rectangle.border_color = if self.plane_systems.bandits[i].locked { [0.0, 0.0, 1.0, 1.0] } else { [0.0, 1.0, 0.0, 1.0] };
                    app.dynamic_ui_components.get_mut("bandits").unwrap()[i].rectangle.position.left = (lock_pos.x() - 20) as u32;
                    app.dynamic_ui_components.get_mut("bandits").unwrap()[i].rectangle.position.right = (lock_pos.x() + 20)  as u32;
                    app.dynamic_ui_components.get_mut("bandits").unwrap()[i].rectangle.position.top = (lock_pos.y() - 20) as u32;
                    app.dynamic_ui_components.get_mut("bandits").unwrap()[i].rectangle.position.bottom = (lock_pos.y() + 20) as u32;
                },
                None => {
                    app.dynamic_ui_components.get_mut("bandits").unwrap()[i].rectangle.border_color = [0.0, 1.0, 0.0, 0.0];
                },
            }
        }
        
        if self.controller.fix_view.up && self.controller.fix_view.time_pressed < self.controller.fix_view_hold_window {
            for bandit in &mut self.plane_systems.bandits {
                if bandit.locked {
                    bandit.locked = false;
                } else {
                    continue;
                }
            }
            self.plane_systems.bandits[closest_index].locked = true;
            self.camera_data.look_at = self.camera_data.next_look_at;
        }

        let plane_direction = app.renderizable_instances.get_mut("f14").unwrap().instance.rotation * Vector3::new(0.0, 0.0, 1000000.0);
        let plane_direction = app.camera.world_to_screen((plane_direction.x, plane_direction.y, plane_direction.z).into() , app.config.width, app.config.height);
        match plane_direction {
            Some(lock_pos) => {
                app.dynamic_ui_components.get_mut("dynamic_static").unwrap()[0].rectangle.border_color = [0.0, 1.0, 0.0, 1.0];
                app.dynamic_ui_components.get_mut("dynamic_static").unwrap()[0].rectangle.position.left = (lock_pos.x() as f32 - 1.0) as u32;
                app.dynamic_ui_components.get_mut("dynamic_static").unwrap()[0].rectangle.position.right = (lock_pos.x() as f32 + 1.0)  as u32;
                app.dynamic_ui_components.get_mut("dynamic_static").unwrap()[0].rectangle.position.top = (lock_pos.y() as f32 - 1.0) as u32;
                app.dynamic_ui_components.get_mut("dynamic_static").unwrap()[0].rectangle.position.bottom = (lock_pos.y() as f32 + 1.0) as u32;
            },
            None => {
                app.dynamic_ui_components.get_mut("dynamic_static").unwrap()[0].rectangle.border_color = [0.0, 1.0, 0.0, 0.0];
            },
        }
    }

    fn calculate_turning(speed: f32, speed_max: f32, turning_min: f32, turning_max: f32) -> f32 {
        turning_min + (turning_max - turning_min) * (1.0 - (speed / speed_max))
    }

    fn map_to_range(x: f64, in_min: f64, in_max: f64, out_min: f64, out_max: f64) -> f64 {
        (x - in_min) * (out_max - out_min) / (in_max - in_min) + out_min
    }

    fn calculate_deceleration(&self, base_deceleration: f32) -> f32 {
        let speed_ratio = self.velocity.z / self.max_speed;
        base_deceleration * speed_ratio * speed_ratio
    }

    fn next_camera(&mut self) {
        match self.camera_data.camera_state {
            CameraState::Normal => self.camera_data.camera_state = CameraState::Front,
            CameraState::Front => self.camera_data.camera_state = CameraState::Cockpit,
            CameraState::Cockpit => self.camera_data.camera_state = CameraState::Normal,
        }
    }
}