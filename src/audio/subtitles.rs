use std::{collections::HashMap, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};

use glyphon::{cosmic_text::Align, Color};
use serde::Deserialize;

use crate::{app::App, ui::{ui_node::{UiNode, UiNodeContent, UiNodeParameters, Visibility}, ui_transform::UiTransform, vertical_container}, utils::lerps::lerp};

const MAX_DURATION: f32 = 5.0;

/// # Subtitles
/// Subtitle is the struct that will let us show text based on elements in screen, this text will be added, and have a duration once the inner timer
/// ends, it will make it disappear on the subtitles.
///
/// texts: A list 
struct SubtitleLine {
    instance_time: Instant,
    color: Color
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
        Self {
            texts: Vec::new(),
        }
    }

    pub fn update(&mut self, app: &mut App) {
        // check every element in the list of texts and then if their life span ends, it will delete them
        match app.ui.renderizable_elements.get_mut("static") {
            Some(static_list) => {
                match static_list {
                    crate::rendering::ui::UiContainer::Tagged(hash_map) => {
                        match hash_map.get_mut("subtitles") {
                            Some(subtitles) => {
                                match &mut subtitles.content {
                                    UiNodeContent::VerticalContainer(vertical_container_data) => {
                                        let mut last_index: Option<usize> = None;
                                        if self.texts.len() > 0 {
                                            subtitles.visibility.background_color[3] = lerp(subtitles.visibility.background_color[3], 0.7, app.time.delta_time * 7.0);
                                            for (index, text) in &mut self.texts.iter_mut().enumerate() {
                                                if text.instance_time.elapsed().as_secs_f32() > MAX_DURATION {
                                                    match &mut vertical_container_data.children {
                                                        crate::ui::ui_node::ChildrenType::IndexedChildren(vec) => {
                                                            match &mut vec[index].content {
                                                                UiNodeContent::Text(label) => {
                                                                    let new_alpha = lerp(text.color.a().into(), 0.0, app.time.delta_time) as u8;
                                                                    text.color = Color::rgba(text.color.r(), text.color.g(), text.color.b(), new_alpha);
                                                                    label.color = text.color;
                                                                },
                                                                _ => {},
                                                            }
                                                        },
                                                        crate::ui::ui_node::ChildrenType::MappedChildren(hash_map) => todo!(),
                                                    }
                                                } else {
                                                    match &mut vertical_container_data.children {
                                                        crate::ui::ui_node::ChildrenType::IndexedChildren(vec) => {
                                                            match &mut vec[index].content {
                                                                UiNodeContent::Text(label) => {
                                                                    let new_alpha = lerp(label.color.a().into(), 255.0, app.time.delta_time * 7.0) as u8;
                                                                    let new_text_color = Color::rgba(text.color.r(), text.color.g(), text.color.b(), new_alpha);
                                                                    text.color = new_text_color;
                                                                    label.color = new_text_color;
                                                                },
                                                                _ => {},
                                                            }
                                                        },
                                                        crate::ui::ui_node::ChildrenType::MappedChildren(hash_map) => todo!(),
                                                    }
                                                }
                                                
                                                if text.color.a() == 0 {
                                                    last_index = Some(index)
                                                }
                                            }    
                                        } else {
                                            subtitles.visibility.background_color[3] = lerp(subtitles.visibility.background_color[3], 0.0, app.time.delta_time * 7.0);
                                        }
                                        
                                        match last_index {
                                            Some(index) => {
                                                match &mut vertical_container_data.children {
                                                    crate::ui::ui_node::ChildrenType::IndexedChildren(vec) => {vec.drain(0..(index + 1));},
                                                    _ => {},
                                                }

                                                self.texts.drain(0..(index + 1));
                                            },
                                            _ => {}
                                        }                                
                                    },
                                    _ => {},
                                }
                            },
                            None => {
                                println!("subtitles not found on there");
                            },
                        }
                    },
                    _ => {},
                }
            },
            None => {},
        }
    }

    pub fn add_text(&mut self, text: &String, app: &mut App) {
        let subtitle_node = UiNode::new(
            UiTransform::new(0.0, 0.0, 30.0, 200.0, 0.0, false),
            Visibility::new([0.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 0.0]),
            UiNodeParameters::Text { text, color: Color::rgba(255, 255, 255, 0), align: Align::Center, font_size: 15.0 }, 
            app,
        );

        let new_text = SubtitleLine {
            instance_time: Instant::now(),
            color: Color::rgba(255, 255, 255, 0),
        };

        match app.ui.renderizable_elements.get_mut("static") {
            Some(static_list) => {
                match static_list {
                    crate::rendering::ui::UiContainer::Tagged(hash_map) => {
                        match hash_map.get_mut("subtitles") {
                            Some(subtitles) => {
                                match &mut subtitles.content {
                                    UiNodeContent::VerticalContainer(vertical_container_data) => {
                                        vertical_container_data.add_if_indexed(subtitle_node);
                                    },
                                    _ => {},
                                }
                            },
                            None => {
                                println!("subtitles not found on there");
                            },
                        }
                    },
                    _ => {},
                }
            },
            None => {},
        }

        self.texts.push(new_text);
    }
}