use std::{
    collections::HashMap,
    time::Instant,
};

use pankarta_software::prelude::*;
use pankarta_software::ui::ui_node::UiNodeContent;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct SubtitleEntry {
    pub text: String,
    pub duration: u64,
}

#[derive(Debug, Deserialize)]
pub struct SubtitleData {
    pub subtitles: HashMap<u64, SubtitleEntry>,
}

/// The subtitle line - the root text node of a `UiLayer` (the play
/// scene's "subtitles"), shown for a while after `add_text`.
pub struct Subtitle {
    show_time: Option<Instant>,
    duration: f32,
}

/// The subtitle line's top-level UI key - its layer's root is the line.
pub const SUBTITLES_PATH: &str = "subtitles";

impl Subtitle {
    pub fn new() -> Self {
        Self { show_time: None, duration: 0.0 }
    }

    pub fn update(&mut self, layer: &mut UiLayer) {
        let visible = match &self.show_time {
            Some(t) if t.elapsed().as_secs_f32() < self.duration => true,
            _ => false,
        };

        let Some(node) = layer.get_mut("") else { return };
        let UiNodeContent::Text(_) = &node.content else { return };

        if visible {
            node.update_style(|s| s.set_text_color(UiColor::WHITE).set_background_color(UiColor::Rgba(0, 0, 0, 179)));
        } else {
            node.update_style(|s| s.set_text_color(UiColor::Rgba(255, 255, 255, 0)).set_background_color(UiColor::TRANSPARENT));
        }
    }

    pub fn add_text(&mut self, text: &str, duration_ms: u64, layer: &mut UiLayer, screen_width: f32) {
        self.duration = duration_ms as f32 / 1000.0;

        let Some(node) = layer.get_mut("") else { return };
        let UiNodeContent::Text(label) = &mut node.content else { return };

        label.set_text(text);

        let text_width = label.get_text_width().width;
        let padded_width = text_width + 20.0;
        node.transform.width = padded_width;
        node.transform.x = (screen_width / 2.0) - (padded_width / 2.0);
        node.transform.rect.left = node.transform.x;
        node.transform.rect.right = node.transform.x + padded_width;

        self.show_time = Some(Instant::now());
    }
}
