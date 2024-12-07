

use glyphon::{cosmic_text::Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, TextArea, TextBounds};

use crate::{app::Size, rendering::vertex::VertexUi};

use super::{ui_node::UiNode, ui_transform::{Rect, UiTransform}};




// make this variable later
const FONT_SIZE: f32 = 20.0;
const LINE_HEIGHT: f32 = 10.0;
const BASE_FONT: Family = Family::SansSerif;

#[derive(Debug)]
pub struct TextWidth {
    pub width: f32,
    pub buffer_width: f32,
}

/// # Label
/// 
/// Label is a Ui node that let us show/display text in a certain point of the screen, with an especific color, centering and other settings.
/// 
/// The main use of if is for the display of game information that are **NOT INTERACTABLE**, at least not by clicking, just by reading or writing on it.
/// 
/// This struct have the next parameters:
/// - text: The displayable text
/// - buffer: The kind of border that will be displayed

pub struct Label {
    pub buffer: Buffer,
    text: String,
    pub color: Color
}

impl Label {
    pub fn new(font_system: &mut FontSystem, text: &str, container_transform: UiTransform, color: Color, align: Align, font_size: f32) -> Self {
        // adjust the line height
        let mut buffer = Buffer::new(font_system, Metrics::new(FONT_SIZE, LINE_HEIGHT));

        if text != "" {
            // set the size and text of the lable
            buffer.set_size( font_system, ((container_transform.x + container_transform.width) - container_transform.x) as f32, ((container_transform.y + container_transform.height) - container_transform.y) as f32,);
            buffer.set_text(font_system, text, Attrs::new().family(BASE_FONT), Shaping::Advanced);

            // alignment for each line
            buffer.lines.iter_mut().for_each(|line| {
                line.set_align(Some(align));
            });

            // how the text cuts if it exceed the size of the container
            buffer.set_wrap(font_system, glyphon::Wrap::None);

            buffer.shape_until_scroll(font_system);
        }

        Self {
            buffer,
            text: text.to_owned(),
            color
        }
    }

    /// # Ui node render data getter
    /// 
    /// This function will mainly get as a parameter information about the renderizable element, mainly a list of vertex and indices
    /// 
    /// ## Returns:
    /// A amount of values ordered as (text area, num_vertices, num indices)
    pub fn ui_node_data_creation(&mut self, size: &Size, vertices: &mut Vec<VertexUi>, vertices_slice: &[VertexUi; 4], indices: &mut Vec<u16>, indices_slice: &[u16; 6], num_vertices: &mut u16, parent_rect: &Rect) -> (TextArea, u16, u32) {
        vertices.extend_from_slice(vertices_slice);
        indices.extend_from_slice(indices_slice); 

        // *num_vertices += node_vertices.len() as u16;
        // *num_indices += UiNode::NUM_INDICES;

        // sets the new text
        (self.text_area(parent_rect), vertices_slice.len() as u16, UiNode::NUM_INDICES)
    }

    pub fn text_area(&self, parent_rect: &Rect) -> TextArea {
        let text_width = self.get_text_width();
        let TextWidth { width, buffer_width } = text_width;

        let text_overlap = if width > buffer_width {
            width - buffer_width
        } else {
            0.0
        };

        let text_area = TextArea {
            buffer: &self.buffer,
            left: parent_rect.left as f32 - text_overlap,
            top: self.vertical_positioning_in_rect(&parent_rect),
            scale: 1.0,
            bounds: self.bounds(&parent_rect),
            default_color: self.color,
        };

        return text_area
    }

    pub fn get_text_width(&self) -> TextWidth {
        TextWidth {
            width: self.buffer.layout_runs().fold(0.0, |width, run| run.line_w.max(width)),
            buffer_width: self.buffer.size().0,
        }
    }

    fn bounds(&self, rect: &Rect) -> TextBounds {
        TextBounds {
            left: rect.left as i32,
            top: rect.top as i32,
            right: rect.right as i32,
            bottom: rect.bottom as i32,
        }
    }  

    // this function is dedicated to center vertically where we display the text in the rect
    fn vertical_positioning_in_rect(&self, rect: &Rect) -> f32 {
        (rect.bottom - (rect.bottom - rect.top) / 2) as f32 - (self.buffer.metrics().line_height / 2.0)
    }

    pub fn set_text(&mut self, font_system: &mut FontSystem, text: &str, realign: bool) {
        if text != self.text {
            self.text = text.to_owned();
            // self.buffer.set_size( font_system, (self.rect_pos.right - self.rect_pos.left) as f32, (self.rect_pos.bottom - self.rect_pos.top) as f32,);
            self.buffer.set_text(font_system, text, Attrs::new().family(Family::SansSerif), Shaping::Advanced);

            if realign {
                self.buffer.lines.iter_mut().for_each(|line| {
                    line.set_align(Some(glyphon::cosmic_text::Align::Center));
                }); 

                self.buffer.set_wrap(font_system, glyphon::Wrap::None);
                self.buffer.shape_until_scroll(font_system);
            }
        }
    }
}