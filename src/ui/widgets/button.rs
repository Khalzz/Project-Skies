//! A clickable text button.
//!
//!     button(app, "Play", &ButtonStyle::default()).on_click(|app| ...)
//!
//! `ButtonStyle::default()` is the engine's base look. A game makes its own
//! from it, changing only what it wants - with the builder methods, or struct
//! update syntax:
//!
//!     let primary = ButtonStyle::default()
//!         .background(UiColor::Rgb(46, 110, 220))
//!         .text(|text| text.size(16.0).weight(Weight::BOLD))
//!         .on_hover(|hover| hover.set_background_color(UiColor::Rgb(70, 135, 245)));
//!     let wide = ButtonStyle { width: Some(200.0), ..primary.clone() };
//!
//! It's a plain `UiNode` once built, so any `UiNode` builder call still works
//! on it afterwards (`.set_child`, `.active`, another `.on_hover`...).

use crate::app::App;
use crate::ui::color::{Fill, UiColor};
use crate::ui::text_style::TextStyle;
use crate::ui::ui_node::{Style, UiNode};
use crate::ui::{Align, CursorIcon};
use crate::ui::ui_transform::{Padding, SizeValue};

/// How a `button` looks - its resting look, plus what changes while it's
/// hovered and pressed.
#[derive(Clone)]
pub struct ButtonStyle {
    /// How its text looks at rest - the hover/press looks can still change
    /// its color.
    pub text: TextStyle,
    /// Space between the text and the button's edges.
    pub padding: Padding,
    /// A fixed width (padding included) - None: as wide as its text.
    pub width: Option<f32>,
    pub background: Fill,
    pub border_color: Fill,
    /// In pixels - 0 for no border.
    pub border_width: f32,
    pub corner_radius: f32,
    /// How long (ms) it takes to ease into its hover/press look and back.
    pub transition_ms: f32,
    /// What changes while the mouse is over it - anything it leaves unset
    /// stays as at rest.
    pub hover: Style,
    /// What changes while it's held down - over the hover look.
    pub press: Style,
    /// What changes while it's disabled (see `UiNode::set_enabled`) - over
    /// everything else.
    pub disabled: Style,
    /// The mouse cursor while it's over the button.
    pub cursor: CursorIcon,
}

impl Default for ButtonStyle {
    fn default() -> Self {
        Self {
            // At rest: just a white outline and white text, nothing filled in.
            text: TextStyle::default().size(13.0).color(UiColor::WHITE).align(Align::Center),
            padding: Padding::from((2.0, 2.0)),
            width: None,
            background: UiColor::TRANSPARENT.into(),
            border_color: UiColor::WHITE.into(),
            border_width: 1.0,
            corner_radius: 4.0,
            transition_ms: 80.0,
            // Hovered: filled white, the text dark on it.
            hover: Style::default()
                .set_background_color(UiColor::WHITE)
                .set_border_color(UiColor::WHITE)
                .set_text_color(UiColor::Rgb(20, 20, 20)),
            // Held down: a little greyer.
            press: Style::default()
                .set_background_color(UiColor::Rgb(205, 205, 205))
                .set_border_color(UiColor::Rgb(205, 205, 205)),
            // Disabled: faded, nothing filled in.
            disabled: disabled_look(),
            cursor: CursorIcon::Pointer,
        }
    }
}

/// The faded look the widgets use while disabled - faint text and border,
/// no fill.
pub(crate) fn disabled_look() -> Style {
    let faint = UiColor::Rgba(255, 255, 255, 60);
    Style::default()
        .set_text_color(faint)
        .set_border_color(faint)
        .set_background_color(UiColor::TRANSPARENT)
}

impl ButtonStyle {
    /// Changes the disabled look - same as `on_hover`.
    pub fn on_disabled(mut self, build: impl FnOnce(Style) -> Style) -> Self {
        self.disabled = build(self.disabled);
        self
    }

    /// Changes how the text looks - `build` gets the current `TextStyle`, so
    /// it can change just a field (`|text| text.size(16.0)`) or use one the
    /// game already has (`|_| heading`).
    pub fn text(mut self, build: impl FnOnce(TextStyle) -> TextStyle) -> Self {
        self.text = build(self.text);
        self
    }

    /// `10.0` (every side), `(x, y)`, or `[top, bottom, left, right]`.
    pub fn padding(mut self, padding: impl Into<Padding>) -> Self {
        self.padding = padding.into();
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    /// A plain `UiColor` or a gradient (`Fill`).
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

    pub fn transition(mut self, ms: f32) -> Self {
        self.transition_ms = ms;
        self
    }

    /// Changes the hover look - `build` gets the current one, so it can change
    /// just a field (`|hover| hover.set_text_color(...)`) or start over
    /// (`|_| Style::default()...`).
    pub fn on_hover(mut self, build: impl FnOnce(Style) -> Style) -> Self {
        self.hover = build(self.hover);
        self
    }

    /// Changes the pressed look - same as `on_hover`.
    pub fn on_press(mut self, build: impl FnOnce(Style) -> Style) -> Self {
        self.press = build(self.press);
        self
    }

    /// The mouse cursor while it's over the button - `Pointer` by default.
    pub fn cursor(mut self, cursor: CursorIcon) -> Self {
        self.cursor = cursor;
        self
    }
}

/// A button showing `text`, looking like `style` - chain `.on_click(...)` on
/// it for what it does.
pub fn button(app: &mut App, text: &str, style: &ButtonStyle) -> UiNode {
    let font_system = &mut app.ui.text.font_system;
    let hover = style.hover.clone();
    let press = style.press.clone();
    let disabled = style.disabled.clone();
    // Padding after the text style, so it grows the box around the text's
    // real size.
    let node = UiNode::label(font_system, text, None, None)
        .set_text_style(font_system, &style.text)
        .set_padding(style.padding)
        .set_background_color(style.background.clone())
        .set_border_color(style.border_color.clone())
        .set_border_width(style.border_width)
        .set_corner_radius(style.corner_radius)
        .set_transition(style.transition_ms)
        .on_hover(|_| hover)
        .on_press(|_| press)
        .on_disabled(|_| disabled)
        .set_cursor(style.cursor);
    match style.width {
        Some(width) => node.set_size(SizeValue::Pixels(width), SizeValue::Fit),
        None => node,
    }
}
