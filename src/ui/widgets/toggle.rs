//! An on/off switch - a pill-shaped track with a round knob at its left end
//! (off) or right end (on).
//!
//!     toggle(true, &ToggleStyle::default())
//!         .on_toggle(|app, on| app.renderer.set_vsync(on))
//!
//! Clicking it flips it; `on_toggle` runs with the new value. From code:
//! `node.toggle_value()` / `node.set_toggle_value(on)`.

use std::cell::Cell;
use std::rc::Rc;

use crate::app::App;
use crate::ui::color::UiColor;
use crate::ui::ui_node::{UiNode, UiNodeContent};
use crate::ui::ui_transform::{Anchor, SizeValue};
use crate::ui::CursorIcon;

/// How a `toggle` looks.
#[derive(Clone, Copy)]
pub struct ToggleStyle {
    /// The track's size, in pixels - the knob is as tall as it, less the
    /// padding.
    pub width: f32,
    pub height: f32,
    /// Space between the track's edge and the knob.
    pub padding: f32,
    pub track_off: UiColor,
    pub track_on: UiColor,
    pub knob_off: UiColor,
    pub knob_on: UiColor,
    pub border_color: UiColor,
    /// In pixels - 0 for no border.
    pub border_width: f32,
    /// How long (ms) its colors take to change when it flips.
    pub transition_ms: f32,
}

impl Default for ToggleStyle {
    /// Faint at rest, lit near-white when on.
    fn default() -> Self {
        Self {
            width: 44.0,
            height: 24.0,
            padding: 3.0,
            track_off: UiColor::Rgba(255, 255, 255, 28),
            track_on: UiColor::Rgba(255, 255, 255, 225),
            knob_off: UiColor::Rgb(220, 220, 220),
            knob_on: UiColor::Rgb(20, 20, 20),
            border_color: UiColor::Rgba(255, 255, 255, 45),
            border_width: 1.0,
            transition_ms: 120.0,
        }
    }
}

impl ToggleStyle {
    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    pub fn track(mut self, off: UiColor, on: UiColor) -> Self {
        self.track_off = off;
        self.track_on = on;
        self
    }

    pub fn knob(mut self, off: UiColor, on: UiColor) -> Self {
        self.knob_off = off;
        self.knob_on = on;
        self
    }

    pub fn border(mut self, color: UiColor, width: f32) -> Self {
        self.border_color = color;
        self.border_width = width;
        self
    }
}

/// A switch starting `on` or off, looking like `style`.
pub fn toggle(on: bool, style: &ToggleStyle) -> UiNode {
    let knob_size = (style.height - style.padding * 2.0).max(1.0);
    let state = ToggleState { on: Rc::new(Cell::new(on)), style: *style };
    let flipped = Rc::clone(&state.on);
    UiNode::container()
        .set_size(SizeValue::Pixels(style.width), SizeValue::Pixels(style.height))
        .set_padding(style.padding)
        .set_corner_radius(style.height / 2.0)
        .set_border_width(style.border_width)
        .set_border_color(style.border_color)
        .set_transition(style.transition_ms)
        .set_cursor(CursorIcon::Pointer)
        // Disabled: a faint outline, a grey knob - whichever way it's set.
        .on_disabled(|disabled| disabled
            .set_background_color(UiColor::Rgba(255, 255, 255, 12))
            .set_border_color(UiColor::Rgba(255, 255, 255, 30)))
        .set_child("knob", UiNode::container()
            .set_size(SizeValue::Pixels(knob_size), SizeValue::Pixels(knob_size))
            .set_corner_radius(knob_size / 2.0)
            .set_transition(style.transition_ms)
            .on_disabled(|disabled| disabled.set_background_color(UiColor::Rgba(255, 255, 255, 70))))
        // Flips on its own - `on_toggle` replaces this with a flip that also
        // tells the game.
        .on_click(move |_| flipped.set(!flipped.get()))
        .with_toggle(state)
}

/// What a toggle holds - kept on its `UiNode` (see `UiNode::toggle_value`).
/// `on` is shared with its click handler, which flips it.
pub struct ToggleState {
    pub(crate) on: Rc<Cell<bool>>,
    style: ToggleStyle,
}

impl ToggleState {
    /// Its look for its value - every frame (see UiNode::node_content_preparation):
    /// the knob at the matching end, the track's and knob's colors (eased by
    /// their transitions).
    pub(crate) fn sync(&self, node_anchor: &mut Anchor, background: &mut Option<crate::ui::color::Fill>, content: &mut UiNodeContent) {
        let on = self.on.get();
        *node_anchor = if on { Anchor::End } else { Anchor::Start };
        *background = Some((if on { self.style.track_on } else { self.style.track_off }).into());
        let UiNodeContent::Container(container) = content else { return };
        if let Some((_, knob)) = container.children.iter_mut().find(|(id, _)| id == "knob") {
            knob.style.background_color = Some((if on { self.style.knob_on } else { self.style.knob_off }).into());
        }
    }
}

impl UiNode {
    /// Runs `f` with the new value whenever this toggle (see `toggle`) is
    /// clicked - no-op on anything else.
    pub fn on_toggle(self, mut f: impl FnMut(&mut App, bool) + 'static) -> Self {
        let Some(on) = self.toggle_cell() else { return self };
        self.on_click(move |app| {
            let flipped = !on.get();
            on.set(flipped);
            f(app, flipped);
        })
    }
}
