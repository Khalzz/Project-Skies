use glyphon::{cosmic_text::Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, TextArea, TextBounds};

use crate::app::Size;
use crate::rendering::vertex::VertexUi;
use crate::ui::ui_node::UiNode;
use crate::ui::ui_transform::Rect;

const FONT_SIZE: f32 = 20.0;
const LINE_HEIGHT: f32 = 10.0;
const BASE_FONT: Family = Family::SansSerif;

#[derive(Debug)]
pub struct TextWidth {
    pub width: f32,
    pub buffer_width: f32,
}

pub struct Label {
    pub buffer: Buffer,
    text: String,
    pub color: Color,
}

impl Label {
    pub fn new(font_system: &mut FontSystem, text: &str, width: f32, height: f32, color: Color, align: Align, _font_size: f32) -> Self {
        let mut buffer = Buffer::new(font_system, Metrics::new(FONT_SIZE, LINE_HEIGHT));

        buffer.set_size(font_system, Some(width), Some(height));
        buffer.set_text(font_system, text, &Attrs::new().family(BASE_FONT), Shaping::Advanced);

        buffer.lines.iter_mut().for_each(|line| {
            line.set_align(Some(align));
        });

        buffer.set_wrap(font_system, glyphon::Wrap::None);
        buffer.shape_until_scroll(font_system, true);

        Self {
            buffer,
            text: text.to_owned(),
            color,
        }
    }

    pub fn ui_node_data_creation(&self, _size: &Size, vertices: &mut Vec<VertexUi>, vertices_slice: &[VertexUi; 4], indices: &mut Vec<u16>, indices_slice: &[u16; 6], parent_rect: &Rect) -> (TextArea, u16, u32) {
        vertices.extend_from_slice(vertices_slice);
        indices.extend_from_slice(indices_slice);

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

        TextArea {
            buffer: &self.buffer,
            left: parent_rect.left - text_overlap,
            top: self.vertical_positioning_in_rect(parent_rect),
            scale: 1.0,
            bounds: self.bounds(parent_rect),
            default_color: self.color,
            custom_glyphs: &[],
        }
    }

    pub fn get_text_width(&self) -> TextWidth {
        let width_buffer = self.buffer.size().0.unwrap_or(0.0);

        TextWidth {
            width: self.buffer.layout_runs().fold(0.0, |width, run| run.line_w.max(width)),
            buffer_width: width_buffer,
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

    fn vertical_positioning_in_rect(&self, rect: &Rect) -> f32 {
        (rect.bottom - (rect.bottom - rect.top) / 2.0) - (self.buffer.metrics().line_height / 2.0)
    }

    pub fn set_text(&mut self, font_system: &mut FontSystem, text: &str, realign: bool) {
        if text != self.text {
            self.text = text.to_owned();
            self.buffer.set_text(font_system, text, &Attrs::new().family(Family::SansSerif), Shaping::Advanced);
            if realign {
                self.realign(font_system);
            }
        }
    }

    pub fn realign(&mut self, font_system: &mut FontSystem) {
        self.buffer.lines.iter_mut().for_each(|line| {
            line.set_align(Some(Align::Center));
        });

        self.buffer.set_wrap(font_system, glyphon::Wrap::None);
        self.buffer.shape_until_scroll(font_system, true);
    }
}
