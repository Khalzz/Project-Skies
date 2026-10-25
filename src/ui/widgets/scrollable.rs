//! A box of fixed size whose children scroll (mouse wheel, while the mouse is
//! over it) when there are more than fit - top to bottom only.
//!
//!     let mut list = scrollable(&ScrollableStyle::default());
//!     for (i, name) in names.iter().enumerate() {
//!         list = list.set_child(format!("row{i}"), label(...));
//!     }
//!
//! It has to have a fixed height (the default is 240px) - one that grew to fit
//! its children would have nothing to scroll.

use crate::ui::color::{Fill, UiColor};
use crate::ui::components::container::Scrollbar;
use crate::ui::ui_node::UiNode;
use crate::ui::ui_transform::{Anchor, Orientation, Padding, SizeValue};

#[derive(Clone)]
pub struct ScrollableStyle {
    /// How wide it is - `Pixels`, or `Percent` of its parent (only when the
    /// parent has a size of its own, not one that fits its children).
    pub width: SizeValue,
    /// How tall the visible part is - see `width`.
    pub height: SizeValue,
    /// Space between its edges and the children. The scrollbar sits inside
    /// it, against the right side, and the children are kept clear of it.
    pub padding: Padding,
    /// Space between one child and the next.
    pub gap: f32,
    /// Where the children sit across it - Start (left), Center, End.
    pub align: Anchor,
    pub background: Fill,
    pub corner_radius: f32,
    pub scrollbar: Scrollbar,
}

impl Default for ScrollableStyle {
    fn default() -> Self {
        Self {
            width: SizeValue::Pixels(280.0),
            height: SizeValue::Pixels(240.0),
            padding: Padding::from([4.0, 4.0, 4.0, 4.0]),
            gap: 4.0,
            align: Anchor::Start,
            background: UiColor::Rgba(0, 0, 0, 60).into(),
            corner_radius: 6.0,
            scrollbar: Scrollbar::default(),
        }
    }
}

impl ScrollableStyle {
    pub fn size(mut self, width: SizeValue, height: SizeValue) -> Self {
        self.width = width;
        self.height = height;
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

    pub fn align(mut self, align: Anchor) -> Self {
        self.align = align;
        self
    }

    pub fn background(mut self, background: impl Into<Fill>) -> Self {
        self.background = background.into();
        self
    }

    pub fn corner_radius(mut self, radius: f32) -> Self {
        self.corner_radius = radius;
        self
    }

    pub fn scrollbar(mut self, scrollbar: Scrollbar) -> Self {
        self.scrollbar = scrollbar;
        self
    }
}

/// An empty scrollable box looking like `style` - add its contents with
/// `.set_child`.
pub fn scrollable(style: &ScrollableStyle) -> UiNode {
    UiNode::container()
        .set_orientation(Orientation::Vertical)
        .set_size(style.width, style.height)
        .set_child_anchor(style.align, Anchor::Start)
        .set_padding(style.padding)
        .set_gap(style.gap)
        .set_background_color(style.background.clone())
        .set_corner_radius(style.corner_radius)
        .set_scrollable(true)
        .set_scrollbar(style.scrollbar)
}
