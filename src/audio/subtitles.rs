use std::{
    collections::HashMap,
    time::Instant,
};

use glyphon::Color;
use serde::Deserialize;

use crate::{
    app::App,
    rendering::ui::UiContainer,
    ui::ui_node::UiNodeContent,
};

#[derive(Debug, Deserialize)]
pub struct SubtitleEntry {
    pub text: String,
    pub duration: u64,
}

#[derive(Debug, Deserialize)]
pub struct SubtitleData {
    pub subtitles: HashMap<u64, SubtitleEntry>,
}

pub struct Subtitle {
    show_time: Option<Instant>,
    duration: f32,
}

impl Subtitle {
    pub fn new() -> Self {
        Self { show_time: None, duration: 0.0 }
    }

    pub fn update(&mut self, app: &mut App) {
        let visible = match &self.show_time {
            Some(t) if t.elapsed().as_secs_f32() < self.duration => true,
            _ => false,
        };

        let Some(UiContainer::Tagged(hash_map)) = app.ui.renderizable_elements.get_mut("static")
        else { return };
        let Some(node) = hash_map.get_mut("subtitles") else { return };
        let UiNodeContent::Text(label) = &mut node.content else { return };

        if visible {
            label.color = Color::rgba(255, 255, 255, 255);
            node.visibility.background_color[3] = 0.7;
        } else {
            label.color = Color::rgba(255, 255, 255, 0);
            node.visibility.background_color[3] = 0.0;
        }
    }

    pub fn add_text(&mut self, text: &str, duration_ms: u64, app: &mut App) {
        self.duration = duration_ms as f32 / 1000.0;
        let screen_width = app.config.width as f32;

        let Some(UiContainer::Tagged(hash_map)) = app.ui.renderizable_elements.get_mut("static")
        else { return };
        let Some(node) = hash_map.get_mut("subtitles") else { return };
        let UiNodeContent::Text(label) = &mut node.content else { return };

        label.set_text(&mut app.ui.text.font_system, text, true);

        let text_width = label.get_text_width().width;
        let padded_width = text_width + 20.0;
        node.transform.width = padded_width;
        node.transform.x = (screen_width / 2.0) - (padded_width / 2.0);
        node.transform.rect.left = node.transform.x;
        node.transform.rect.right = node.transform.x + padded_width;

        self.show_time = Some(Instant::now());
    }
}
