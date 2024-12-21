// this structure will define the buttons, the data type of each and what we will do with each one of them, 
// we will modify this every time we will add or delete a control

use std::time::Instant;

use sdl2::{controller::{Axis, GameController}, event::Event, keyboard::Keycode};

use crate::app::{App, AppState};

/// # Input
/// This structure will be setted for key presses that are supposed to be taken as booleans.
pub struct Input {
    pub pressed: bool,
    pub just_pressed: bool,
    pub released: bool,
    pub time_pressed: f32,
}

pub struct Mouse {
    pub x: i32,
    pub y: i32,
    pub sensitivity: f32
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
    pub change_camera: Input,
    pub look_back: bool,
    pub ui_up: bool,
    pub ui_down: bool,
    pub ui_left: bool,
    pub ui_right: bool,
    pub ui_select: bool,
    pub mouse: Mouse
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
            fix_view: Input { pressed: false, just_pressed: false, released: false, time_pressed: 0.0 },
            fix_view_hold_window: 0.2,
            change_camera: Input { pressed: false, just_pressed: false, released: false, time_pressed: 0.0 },
            look_back: false,
            ui_up: false,
            ui_down: false,
            ui_left: false,
            ui_right: false,
            ui_select: false,
            mouse: Mouse { x: 0, y: 0, sensitivity: 0.5 },
        }
    }

    pub fn update(&mut self, app_state: &mut AppState, event_pump: &mut sdl2::EventPump, app: &mut App, controller: &Option<GameController>, delta_time: f32) {
        if self.fix_view.pressed {
            self.fix_view.time_pressed += delta_time
        } else {
            self.fix_view.released = false;
        }

        self.fix_view.just_pressed = false;

        if !self.change_camera.pressed {
            self.change_camera.released = false;
        }

        if self.ui_down == true {
            self.ui_down = false;
        }

        if self.ui_up == true {
            self.ui_up = false;
        }

        if self.ui_left == true {
            self.ui_left = false;
        }

        if self.ui_right == true {
            self.ui_right = false;
        }


        if app.throttling.last_controller_update.elapsed() >= app.throttling.controller_update_interval {
            for event in event_pump.poll_iter() {
                match event {
                    Event::ControllerButtonDown { button, .. } => {
                        match button {
                            sdl2::controller::Button::Y => {
                                self.fix_view.pressed = true;
                                self.fix_view.just_pressed = true;
                                self.fix_view.time_pressed = 0.0;
                            },
                            sdl2::controller::Button::RightStick => self.change_camera.pressed = true,
                            sdl2::controller::Button::LeftShoulder => self.yaw = -1.0,
                            sdl2::controller::Button::RightShoulder => self.yaw = 1.0,
                            sdl2::controller::Button::DPadUp => self.ui_up = true,
                            sdl2::controller::Button::DPadDown => self.ui_down = true,
                            sdl2::controller::Button::DPadLeft => self.ui_left = true,
                            sdl2::controller::Button::DPadRight => self.ui_right = true,
                            sdl2::controller::Button::A => self.ui_select = true,
                            _ => {}
                        }
                    }
                    Event::ControllerButtonUp { button, .. } => {
                        match button {
                            sdl2::controller::Button::Y => {
                                self.fix_view.pressed = false;
                                self.fix_view.released = true;
                            },
                            sdl2::controller::Button::Back => {
                                // change camera
                            },
                            sdl2::controller::Button::RightStick => {
                                self.change_camera.pressed = false;
                                self.change_camera.released = true;
                            },
                            sdl2::controller::Button::LeftShoulder => {
                                self.yaw = 0.0
                            },
                            sdl2::controller::Button::RightShoulder => {
                                self.yaw = 0.0
                            },
                            sdl2::controller::Button::A => {
                                self.ui_select = false;
                            },
                            _ => {}
                        }
                    },
                    Event::JoyAxisMotion { timestamp: _, which: _, axis_idx, value } => {
                        // println!("Joystick {} Axis {} moved to {}", which, axis_idx, value);
                        if axis_idx == 0 {
                            self.x = value as f32 / 32767.0;                                    
                        } else if axis_idx == 1 {
                            self.y = -(value as f32 / 32767.0);
                        } else if axis_idx == 2 {
                            self.power = (value as i32 - 32767).abs() as f32 / 65536.0;
                        } else if axis_idx == 5 {
                            self.yaw = -(value as f32 / 32767.0);
                        }
                    }
                    Event::JoyButtonDown { timestamp: _, which: _, button_idx } => {
                        // println!("Joystick {} Button {} pressed", which, button_idx);
                        if button_idx == 19 {
                            self.fix_view.pressed = true;
                            self.fix_view.just_pressed = true;
                            self.fix_view.time_pressed = 0.0;
                        } else if button_idx == 3 {
                            self.change_camera.pressed = true;
                        }
                    }
                    Event::JoyButtonUp { timestamp: _, which: _, button_idx } => {
                        // println!("Joystick {} Button {} pressed", which, button_idx);
                        if button_idx == 19 {
                            self.fix_view.pressed = false;
                            self.fix_view.released = true;
                        } else if button_idx == 3 {
                            self.change_camera.pressed = false;
                            self.change_camera.released = true;
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
                        }
                    }
                    Event::KeyDown { keycode, .. } => {
                        match keycode {
                            Some(Keycode::Escape) => app_state.is_running = false,
                            Some(Keycode::Tab) => app.show_depth_map = !app.show_depth_map,
                            Some(Keycode::Space) => {
                                self.fix_view.pressed = true;
                                self.fix_view.just_pressed = true;
                            },
                            Some(Keycode::Down) => self.power = -1.0,
                            Some(Keycode::Up) => self.power = 1.0,
                            Some(Keycode::Q) => self.yaw = -1.0,
                            Some(Keycode::E) => self.yaw = 1.0,
                            Some(Keycode::A) => {
                                self.ui_left = true;
                                self.x = -1.0;
                            },
                            Some(Keycode::D) => {
                                self.ui_right = true;
                                self.x = 1.0;
                            },
                            Some(Keycode::S) => self.y = -1.0,
                            Some(Keycode::W) => self.y = 1.0,
                            Some(Keycode::V) => self.change_camera.pressed = true,
                            _ => {},
                        }
                    },
                    Event::KeyUp { keycode, .. } => {
                        match keycode {
                            Some(Keycode::Down) => self.power = 0.0,
                            Some(Keycode::Space) => {
                                self.fix_view.pressed = false;
                                self.fix_view.released = true;
                            },
                            Some(Keycode::Up) => self.power = 0.0,
                            Some(Keycode::Q) => self.yaw = 0.0,
                            Some(Keycode::E) => self.yaw = 0.0,
                            Some(Keycode::A) => self.x = 0.0,
                            Some(Keycode::D) => self.x = 0.0,
                            Some(Keycode::S) => self.y = 0.0,
                            Some(Keycode::W) => self.y = 0.0,
                            Some(Keycode::V) => {
                                self.change_camera.pressed = false;
                                self.change_camera.released = true;
                            },
                            _ => {},
                        }
                    },
                    Event::MouseMotion { xrel, yrel, .. } => {
                        self.mouse.x += xrel;

                        let value = (self.mouse.y + yrel).clamp(-80, 80);
                        self.mouse.y = value;
                    }
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