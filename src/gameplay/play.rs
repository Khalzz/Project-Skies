use std::time::{Duration, Instant};

use cgmath::{num_traits::Signed, InnerSpace, Point3, Rotation3, Vector3};
use sdl2::{controller::{Axis, GameController}, event::Event, keyboard::Keycode, pixels::Color, ttf::Font};
use wgpu::BindGroupLayoutDescriptor;
use crate::{app::{App, AppState}, game_object::GameObject, input::button_module::{Button, TextAlign}, rendering::textures::Texture};

pub struct Controller {
    pub yaw: f32, // rotate on the y axis
    pub throttle: bool,
    pub brake: bool,
    pub x: f32, // rotate on the z axis
    pub y: f32, // rotate on the x axis
    pub ls_deathzone: f32,
    pub rx: f32,
    pub ry: f32,
    pub rs_deathzone:f32,
    pub power: f32,
    pub fix_view: bool,
    pub look_back: bool,

}

pub struct GameLogic { // here we define the data we use on our script
    fps: u32,
    fps_text: Button,
    last_frame: Instant,
    pub start_time: Instant,
    frame_count: u32,
    frame_timer: Duration,
    pub controller: Controller,
    pub velocity: Vector3<f32>,
    rotation: Vector3<f32>,

} 

impl GameLogic {
    // this is called once
    pub fn new(_app: &mut App, speed: f64) -> Self {
        // UI ELEMENTS AND LIST
        let framerate = Button::new(GameObject {active: true, x:10 as f32, y: 10.0, width: 0.0, height: 0.0},Some(String::from("Framerate")),Color::RGBA(100, 100, 100, 0),Color::WHITE,Color::RGB(0, 200, 0),Color::RGB(0, 0, 0),None, TextAlign::Left);
        let velocity = Vector3::new(0.0, 0.0, 500.0);
        let rotation = Vector3::new(0.0, 0.0, 0.0);


        Self {
            fps: 0,
            fps_text: framerate,
            last_frame: Instant::now(),
            start_time: Instant::now(),
            frame_count: 0,
            frame_timer: Duration::new(0, 0),
            controller: Controller {throttle: false, brake: false, yaw: 0.0, x: 0.0, y: 0.0, ls_deathzone: 0.3, rx: 0.0, ry: 0.0, rs_deathzone: 0.1, power: 0.0, fix_view: false, look_back: false },
            velocity,
            rotation,
        }
    }

    // this is called every frame
    pub fn update(&mut self, mut app_state: &mut AppState, mut event_pump: &mut sdl2::EventPump, app: &mut App, controller: &Option<GameController>) {
        let delta_time = self.delta_time().as_secs_f32();
        self.display_framerate(delta_time);

        let current = app.instances[0].rotation;

        let throttle = if self.controller.power > 0.1 {
            app.camera.projection.fovy = Self::lerp(app.camera.projection.fovy, 70.0, delta_time);
            if self.velocity.z < 2485.0 / 5.0 {
                self.velocity.z += 200.0 * delta_time;
            }
            1.0 
        } else if self.controller.power < -0.1 {
            app.camera.projection.fovy = Self::lerp(app.camera.projection.fovy, 45.0, delta_time);
            if self.velocity.z > 0.0 {
                self.velocity.z -= 200.0 * delta_time;
            }
            -1.0 
        } else {
            app.camera.projection.fovy = Self::lerp(app.camera.projection.fovy, 60.0, delta_time);
            if self.velocity.z > 0.0 {
                self.velocity.z -= 2.0 * delta_time;
            }
            0.0 
        };

        let x = if self.controller.y > 0.2 { 
            if throttle == -1.0 {
                1.0
            } else {
                0.5
            }
        } else if self.controller.y < -0.2 {
            if throttle == -1.0 {
                -3.0
            } else {
                -1.5
            }
        } else { 0.0 };

        let y = 0.5 * -self.controller.yaw;
        let z = 5.5 * self.controller.x;

        if self.controller.x < 0.2 && self.controller.x > -0.2 {
            self.rotation.z = Self::lerp(self.rotation.z,z, delta_time * 5.0);
        } else {
            self.rotation.z = Self::lerp(self.rotation.z,z, delta_time * 2.0);
        }
        self.rotation.x = Self::lerp(self.rotation.x, x, delta_time * 3.0);
        self.rotation.y = Self::lerp(self.rotation.y, y, delta_time);

        let amount_x = cgmath::Quaternion::from_angle_x(cgmath::Rad(self.rotation.x) * delta_time);
        let amount_y = cgmath::Quaternion::from_angle_y(cgmath::Rad(self.rotation.y) * delta_time);
        let amount_z = cgmath::Quaternion::from_angle_z(cgmath::Rad(self.rotation.z) * delta_time);
        
        app.camera.camera.position.y = self.controller.ry;
        app.instances[0].rotation = current * (amount_x * amount_z * amount_y);
        // self.plane_position += current * Vector3::new(self.velocity.x, self.velocity.y, self.velocity.z) * delta_time;

        Self::event_handler(self, &mut app_state, &mut event_pump, app, delta_time, controller);
    }

