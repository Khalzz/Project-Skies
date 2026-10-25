//! How text looks - its font (family and weight), size, color and alignment,
//! in one value every text-showing thing takes: a plain label
//! (`widgets::text`, `UiNode::set_text_style`), a button's text
//! (`ButtonStyle::text`)...
//!
//! A game defines its text once and reuses it:
//!
//!     let body = TextStyle::default().size(14.0);
//!     let heading = body.clone().size(22.0).weight(Weight::BOLD);
//!
//!     text(app, "Settings", &heading)
//!     ButtonStyle::default().text(|_| body.clone())

use glyphon::Weight;

use crate::ui::color::UiColor;
use crate::ui::components::label::{Font, DEFAULT_FONT_SIZE};
use crate::ui::Align;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    /// Family and weight - see `Font`. A named family must be loaded into the
    /// UI's font system, or the default sans-serif is used.
    pub font: Font,
    /// In pixels.
    pub size: f32,
    pub color: UiColor,
    /// Where the text sits across its box.
    pub align: Align,
}

impl Default for TextStyle {
    /// The engine's base text: the default sans-serif, regular weight, white,
    /// left-aligned, at the default size.
    fn default() -> Self {
        Self { font: Font::default(), size: DEFAULT_FONT_SIZE, color: UiColor::WHITE, align: Align::Left }
    }
}

impl TextStyle {
    pub fn font(mut self, font: Font) -> Self {
        self.font = font;
        self
    }

    /// The font family by name, e.g. "Inter".
    pub fn family(mut self, family: &'static str) -> Self {
        self.font.family = Some(family);
        self
    }

    /// `Weight::BOLD`, `Weight::LIGHT`, `Weight(600)`...
    pub fn weight(mut self, weight: Weight) -> Self {
        self.font.weight = weight;
        self
    }

    pub fn size(mut self, size: f32) -> Self {
        self.size = size;
        self
    }

    pub fn color(mut self, color: UiColor) -> Self {
        self.color = color;
        self
    }

    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }
}
