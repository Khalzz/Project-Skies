//! The `UiNodeContent::Text` payload - deliberately thin, since color/font_size/
//! alignment/alpha all live on the owning `UiNode`'s `Style` instead (see
//! `UiNode.style`/`.hover`), resolved fresh every relevant frame in
//! `node_content_preparation`. That's what lets a hover override apply uniformly to
//! every content type without `Label` needing its own copy/restore logic.
//!
//! - `Label::new(font_system, text, width, height)` - builds the glyphon `Buffer` at
//!   `DEFAULT_FONT_SIZE`; the owning `UiNode`'s actual font size/alignment take over
//!   before this is ever rendered, see `apply_style`.
//! - `measure_or(...)` - auto-measures whichever of width/height the caller left as
//!   `None`, used by `UiNode::label`.
//! - `apply_style(font_system, font_size, align)` - re-shapes the buffer at a given
//!   size/alignment, called fresh every frame from the owning node's resolved style.
//! - `text_area`/`ui_node_data_creation` - build the glyphon `TextArea` (and vertex/
//!   index data) this frame's render pass needs, given a resolved `color`.
//! - `set_text(font_system, text, realign)` - changes the displayed text in place
//!   (a no-op if it's unchanged); `get_text_width` reads back the shaped result.

use glyphon::{cosmic_text::Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, TextArea, TextBounds};

use crate::app::Size;
use crate::engine::rendering::vertex::VertexUi;
use crate::engine::ui::ui_node::UiNode;
use crate::engine::ui::ui_transform::Rect;

// Used as the fallback when a node's resolved Style has no font_size (see
// UiNode::node_content_preparation) - not baked into rendering unconditionally,
// callers change it via `.set_font_size(...)`.
pub const DEFAULT_FONT_SIZE: f32 = 20.0;
// Line height is derived from font_size (see Metrics::new calls below) rather than a
// flat constant, specifically so it scales with whatever size is actually in use -
// a fixed line height smaller than the glyph size was the previous source of
// overlapping multi-line text (see AUTO_HEIGHT below).
const LINE_HEIGHT_RATIO: f32 = 1.2;
const BASE_FONT: Family = Family::SansSerif;
// Auto-height fallback for a single line of text - generous enough for the default
// font size without clipping. Multi-line auto-sizing still isn't supported: this is
// a flat value regardless of font_size, so a much larger custom size could still
// clip - not fixed here, out of scope for enabling font_size itself.
const AUTO_HEIGHT: f32 = 28.0;

#[derive(Debug)]
pub struct TextWidth {
    pub width: f32,
    pub buffer_width: f32,
}

pub struct Label {
    pub buffer: Buffer,
    text: String,
}

impl Label {
    /// Resolves a label's box size, auto-measuring whichever dimension the caller
    /// left as `None`. Width is measured from the text's own natural (unwrapped)
    /// layout; height just falls back to `AUTO_HEIGHT` (see its doc comment for why
    /// auto height doesn't attempt to support multi-line text).
    pub fn measure_or(font_system: &mut FontSystem, text: &str, width: Option<f32>, height: Option<f32>) -> (f32, f32) {
        let resolved_height = height.unwrap_or(AUTO_HEIGHT);
        let resolved_width = match width {
            Some(w) => w,
            None => {
                let mut buffer = Buffer::new(font_system, Metrics::new(DEFAULT_FONT_SIZE, DEFAULT_FONT_SIZE * LINE_HEIGHT_RATIO));
                buffer.set_size(font_system, None, Some(resolved_height));
                buffer.set_text(font_system, text, &Attrs::new().family(BASE_FONT), Shaping::Advanced);
                buffer.set_wrap(font_system, glyphon::Wrap::None);
                buffer.shape_until_scroll(font_system, true);
                buffer.layout_runs().fold(0.0f32, |w, run| run.line_w.max(w))
            }
        };
        (resolved_width, resolved_height)
    }

    /// Builds the buffer at `DEFAULT_FONT_SIZE` with no particular alignment set -
    /// `node_content_preparation` calls `apply_style` before this label is ever
    /// rendered, so the real font size/alignment (from the owning UiNode's resolved
    /// Style) takes over before anyone sees this initial state.
    pub fn new(font_system: &mut FontSystem, text: &str, width: f32, height: f32) -> Self {
        let mut buffer = Buffer::new(font_system, Metrics::new(DEFAULT_FONT_SIZE, DEFAULT_FONT_SIZE * LINE_HEIGHT_RATIO));

        buffer.set_size(font_system, Some(width), Some(height));
        buffer.set_text(font_system, text, &Attrs::new().family(BASE_FONT), Shaping::Advanced);
        buffer.set_wrap(font_system, glyphon::Wrap::None);
        buffer.shape_until_scroll(font_system, true);

        Self { buffer, text: text.to_owned() }
    }

    /// Re-shapes this label's buffer at `font_size`/`align` - called fresh every
    /// applicable frame (see `UiNode::node_content_preparation`) from whatever the
    /// owning node's resolved `Style` says this frame (its own, or hover-
    /// overridden), so leaving hover naturally re-shapes back to the resting value
    /// next frame - no restore bookkeeping needed.
    pub fn apply_style(&mut self, font_system: &mut FontSystem, font_size: f32, align: Align) {
        self.buffer.set_metrics(font_system, Metrics::new(font_size, font_size * LINE_HEIGHT_RATIO));
        self.buffer.lines.iter_mut().for_each(|line| {
            line.set_align(Some(align));
        });
        self.buffer.shape_until_scroll(font_system, true);
    }

    /// `color` - the owning UiNode's resolved Style color for this frame (own or
    /// hover-overridden - see `UiNode::node_content_preparation`).
    pub fn ui_node_data_creation(&self, _size: &Size, vertices: &mut Vec<VertexUi>, vertices_slice: &[VertexUi; 4], indices: &mut Vec<u16>, indices_slice: &[u16; 6], parent_rect: &Rect, color: Color) -> (TextArea, u16, u32) {
        vertices.extend_from_slice(vertices_slice);
        indices.extend_from_slice(indices_slice);

        (self.text_area(parent_rect, color), vertices_slice.len() as u16, UiNode::NUM_INDICES)
    }

    pub fn text_area(&self, parent_rect: &Rect, color: Color) -> TextArea {
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
            default_color: color,
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
