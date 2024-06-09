use std::time::{Duration, Instant};

use cgmath::{InnerSpace, Point3, Rotation3, Vector3};
use sdl2::{event::Event, keyboard::Keycode, pixels::Color, ttf::Font};
use wgpu::BindGroupLayoutDescriptor;
use crate::{app::{App, AppState}, game_object::GameObject, input::button_module::{Button, TextAlign}, rendering::textures::Texture};

pub struct Velocity {
    x: f32,
    y: f32,
    z: f32
}
pub struct Controller {
    pub forward: bool,
    pub backwards: bool,
    pub left: bool,
    pub right: bool,
    pub rotate_left: bool,
    pub rotate_right: bool,
    pub throttle: bool,
    pub brake: bool,
}

pub struct GameLogic { // here we define the data we use on our script
    fps: u32,
    fps_text: Button,
    last_frame: Instant,
    pub start_time: Instant,
    frame_count: u32,
    frame_timer: Duration,
    pub controller: Controller,
    velocity: Velocity,
    rotation: Velocity
} 

impl GameLogic {
    // this is called once
    pub fn new(_app: &mut App, speed: f64) -> Self {
        // UI ELEMENTS AND LIST
        let framerate = Button::new(GameObject {active: true, x:10 as f32, y: 10.0, width: 0.0, height: 0.0},Some(String::from("Framerate")),Color::RGBA(100, 100, 100, 0),Color::WHITE,Color::RGB(0, 200, 0),Color::RGB(0, 0, 0),None, TextAlign::Left);
        let velocity = Velocity { x: 0.0, y: 0.0, z: 500.0 };
        let rotation = Velocity { x: 0.0, y: 0.0, z: 0.0 };
        Self {
            fps: 0,
            fps_text: framerate,
            last_frame: Instant::now(),
            start_time: Instant::now(),
            frame_count: 0,
            frame_timer: Duration::new(0, 0),
            controller: Controller { forward: false, backwards: false, left: false, right: false, rotate_left: false, rotate_right: false, throttle: false, brake: false },
            velocity,
            rotation
        }
    }

    // this is called every frame
    pub fn update(&mut self, _font: &Font, mut app_state: &mut AppState, mut event_pump: &mut sdl2::EventPump, app: &mut App) {
        let delta_time = self.delta_time().as_secs_f32();
        self.display_framerate(delta_time);

        let current = app.instances[0].rotation;

        let throttle = if self.controller.throttle {
            app.camera.projection.fovy = Self::lerp(app.camera.projection.fovy, 70.0, delta_time);
            if self.velocity.z < 2485.0 {
                self.velocity.z += 200.0 * delta_time;
            }
            1.0 
        } else if self.controller.brake {
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

        let x = if self.controller.forward { 
            if throttle == -1.0 {
                1.5
            } else {
                0.8
            }
        } else if self.controller.backwards {
            if throttle == -1.0 {
                -3.0
            } else {
                -2.0
            }
        } else { 0.0 };

        let y = if self.controller.rotate_left { 0.5 } else if self.controller.rotate_right { -0.5 } else { 0.0 };
        let z = if self.controller.left { -7.0 } else if self.controller.right { 7.0 } else { 0.0 };

        if self.controller.left || self.controller.right {
            self.rotation.z = Self::lerp(self.rotation.z,z, delta_time * 2.0);
        } else {
            self.rotation.z = Self::lerp(self.rotation.z,z, delta_time * 5.0);
        }
        self.rotation.x = Self::lerp(self.rotation.x, x, delta_time * 3.0);
        self.rotation.y = Self::lerp(self.rotation.y, y, delta_time);

        let amount_x = cgmath::Quaternion::from_angle_x(cgmath::Rad(self.rotation.x) * delta_time);
        let amount_y = cgmath::Quaternion::from_angle_y(cgmath::Rad(self.rotation.y) * delta_time);
        let amount_z = cgmath::Quaternion::from_angle_z(cgmath::Rad(self.rotation.z) * delta_time);
        
        app.instances[0].rotation = current * (amount_x * amount_z * amount_y);
        app.instances[0].position += current * Vector3::new(self.velocity.x, self.velocity.y, self.velocity.z) * delta_time;

        Self::event_handler(self, &mut app_state, &mut event_pump, app, delta_time);
    }

    fn event_handler(&mut self, app_state: &mut AppState, event_pump: &mut sdl2::EventPump, app: &mut App, delta_time: f32) {
        for event in event_pump.poll_iter() {
            match event {
                Event::KeyDown { keycode: Some(Keycode::Space), .. } => {
                    app.show_depth_map = !app.show_depth_map
                }
                Event::KeyDown { keycode: Some(Keycode::Up), .. } => {
                    self.controller.throttle = true
                }
                Event::KeyUp { keycode: Some(Keycode::Up), .. } => {
                    self.controller.throttle = false
                }
                Event::KeyDown { keycode: Some(Keycode::Down), .. } => {
                    self.controller.brake = true
                }
                Event::KeyUp { keycode: Some(Keycode::Down), .. } => {
                    self.controller.brake = false
                }
                Event::KeyDown { keycode: Some(Keycode::Q), .. } => {
                    self.controller.rotate_left = true
                }
                Event::KeyUp { keycode: Some(Keycode::Q), .. } => {
                    self.controller.rotate_left = false
                }
                Event::KeyDown { keycode: Some(Keycode::E), .. } => {
                    self.controller.rotate_right = true
                }
                Event::KeyUp { keycode: Some(Keycode::E), .. } => {
                    self.controller.rotate_right = false
                }
                Event::KeyDown { keycode: Some(Keycode::W), .. } => {
                    self.controller.forward = true
                }
                Event::KeyUp { keycode: Some(Keycode::W), .. } => {
                    self.controller.forward = false
                }
                Event::KeyDown { keycode: Some(Keycode::A), .. } => {
                    self.controller.left = true
                }
                Event::KeyUp { keycode: Some(Keycode::A), .. } => {
                    self.controller.left = false
                }
                Event::KeyDown { keycode: Some(Keycode::S), .. } => {
                    self.controller.backwards = true
                }
                Event::KeyUp { keycode: Some(Keycode::S), .. } => {
                    self.controller.backwards = false
                }
                Event::KeyDown { keycode: Some(Keycode::D), .. } => {
                    self.controller.right = true
                }
                Event::KeyUp { keycode: Some(Keycode::D), .. } => {
                    self.controller.right = false
                }
                Event::KeyDown { keycode: Some(Keycode::Escape), .. }  => {
                    app_state.is_running = false;
                }, Event::Quit { .. } => {
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