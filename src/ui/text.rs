use sdl2::{render::{Canvas, TextureCreator, TextureQuery}, ttf::Font, video::{Window, WindowContext}};
use sdl2::pixels::Color;
use sdl2::rect::Rect;

use crate::game_object::GameObject;

#[derive(Clone)]

pub struct Label {
    pub game_object: GameObject,
    pub text: String,
    pub color: Color,
    pub text_color: Color,
}

impl Label {
    pub fn new(game_object: GameObject, text: String, color: Color, text_color: Color) -> Self {
        Label {
            game_object,
            text: text,
            color,
            text_color, 
        }
    }

    pub fn render(&self, canvas: &mut Canvas<Window>, texture_creator: &TextureCreator<WindowContext>, font: &Font) {
        if self.game_object.active == true {
            canvas.set_draw_color(self.color); // it must be a Color::RGB() or other
            canvas.fill_rect(Rect::new(self.game_object.x as i32, self.game_object.y as i32, self.game_object.width as u32, self.game_object.height as u32)).unwrap();

            // Render the button text
                    let surface = font.render(&self.text).solid(self.text_color).expect("Something went wrong while creating the surface");
                    let texture = texture_creator.create_texture_from_surface(&surface).expect("Something went wrong while creating the texture");
        
                    // We center the text on the button
                    let TextureQuery { width: text_width, height: text_height, .. } = texture.query();
                    let text_x = self.game_object.x as i32 + (self.game_object.width as i32 - text_width as i32) / 2;
                    let text_y = self.game_object.y as i32 + (self.game_object.height as i32 - text_height as i32) / 2;
        
                    // render
                    canvas.copy(&texture, None, Rect::new(text_x, text_y, text_width, text_height)).unwrap();
        }
    }
}