//! A dialog over everything else: a screen-wide backdrop dimming what's
//! behind, with a card in the middle. While it's open nothing behind it can be
//! hovered or clicked - only the modal itself.
//!
//!     let content = UiNode::container()
//!         .set_child("message", label)
//!         .set_child("ok", button(app, "OK", &ButtonStyle::default())
//!             .on_click(|app| close_modal(app, "confirm")));
//!     open_modal(app, "confirm", modal(content, &ModalStyle::default()));
//!
//! Its nodes are found under its id like any other UI: "confirm/card/content/ok".

use crate::app::App;
use crate::ui::color::{Fill, UiColor};
use crate::ui::ui_node::UiNode;
use crate::ui::ui_transform::{Anchor, PositionValue, SizeValue};
use crate::ui::widgets::card::{card, CardStyle};

#[derive(Clone)]
pub struct ModalStyle {
    /// Covers the whole screen behind the card - usually a translucent dark.
    pub backdrop: Fill,
    /// Blurs what's behind the backdrop, in pixels. 0: off.
    pub backdrop_blur: f32,
    /// The box in the middle holding the content.
    pub card: CardStyle,
    /// About how long (ms) the backdrop's dimming and blur take to fade in
    /// when it opens. 0: shown at once.
    pub fade_in_ms: f32,
}

impl Default for ModalStyle {
    fn default() -> Self {
        Self {
            backdrop: UiColor::Rgba(0, 0, 0, 200).into(),
            backdrop_blur: 15.0,
            card: CardStyle::default().padding(20.0).gap(14.0),
            fade_in_ms: 200.0,
        }
    }
}

impl ModalStyle {
    pub fn backdrop(mut self, backdrop: impl Into<Fill>) -> Self {
        self.backdrop = backdrop.into();
        self
    }

    pub fn backdrop_blur(mut self, radius_px: f32) -> Self {
        self.backdrop_blur = radius_px;
        self
    }

    pub fn card(mut self, card: CardStyle) -> Self {
        self.card = card;
        self
    }

    pub fn fade_in(mut self, ms: f32) -> Self {
        self.fade_in_ms = ms;
        self
    }
}

/// `content` in a card in the middle of a screen-wide backdrop, looking like
/// `style` - show it with `open_modal`.
pub fn modal(content: UiNode, style: &ModalStyle) -> UiNode {
    let backdrop = UiNode::container()
        .set_size(SizeValue::Percent(100.0), SizeValue::Percent(100.0))
        .set_position(PositionValue::Start(0.0), PositionValue::Start(0.0))
        .set_child_anchor(Anchor::Center, Anchor::Center)
        .set_background_color(style.backdrop.clone())
        .set_background_blur(style.backdrop_blur)
        .set_child("card", card(&style.card).set_child("content", content));
    if style.fade_in_ms <= 0.0 {
        return backdrop;
    }
    // Starts clear and sharp, and eases into the dimmed, blurred look.
    backdrop
        .set_transition(style.fade_in_ms)
        .set_enter(|enter| enter.set_background_color(UiColor::TRANSPARENT).set_background_blur(0.0))
}

/// Shows `modal` (see `modal`) over everything, under `id` - replacing one
/// already open under the same id.
pub fn open_modal(app: &mut App, id: &str, mut modal: UiNode) {
    let (width, height) = app.ui.size();
    modal.resolve(width, height);
    app.ui.add_to_ui(id.to_owned(), modal);
    if !app.ui.always_on_top.iter().any(|key| key == id) {
        app.ui.always_on_top.push(id.to_owned());
    }
    app.ui.has_changed = true;
}

/// Closes the modal open under `id`, if there is one - e.g. from one of its
/// own buttons' `on_click`.
pub fn close_modal(app: &mut App, id: &str) {
    app.ui.renderizable_elements.remove(id);
    app.ui.always_on_top.retain(|key| key != id);
    app.ui.has_changed = true;
}

pub fn is_modal_open(app: &App, id: &str) -> bool {
    app.ui.always_on_top.iter().any(|key| key == id) && app.ui.renderizable_elements.contains_key(id)
}