    fn event_handler(&mut self, app_state: &mut AppState, event_pump: &mut sdl2::EventPump, app: &mut App, delta_time: f32, controller: &Option<GameController>) {
        for event in event_pump.poll_iter() {
            match event {
                Event::ControllerButtonDown { button, .. } => {
                    match button {
                        sdl2::controller::Button::Y => {
                            self.controller.fix_view = true
                        },
                        sdl2::controller::Button::Back => {
                            // change camera
                        },
                        sdl2::controller::Button::RightStick => {
                            self.controller.look_back = true
                        },
                        sdl2::controller::Button::LeftShoulder => {
                            self.controller.yaw = -1.0
                        },
                        sdl2::controller::Button::RightShoulder => {
                            self.controller.yaw = 1.0
                        },
                        _ => {}
                    }
                }
                Event::ControllerButtonUp { button, .. } => {
                    match button {
                        sdl2::controller::Button::Y => {
                            self.controller.fix_view = false
                        },
                        sdl2::controller::Button::Back => {
                            // change camera
                        },
                        sdl2::controller::Button::RightStick => {
                            self.controller.look_back = false
                        },
                        sdl2::controller::Button::LeftShoulder => {
                            self.controller.yaw = 0.0
                        },
                        sdl2::controller::Button::RightShoulder => {
                            self.controller.yaw = 0.0
                        },
                        _ => {}
                    }
                }
                Event::ControllerAxisMotion { axis, .. } => {
                    match axis {
                        Axis::LeftX | Axis::LeftY => {
                            let x = controller.as_ref().map_or(0, |c| c.axis(Axis::LeftX)) as f32 / 32767.0;
                            if x > self.controller.ls_deathzone || x < -self.controller.ls_deathzone {
                                self.controller.x = x;
                            } else {
                                self.controller.x = 0.0;
                            }
                            let y = controller.as_ref().map_or(0, |c| c.axis(Axis::LeftY)) as f32 / 32767.0;
                            if y > self.controller.ls_deathzone || y < -self.controller.ls_deathzone {
                                self.controller.y = -y;
                            } else {
                                self.controller.y = 0.0;
                            }
                        },
                        Axis::RightX | Axis::RightY => {
                            let x = controller.as_ref().map_or(0, |c| c.axis(Axis::RightX)) as f32 / 32767.0;
                            self.controller.rx = x;

                            let y = controller.as_ref().map_or(0, |c| c.axis(Axis::RightY)) as f32 / 32767.0;
                            self.controller.ry = -y;
                        },
                        Axis::TriggerLeft | Axis::TriggerRight => {
                            let left = -(controller.as_ref().map_or(0, |c| c.axis(Axis::TriggerLeft)) as f32 / 32767.0);
                            let right = controller.as_ref().map_or(0, |c| c.axis(Axis::TriggerRight)) as f32 / 32767.0;
                            self.controller.power = left + right;
                        },
                        _ => {}
                    }
                }
                Event::KeyDown { keycode, .. } => {
                    match keycode {
                        Some(Keycode::Escape) => app_state.is_running = false,
                        Some(Keycode::Space) => app.show_depth_map = !app.show_depth_map,
                        Some(Keycode::Down) => self.controller.power = -1.0,
                        Some(Keycode::Up) => self.controller.power = 1.0,
                        Some(Keycode::Q) => self.controller.yaw = -1.0,
                        Some(Keycode::E) => self.controller.yaw = 1.0,
                        Some(Keycode::A) => self.controller.x = -1.0,
                        Some(Keycode::D) => self.controller.x = 1.0,
                        Some(Keycode::S) => self.controller.y = -1.0,
                        Some(Keycode::W) => self.controller.y = 1.0,
                        _ => {},
                    }
                }
                Event::KeyUp { keycode, .. } => {
                    match keycode {
                        Some(Keycode::Down) => self.controller.power = 0.0,
                        Some(Keycode::Up) => self.controller.power = 0.0,
                        Some(Keycode::Q) => self.controller.yaw = 0.0,
                        Some(Keycode::E) => self.controller.yaw = 0.0,
                        Some(Keycode::A) => self.controller.x = 0.0,
                        Some(Keycode::D) => self.controller.x = 0.0,
                        Some(Keycode::S) => self.controller.y = 0.0,
                        Some(Keycode::W) => self.controller.y = 0.0,
                        _ => {},
                    }
                }
                Event::Quit { .. } => {
                    app_state.is_running = false;
                } 
                _ => {}
            }
        }
    }

    fn delta_time(&mut self) -> Duration {
        let current_time = Instant::now();
        let delta_time = current_time.duration_since(self.last_frame); // this is our Time.deltatime
        self.last_frame = current_time;
        return delta_time
    }

    fn display_framerate(&mut self, delta_time: f32) {
    }

    fn lerp(start: f32, end: f32, t: f32) -> f32 {
        start + (end - start) * t
    }

    
}