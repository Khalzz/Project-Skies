use std::path::Component;

use glyphon::{FontSystem, SwashCache, TextAtlas, TextRenderer};
use sdl2::sys::Window;

use crate::app::{MousePos, Size};

use super::{rectangle::{RectPos, Rectangle}, text::Text};

pub struct Button {
    pub text: Text,
    pub rectangle: Rectangle,
    // on_click: Box<dyn Fn()>,
}

struct InputState {
    clicked: bool,
    mouse_coords: MousePos,
}

pub struct ButtonConfig {
    pub rect_pos: RectPos,
    pub fill_color: [f32; 4],
    pub fill_color_active: [f32; 4],
    pub border_color: [f32; 4],
    pub border_color_active: [f32; 4],
    pub text: &'static str,
    pub text_color: glyphon::Color,
    pub text_color_active: glyphon::Color,
    // pub on_click: Box<dyn Fn()>,
}

impl Button {
    pub fn new(cfg: ButtonConfig, font_system: &mut glyphon::FontSystem) -> Self {
        Self {
            rectangle: Rectangle::new(
                cfg.rect_pos,
                cfg.fill_color,
                cfg.fill_color_active,
                cfg.border_color,
                cfg.border_color_active,
            ),
            text: Text::new(
                font_system,
                cfg.rect_pos,
                cfg.text,
                cfg.text_color,
                cfg.text_color_active,
            ),
            // on_click: cfg.on_click,
        }
    }

    pub fn click(&mut self) {
        // (self.on_click)()
    }

    pub fn is_hovered(&self, mouse_coords: &MousePos) -> bool {
        self.rectangle.is_hovered(mouse_coords)
    }

    fn rgba_to_vec4(color: glyphon::Color) -> [f32; 4] {
        let r = color.r() as f32 / 255.0;
        let g = color.g() as f32 / 255.0;
        let b = color.b() as f32 / 255.0;
        let a = color.a() as f32 / 255.0;
        [r, g, b, a]
    }
}