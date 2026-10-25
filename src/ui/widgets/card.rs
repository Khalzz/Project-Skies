//! A panel that groups other nodes - a rounded, padded box that stacks its
//! children and grows to fit them.
//!
//!     card(&CardStyle::default())
//!         .set_child("title", title_label)
//!         .set_child("play", play_button)
//!
//! Like every widget, `CardStyle::default()` is the engine's base look - change
//! only the fields you want (`CardStyle { gap: 16.0, ..CardStyle::default() }`
//! or the builder methods).

use crate::ui::color::{Fill, UiColor};
use crate::ui::ui_node::UiNode;
use crate::ui::ui_transform::{Anchor, Orientation, Padding};

#[derive(Clone)]
pub struct CardStyle {
    pub background: Fill,
    pub border_color: Fill,
    /// In pixels - 0 for no border.
    pub border_width: f32,
    pub corner_radius: f32,
    /// Space between the card's edges and its children.
    pub padding: Padding,
    /// Space between one child and the next.
    pub gap: f32,
    /// Children stacked top to bottom (Vertical) or left to right.
    pub orientation: Orientation,
    /// Where the children sit across the card - Start (left/top), Center, End.
    pub align: Anchor,
    /// Blurs what's behind the card, in pixels - shows through a translucent
    /// background. 0: off.
    pub background_blur: f32,
}

impl Default for CardStyle {
    fn default() -> Self {
        Self {
            background: UiColor::Rgba(18, 18, 22, 235).into(),
            border_color: UiColor::Rgba(255, 255, 255, 200).into(),
            border_width: 1.0,
            corner_radius: 10.0,
            padding: Padding::all(16.0),
            gap: 8.0,
            orientation: Orientation::Vertical,
            align: Anchor::Start,
            background_blur: 0.0,
        }
    }
}

impl CardStyle {
    pub fn background(mut self, background: impl Into<Fill>) -> Self {
        self.background = background.into();
        self
    }

    pub fn border(mut self, color: impl Into<Fill>, width: f32) -> Self {
        self.border_color = color.into();
        self.border_width = width;
        self
    }

    pub fn corner_radius(mut self, radius: f32) -> Self {
        self.corner_radius = radius;
        self
    }

    /// `10.0` (every side), `(x, y)`, or `[top, bottom, left, right]`.
    pub fn padding(mut self, padding: impl Into<Padding>) -> Self {
        self.padding = padding.into();
        self
    }

    pub fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    pub fn orientation(mut self, orientation: Orientation) -> Self {
        self.orientation = orientation;
        self
    }

    pub fn align(mut self, align: Anchor) -> Self {
        self.align = align;
        self
    }

    pub fn background_blur(mut self, radius_px: f32) -> Self {
        self.background_blur = radius_px;
        self
    }
}

/// An empty card looking like `style` - add its contents with `.set_child`.
pub fn card(style: &CardStyle) -> UiNode {
    // The cross axis is the one `align` places children on; the stacking axis
    // starts at the top/left.
    let (horizontal, vertical) = match style.orientation {
        Orientation::Vertical => (style.align, Anchor::Start),
        Orientation::Horizontal => (Anchor::Start, style.align),
    };
    UiNode::container()
        .set_orientation(style.orientation)
        .set_child_anchor(horizontal, vertical)
        .set_padding(style.padding)
        .set_gap(style.gap)
        .set_background_color(style.background.clone())
        .set_border_color(style.border_color.clone())
        .set_border_width(style.border_width)
        .set_corner_radius(style.corner_radius)
        .set_background_blur(style.background_blur)
}
