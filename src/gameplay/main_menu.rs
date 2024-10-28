use std::time::{Duration, Instant};

use cgmath::{Quaternion, Zero};
use glyphon::Color;
use sdl2::controller::GameController;
use crate::{app::{App, AppState, GameState}, primitive::rectangle::RectPos, ui::button};

use super::controller::Controller;

pub struct GameLogic { // here we define the data we use on our script
    last_frame: Instant,
    pub controller: Controller,
    pub timer: f32,
    pub selected: u8,
} 

impl GameLogic {
    // this is called once
    pub fn new(app: &mut App) -> Self {
        app.components.clear();
        // app.components.insert("background".to_owned(), background);

        Self {
            last_frame: Instant::now(),
            controller: Controller::new(0.3, 0.2),
            timer: 0.0,
            selected: 0
        }
    }

    // this is called every frame
    pub fn update(&mut self, mut app_state: &mut AppState, mut event_pump: &mut sdl2::EventPump, app: &mut App, controller: &mut Option<GameController>) {
        let delta_time_duration = self.delta_time();
        let delta_time = delta_time_duration.as_secs_f32();
        self.ui_control(app, delta_time, app_state);
        self.controller.update(&mut app_state, &mut event_pump, app, controller, delta_time);
    }

    fn ui_control(&mut self, app: &mut App, delta_time: f32, mut app_state: &mut AppState) {
        self.timer += delta_time;

        if self.controller.ui_down {
            if self.selected as usize >= 1 {
                self.selected = 0;
            } else {
                self.selected += 1
            }
        } 
        if self.controller.ui_up {
            if self.selected == 0 {
                self.selected = 1
            } else {
                self.selected -= 1
            }
        }

        match app.components.get_mut("play") {
            Some(play) => {
                if self.selected == 0 {
                    play.rectangle.color = [0.0, 1.0, 0.0, 1.0];
                    play.text.color = Color::rgba(0, 0, 0, 255);

                    if self.controller.ui_select {
                        app_state.state = GameState::Playing;
                        app_state.reset = true;
                    }
                } else {
                    play.rectangle.color = [0.0, 0.0, 0.0, 0.0];
                    play.text.color = Color::rgba(0, 255, 75, 255)
                }
            },
            None => {
                if self.timer >= 0.5 {
                    let play = button::Button::new(
                        button::ButtonConfig {
                            rect_pos: RectPos { top: app.config.height / 2 - 10, left: app.config.width / 2 - 70, bottom: app.config.height / 2 + 30, right: app.config.width / 2 + 70 },
                            fill_color: [0.0, 0.0, 0.0, 0.0],
                            fill_color_active: [0.0, 0.0, 0.0, 0.0],
                            border_color: [0.0, 1.0, 0.0, 1.0],
                            border_color_active: [0.0, 1.0, 0.0, 1.0],
                            text: "Play",
                            text_color: Color::rgba(0, 255, 75, 255),
                            text_color_active: Color::rgba(0, 255, 75, 000),
                            rotation: Quaternion::zero()
                        },
                        &mut app.ui.text.font_system,
                    );
                    app.components.insert("play".to_owned(), play);
                } 
            },
        }

        match app.components.get_mut("exit") {
            Some(exit) => {
                if self.selected == 1 {
                    exit.rectangle.color = [0.0, 1.0, 0.0 , 1.0];                    
                    exit.text.color = Color::rgba(0, 0, 0, 255);

                    if self.controller.ui_select {
                        app_state.is_running = false;
                    }
                } else {
                    exit.rectangle.color = [0.0, 0.0, 0.0, 0.0];                    
                    exit.text.color = Color::rgba(0, 255, 75, 255)
                }
            },
            None => {
                if self.timer >= 1.0 {
                    let exit = button::Button::new(
                        button::ButtonConfig {
                            rect_pos: RectPos { top: app.config.height / 2 + 40, left: app.config.width / 2 - 70, bottom: app.config.height / 2 + 80, right: app.config.width / 2 + 70 },
                            fill_color: [0.0, 0.0, 0.0, 0.0],
                            fill_color_active: [0.0, 0.0, 0.0, 0.0],
                            border_color: [0.0, 1.0, 0.0, 1.0],
                            border_color_active: [0.0, 1.0, 0.0, 1.0],
                            text: "Exit",
                            text_color: Color::rgba(0, 255, 75, 255),
                            text_color_active: Color::rgba(0, 255, 75, 000),
                            rotation: Quaternion::zero()
                        },
                        &mut app.ui.text.font_system,
                    );
                    app.components.insert("exit".to_owned(), exit);
                }
            },
        }
    }

    fn delta_time(&mut self) -> Duration {
        let current_time = Instant::now();
        let delta_time = current_time.duration_since(self.last_frame); // this is our Time.deltatime
        self.last_frame = current_time;
        return delta_time
    }
}