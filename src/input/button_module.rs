use sdl2::{mouse::MouseButton, render::{Canvas, TextureCreator, TextureQuery}, ttf::Font, video::{Window, WindowContext}};
use sdl2::pixels::Color;
use sdl2::rect::Rect;

use crate::game_object::GameObject;

#[derive(Clone)]
pub enum TextAlign {
    Left,
    Center,
}

#[derive(Clone)]

pub struct Button {
    pub game_object: GameObject,
    pub text: Option<String>,
    pub color: Color,
    pub text_color: Color,
    pub base_color: Color,
    pub hover_color: Color,
    pub clicked_color: Color,
    pub hover: bool,
    pub clicked: bool,
    pub lclicked: bool,
    pub toggle: Option<bool>,
    pub text_align: TextAlign,
}

impl Button {
    pub fn new(game_object: GameObject, text: Option<String>, color: Color, text_color: Color, hover_color: Color, clicked_color: Color, toggle: Option<bool>, text_align: TextAlign) -> Self {
        
        Button {
            game_object,
            text: text,
            color,
            text_color,
            base_color: color,
            hover_color,
            clicked_color,
            hover: false,
            clicked: false,
            lclicked: false,
            toggle,
            text_align,
        }
    }

    pub fn render(&self, canvas: &mut Canvas<Window>, texture_creator: &TextureCreator<WindowContext>, font: &Font) {
        if self.game_object.active == true {
            match self.toggle {
                Some(value) => {
                    if value {
                        canvas.set_draw_color(self.clicked_color); // it must be a Color::RGB() or other
                    } else {
                        canvas.set_draw_color(self.color); // it must be a Color::RGB() or other
                    }
                },
                None => {
                    canvas.set_draw_color(self.color); // it must be a Color::RGB() or other
                },
            }
            canvas.fill_rect(Rect::new(self.game_object.x as i32, self.game_object.y as i32, self.game_object.width as u32, self.game_object.height as u32)).unwrap();

            // Render the button text
            match &self.text {
                Some(_text) => {
                    match font.render(&_text).solid(self.text_color) {
                        Ok(surface) => {
                            match texture_creator.create_texture_from_surface(&surface) {
                                Ok(texture) => {
                                    let TextureQuery { width: text_width, height: text_height, .. } = texture.query();
                                    let mut text_x = 0;
                                    let mut text_y = 0;

                                    match self.text_align {
                                        TextAlign::Left => {
                                            text_x = self.game_object.x as i32;
                                            text_y = self.game_object.y as i32;
                                        },
                                        TextAlign::Center => {
                                            text_x = self.game_object.x as i32 + (self.game_object.width as i32 - text_width as i32) / 2;
                                            text_y = self.game_object.y as i32 + (self.game_object.height as i32 - text_height as i32) / 2;
                                        },
                                    }
                        
                                    // render
                                    canvas.copy(&texture, None, Rect::new(text_x, text_y, text_width, text_height)).unwrap();
                                },
                                // if i do a tabulation this dont works good on calibnration
                                Err(_) => {},
                            }
                        },
                        Err(_) => {},
                    };
                },
                None => {

                },
            }
            
        }
    }

    pub fn is_hover(&mut self, event: &sdl2::event::Event) {
        if self.game_object.active {
            match event { 
                sdl2::event::Event::MouseMotion {x, y, .. } => {
                    if (x > &(self.game_object.x as i32) && x < &(self.game_object.x as i32 + (self.game_object.width as i32))) && (y >= &(self.game_object.y as i32) && y <= &(self.game_object.y as i32 + (self.game_object.height as i32))) {
                        self.color = self.hover_color;
                        self.hover = true;
                    } else {
                        self.color = self.base_color;
                        self.hover = false;
                    }
                },
                _ => {} // in every other case we will do nothing
            } 
        } else {
            self.hover = false;
        }
    }

    // this function will only return true or false based on if its pressed or not
    pub fn is_clicked(&mut self, event: &sdl2::event::Event) -> bool {
        self.is_hover(event);
        self.clicked = false;
        if self.game_object.active {
            match event { 
                sdl2::event::Event::MouseButtonDown { mouse_btn: MouseButton::Left, .. } => {
                    if self.hover {
                        // (self.onclick)(pos_x, numbers, canvas);
                        self.clicked = true;
                    }
                },
                
                _ => {} // in every other case we will do nothing
            }   
            return self.clicked;   
        } else {
            return false;
        }
    }

    pub fn is_lclicked(&mut self, event: &sdl2::event::Event) -> bool {
        self.is_hover(event);
        self.lclicked = false;
        if self.game_object.active {
            match event { 
                sdl2::event::Event::MouseButtonDown { mouse_btn: MouseButton::Right, .. } => {
                    if self.hover {
                        // (self.onclick)(pos_x, numbers, canvas);
                        self.lclicked = true;
                    }
                },
                _ => {} // in every other case we will do nothing
            }   
            return self.lclicked;   
        } else {
            return false;
        }
    }

    // this function will return nothing but it will run a function inside of itself so i can deactivate it while it runs
    pub fn on_click(&mut self, event: &sdl2::event::Event) -> bool{
        if self.game_object.active {
            self.is_hover(event);
            
            if self.is_clicked(event) {
                self.hover = false;
                self.color = self.base_color;
                return true;
            }

            if self.hover {
                self.color = self.hover_color;
            } else {
                self.color = self.base_color;
            }
        }
        return false
    }

    pub fn on_lclick(&mut self, event: &sdl2::event::Event) -> bool{
        if self.game_object.active {
            self.is_hover(event);
            
            if self.is_lclicked(event) {
                return true;
            }
        }
        return false
    }
}