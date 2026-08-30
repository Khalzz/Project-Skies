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
// Deliberately more generous than LINE_HEIGHT_RATIO - purely clip headroom for
// min_height_for_font_size (below), doesn't touch the actual rendered line
// spacing (see apply_style). A box sized to exactly one line's rendered height
// leaves zero margin, and descenders (e.g. the "j" in "Project") sit close
// enough to the bottom of that nominal line box that they still got clipped by
// this label's own TextBounds (see `bounds` below, which is exactly this box) -
// a bare 1:1 fit isn't actually safe in practice.
const MIN_HEIGHT_RATIO: f32 = 1.5;
const BASE_FONT: Family = Family::SansSerif;
// Auto-height fallback for a single line of text - generous enough for
// DEFAULT_FONT_SIZE without clipping. A larger custom font_size needs more than
// this flat value gives, which is what min_height_for_font_size (below) is for
// - see UiNode::set_font_size, the only place that reads it.
const AUTO_HEIGHT: f32 = 28.0;

// A box shorter than one line at `font_size` clips text instead of showing it -
// this label's own box doubles as its TextBounds clip rect (see `bounds` below),
// so a box sized for DEFAULT_FONT_SIZE (e.g. the AUTO_HEIGHT default) no longer
// fits once a larger font_size is set. UiNode::set_font_size grows the box up to
// this if it's currently smaller, rather than leaving it to clip silently.
pub fn min_height_for_font_size(font_size: f32) -> f32 {
    font_size * MIN_HEIGHT_RATIO
}

// Shared by `measure_or` (always measures at DEFAULT_FONT_SIZE) and
// `Label::natural_width_at` (measures at whatever font_size is passed in) - a
// scratch buffer, discarded after reading back its shaped, unwrapped line width,
// so this never disturbs the caller's own buffer/shaping state. Width is left
// unbounded (`None`) rather than passed through from the real box - wrapping is
// off (`Wrap::None`) regardless, but an unbounded width is what actually reports
// the text's true natural size instead of whatever the (possibly wrong, that's
// the point of calling this) current box width would clip it to.
fn natural_line_width(font_system: &mut FontSystem, text: &str, font_size: f32) -> f32 {
    let mut buffer = Buffer::new(font_system, Metrics::new(font_size, font_size * LINE_HEIGHT_RATIO));
    buffer.set_size(font_system, None, Some(font_size * LINE_HEIGHT_RATIO));
    buffer.set_text(font_system, text, &Attrs::new().family(BASE_FONT), Shaping::Advanced);
    buffer.set_wrap(font_system, glyphon::Wrap::None);
    buffer.shape_until_scroll(font_system, true);
    buffer.layout_runs().fold(0.0f32, |w, run| run.line_w.max(w))
}

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
            None => natural_line_width(font_system, text, DEFAULT_FONT_SIZE),
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

    /// This label's own natural (unwrapped) line width if it were shaped at
    /// `font_size` instead of whatever it's currently using - see
    /// `UiNode::set_font_size`, which grows the node's box to at least this
    /// whenever the box is currently narrower (e.g. still sized for whatever
    /// font_size/width it had before, most commonly `DEFAULT_FONT_SIZE`'s natural
    /// width from `measure_or`). Doesn't touch `self.buffer` - see
    /// `natural_line_width`.
    pub fn natural_width_at(&self, font_system: &mut FontSystem, font_size: f32) -> f32 {
        natural_line_width(font_system, &self.text, font_size)
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
    /// hover-overridden - see `UiNode::node_content_preparation`). `vertices_slice`/
    /// `indices_slice` are plain slices, not fixed-size arrays - a node's own
    /// background/border quad isn't always exactly 4 vertices/6 indices any more,
    /// see `UiNode::compute_quad`.
    pub fn ui_node_data_creation(&self, _size: &Size, dpi_scale: f32, vertices: &mut Vec<VertexUi>, vertices_slice: &[VertexUi], indices: &mut Vec<u16>, indices_slice: &[u16], parent_rect: &Rect, color: Color, clip_rect: Option<&Rect>) -> (TextArea, u16, u32) {
        vertices.extend_from_slice(vertices_slice);
        indices.extend_from_slice(indices_slice);

        (self.text_area(parent_rect, dpi_scale, color, clip_rect), vertices_slice.len() as u16, indices_slice.len() as u32)
    }

    // `dpi_scale` (pixel_size / points size, see WindowManager) - everything
    // else in this codebase (UiTransform/Rect, mouse position, this label's own
    // parent_rect) is in points, but glyphon's Resolution/TextArea coordinate
    // space is real backing pixels (see glyphon::Resolution's own doc comment:
    // "the width/height of the screen in pixels") - left/top/bounds have to be
    // converted to that space here, and `scale` set to the same factor so
    // glyphs actually rasterize at the display's real pixel density instead of
    // just being scaled-up points-resolution glyphs (blurry on a Retina
    // display). Without this, text renders squeezed into whatever fraction of
    // the real screen points-space is of pixel-space (e.g. a quarter of the
    // screen, top-left, at a typical 2x HiDPI scale).
    pub fn text_area(&self, parent_rect: &Rect, dpi_scale: f32, color: Color, clip_rect: Option<&Rect>) -> TextArea {
        let text_width = self.get_text_width();
        let TextWidth { width, buffer_width } = text_width;

        let text_overlap = if width > buffer_width {
            width - buffer_width
        } else {
            0.0
        };

        TextArea {
            buffer: &self.buffer,
            left: (parent_rect.left - text_overlap) * dpi_scale,
            top: self.vertical_positioning_in_rect(parent_rect) * dpi_scale,
            scale: dpi_scale,
            bounds: self.bounds(parent_rect, clip_rect, dpi_scale),
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

    // Intersected with clip_rect (this label's tightest scrollable ancestor's
    // own content rect, if any - see UiNode::set_scrollable/
    // node_content_preparation's clip_rect) rather than just this label's own
    // rect - glyphon already clips every glyph to TextBounds on its own, so a
    // scrolled-out label's text needs no shader-side work, unlike the
    // background/border quads (see text_shader.wgsl's clip discard).
    fn bounds(&self, rect: &Rect, clip_rect: Option<&Rect>, dpi_scale: f32) -> TextBounds {
        let left = rect.left.max(clip_rect.map_or(f32::MIN, |c| c.left));
        let top = rect.top.max(clip_rect.map_or(f32::MIN, |c| c.top));
        let right = rect.right.min(clip_rect.map_or(f32::MAX, |c| c.right));
        let bottom = rect.bottom.min(clip_rect.map_or(f32::MAX, |c| c.bottom));
        TextBounds {
            left: (left * dpi_scale) as i32,
            top: (top * dpi_scale) as i32,
            right: (right * dpi_scale) as i32,
            bottom: (bottom * dpi_scale) as i32,
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
