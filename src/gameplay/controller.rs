// this structure will define the buttons, the data type of each and what we will do with each one of them, 
// we will modify this every time we will add or delete a control

use std::time::Instant;

use sdl2::{controller::{self, Axis, GameController}, event::{Event, EventPollIterator}, keyboard::Keycode};

use crate::app::{self, App, AppState};

pub struct Input {
    pub pressed: bool,
    pub up: bool,
    pub time_pressed: f32,
}

pub struct Controller {
    pub yaw: f32, // rotate on the y axis
    pub throttle: f32,
    pub brake: f32,
    pub x: f32, // rotate on the z axis
    pub y: f32, // rotate on the x axis
    pub ls_deathzone: f32,
    pub rx: f32,
    pub ry: f32,
    pub rs_deathzone:f32,
    pub power: f32,
    pub fix_view: Input,
    pub fix_view_hold_window: f32,
    pub look_back: bool,
}

impl Controller {
    pub fn new(ls_deathzone: f32, rs_deathzone: f32) -> Self {
        Self {
            yaw: 0.0,
            throttle: 0.0,
            brake: 0.0,
            x: 0.0,
            y: 0.0,
            ls_deathzone,
            rx: 0.0,
            ry: 0.0,
            rs_deathzone,
            power: 0.0,
            fix_view: Input { pressed: false, up: false, time_pressed: 0.0 },
            fix_view_hold_window: 0.2,
            look_back: false,
        }
    }

    pub fn update(&mut self, app_state: &mut AppState, event_pump: &mut sdl2::EventPump, app: &mut App, controller: &Option<GameController>, delta_time: f32) {
        if self.fix_view.pressed {
            self.fix_view.time_pressed += delta_time
        } else {
            self.fix_view.up = false;
        }
        if app.throttling.last_controller_update.elapsed() >= app.throttling.controller_update_interval {
            for event in event_pump.poll_iter() {
                match event {
                    Event::ControllerButtonDown { button, .. } => {
                        match button {
                            sdl2::controller::Button::Y => {
                                self.fix_view.pressed = true;
                                self.fix_view.time_pressed = 0.0;
                            },
                            sdl2::controller::Button::Back => {
                                // change camera
                            },
                            sdl2::controller::Button::RightStick => {
                                self.look_back = true
                            },
                            sdl2::controller::Button::LeftShoulder => {
                                self.yaw = -1.0
                            },
                            sdl2::controller::Button::RightShoulder => {
                                self.yaw = 1.0
                            },
                            _ => {}
                        }
                    }
                    Event::ControllerButtonUp { button, .. } => {
                        match button {
                            sdl2::controller::Button::Y => {
                                self.fix_view.pressed = false;
                                self.fix_view.up = true;
                            },
                            sdl2::controller::Button::Back => {
                                // change camera
                            },
                            sdl2::controller::Button::RightStick => {
                                self.look_back = false
                            },
                            sdl2::controller::Button::LeftShoulder => {
                                self.yaw = 0.0
                            },
                            sdl2::controller::Button::RightShoulder => {
                                self.yaw = 0.0
                            },
                            _ => {}
                        }
                    }
                    Event::ControllerAxisMotion { axis, .. } => {
                        match axis {
                            Axis::LeftX | Axis::LeftY => {
                                let x = controller.as_ref().map_or(0, |c| c.axis(Axis::LeftX)) as f32 / 32767.0;
                                if x > self.ls_deathzone || x < -self.ls_deathzone {
                                    self.x = x;
                                } else {
                                    self.x = 0.0;
                                }
                                let y = controller.as_ref().map_or(0, |c| c.axis(Axis::LeftY)) as f32 / 32767.0;
                                if y > self.ls_deathzone || y < -self.ls_deathzone {
                                    self.y = -y;
                                } else {
                                    self.y = 0.0;
                                }
                            },
                            Axis::RightX | Axis::RightY => {
                                let x = controller.as_ref().map_or(0, |c| c.axis(Axis::RightX)) as f32 / 32767.0;
                                self.rx = x;
        
                                let y = controller.as_ref().map_or(0, |c| c.axis(Axis::RightY)) as f32 / 32767.0;
                                self.ry = -y;
                            },
                            Axis::TriggerLeft | Axis::TriggerRight => {
                                self.throttle = -controller.as_ref().map_or(0, |c| c.axis(Axis::TriggerLeft)) as f32 / 32767.0;
                                self.brake = controller.as_ref().map_or(0, |c| c.axis(Axis::TriggerRight)) as f32 / 32767.0;
                                self.power = self.brake + self.throttle;
                            },
                            _ => {}
                        }
                    }
                    Event::KeyDown { keycode, .. } => {
                        match keycode {
                            Some(Keycode::Escape) => app_state.is_running = false,
                            Some(Keycode::Tab) => app.show_depth_map = !app.show_depth_map,
                            Some(Keycode::Space) => self.fix_view.pressed = true,
                            Some(Keycode::Down) => self.power = -1.0,
                            Some(Keycode::Up) => self.power = 1.0,
                            Some(Keycode::Q) => self.yaw = -1.0,
                            Some(Keycode::E) => self.yaw = 1.0,
                            Some(Keycode::A) => self.x = -1.0,
                            Some(Keycode::D) => self.x = 1.0,
                            Some(Keycode::S) => self.y = -1.0,
                            Some(Keycode::W) => self.y = 1.0,
                            _ => {},
                        }
                    },
                    Event::KeyUp { keycode, .. } => {
                        match keycode {
                            Some(Keycode::Down) => self.power = 0.0,
                            Some(Keycode::Space) => self.fix_view.pressed = false,
                            Some(Keycode::Up) => self.power = 0.0,
                            Some(Keycode::Q) => self.yaw = 0.0,
                            Some(Keycode::E) => self.yaw = 0.0,
                            Some(Keycode::A) => self.x = 0.0,
                            Some(Keycode::D) => self.x = 0.0,
                            Some(Keycode::S) => self.y = 0.0,
                            Some(Keycode::W) => self.y = 0.0,
                            _ => {},
                        }
                    },
                    Event::Quit { .. } => {
                        app_state.is_running = false;
                    },
                    _ => {}
                }
            }
            app.throttling.last_controller_update = Instant::now();
        }
    }
}