use std::{
    collections::HashMap,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use glyphon::{cosmic_text::Align, Color};
use serde::Deserialize;

use crate::{
    app::App,
    rendering::ui::UiContainer,
    ui::{
        ui_node::{ChildrenType, UiNode, UiNodeContent, UiNodeParameters, Visibility},
        ui_transform::UiTransform,
        vertical_container,
    },
    utils::lerps::lerp,
};

const MAX_DURATION: f32 = 5.0;

/// # Subtitles
/// Subtitle is the struct that will let us show text based on elements in screen, this text will be added, and have a duration once the inner timer
/// ends, it will make it disappear on the subtitles.
///
/// texts: A list
struct SubtitleLine {
    instance_time: Instant,
    color: Color,
}

#[derive(Debug, Deserialize)]
pub struct SubtitleData {
    pub subtitles: HashMap<u64, String>,
}

/*
SubtitleData(
    subtitles: {
    3000: "So, have you found a reason to fight yet?"
    7000: "Buddy."
    },
)
*/

pub struct Subtitle {
    texts: Vec<SubtitleLine>,
}

impl Subtitle {
    pub fn new() -> Self {
        Self { texts: Vec::new() }
    }

    pub fn update(&mut self, app: &mut App) {
        // check every element in the list of texts and then if their life span ends, it will delete them
        let Some(UiContainer::Tagged(hash_map)) = app.ui.renderizable_elements.get_mut("static")
        else {
            return;
        };

        let Some(subtitles) = hash_map.get_mut("subtitles") else {
            println!("subtitles not found on there");
            return;
        };

        let UiNodeContent::VerticalContainer(vertical_container_data) = &mut subtitles.content
        else {
            return;
        };

        let subtitles_alpha_end = if self.texts.len() > 0 { 0.7 } else { 0.0 };

        subtitles.visibility.background_color[3] = lerp(
            subtitles.visibility.background_color[3],
            subtitles_alpha_end,
            app.time.delta_time * 7.0,
        );

        let mut last_index: Option<usize> = None;
        for (index, text) in &mut self.texts.iter_mut().enumerate() {
            if let ChildrenType::IndexedChildren(vec) = &mut vertical_container_data.children {
                let UiNodeContent::Text(label) = &mut vec[index].content else {
                    continue;
                };

                let new_alpha = if text.instance_time.elapsed().as_secs_f32() > MAX_DURATION {
                    lerp(text.color.a().into(), 0.0, app.time.delta_time) as u8
                } else {
                    lerp(label.color.a().into(), 255.0, app.time.delta_time * 7.0) as u8
                };

                let new_text_color =
                    Color::rgba(text.color.r(), text.color.g(), text.color.b(), new_alpha);
                text.color = new_text_color;
                label.color = new_text_color;
            } else {
                todo!("ChildrenType::MappedChildren")
            };

            if text.color.a() == 0 {
                last_index = Some(index + 1)
            }
        }

        if let Some(index) = last_index {
            if let ChildrenType::IndexedChildren(vec) = &mut vertical_container_data.children {
                vec.drain(0..index);
            }

            self.texts.drain(0..index);
        }
    }

    pub fn add_text(&mut self, text: &String, app: &mut App) {
        let subtitle_node = UiNode::new(
            UiTransform::new(0.0, 0.0, 30.0, 200.0, 0.0, false),
            Visibility::new([0.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 0.0]),
            UiNodeParameters::Text {
                text,
                color: Color::rgba(255, 255, 255, 0),
                align: Align::Center,
                font_size: 15.0,
            },
            app,
        );

        let new_text = SubtitleLine {
            instance_time: Instant::now(),
            color: Color::rgba(255, 255, 255, 0),
        };

        let Some(UiContainer::Tagged(hash_map)) = app.ui.renderizable_elements.get_mut("static")
        else {
            return;
        };

        if let Some(subtitles) = hash_map.get_mut("subtitles") {
            if let UiNodeContent::VerticalContainer(vertical_container_data) =
                &mut subtitles.content
            {
                vertical_container_data.add_if_indexed(subtitle_node);
            }
        } else {
            println!("subtitles not found on there");
        }

        self.texts.push(new_text);
    }
}
