//! A one-line box to type text into - a name, a seed, a server address.
//!
//!     let name = text_input(app, "Player name", &TextInputStyle::default());
//!     Layer::new(app).set_child("name", name).build(app);
//!
//!     // Later - what's been typed:
//!     let value = Ui::get_ui_node(&mut app.ui.renderizable_elements, "name")
//!         .and_then(|node| node.input_value());
//!
//! Click it to type (the border lights up and a caret blinks); click
//! anywhere else, or press Enter or Escape, to stop. Backspace, Delete, the
//! arrows, Home and End edit as usual. Drag across the text (or hold Shift
//! with the arrows, Home, End) to select it, Ctrl+A for all of it; Ctrl+C,
//! Ctrl+X and Ctrl+V copy, cut and paste. Text longer than the box scrolls
//! so its end stays in view.
//!
//! While one has focus, `app.ui.ui_rendering.text_input_focused` is true - a
//! game should ignore its own keyboard controls then, so typing "w" in a name
//! doesn't also walk forward.

use glyphon::FontSystem;

use crate::app::App;
use crate::input::clipboard::{clipboard, set_clipboard};
use crate::input::input::{self, TypedInput};
use crate::ui::color::{Fill, UiColor};
use crate::ui::components::label::{text_width, Font, DEFAULT_FONT_SIZE};
use crate::ui::text_style::TextStyle;
use crate::ui::ui_node::{Style, UiNode, UiNodeContent};
use crate::ui::ui_transform::{Padding, PositionValue, SizeValue};
use crate::ui::CursorIcon;

/// How long the caret stays on, then off (seconds).
const CARET_BLINK: f32 = 0.5;
/// The caret's width in pixels.
const CARET_WIDTH: f32 = 1.5;

/// How a `text_input` looks.
#[derive(Clone)]
pub struct TextInputStyle {
    /// Its width, padding included - the height fits the text.
    pub width: f32,
    /// How the typed text looks.
    pub text: TextStyle,
    /// The color of the placeholder, shown while it's empty.
    pub placeholder_color: UiColor,
    pub padding: Padding,
    pub background: Fill,
    /// Blurs what's behind it, in pixels. 0: off.
    pub background_blur: f32,
    pub border_color: Fill,
    /// The border while it has focus.
    pub focus_border_color: Fill,
    /// In pixels - 0 for no border.
    pub border_width: f32,
    pub corner_radius: f32,
    pub caret_color: UiColor,
    /// Behind the selected text.
    pub selection_color: UiColor,
    /// The most characters it takes - None: no limit.
    pub max_length: Option<usize>,
}

impl Default for TextInputStyle {
    fn default() -> Self {
        Self {
            width: 220.0,
            text: TextStyle::default().size(14.0),
            placeholder_color: UiColor::Rgba(255, 255, 255, 110),
            padding: Padding::from((8.0, 2.0)),
            background: UiColor::Rgba(0, 0, 0, 90).into(),
            background_blur: 0.0,
            border_color: UiColor::Rgba(255, 255, 255, 60).into(),
            focus_border_color: UiColor::WHITE.into(),
            border_width: 1.0,
            corner_radius: 4.0,
            caret_color: UiColor::WHITE,
            selection_color: UiColor::Rgba(90, 150, 255, 120),
            max_length: None,
        }
    }
}

impl TextInputStyle {
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub fn text(mut self, build: impl FnOnce(TextStyle) -> TextStyle) -> Self {
        self.text = build(self.text);
        self
    }

    pub fn placeholder_color(mut self, color: UiColor) -> Self {
        self.placeholder_color = color;
        self
    }

    /// `10.0` (every side), `(x, y)`, or `[top, bottom, left, right]`.
    pub fn padding(mut self, padding: impl Into<Padding>) -> Self {
        self.padding = padding.into();
        self
    }

    pub fn background(mut self, background: impl Into<Fill>) -> Self {
        self.background = background.into();
        self
    }

    pub fn background_blur(mut self, radius_px: f32) -> Self {
        self.background_blur = radius_px;
        self
    }

    pub fn border(mut self, color: impl Into<Fill>, width: f32) -> Self {
        self.border_color = color.into();
        self.border_width = width;
        self
    }

    pub fn focus_border(mut self, color: impl Into<Fill>) -> Self {
        self.focus_border_color = color.into();
        self
    }

    pub fn corner_radius(mut self, radius: f32) -> Self {
        self.corner_radius = radius;
        self
    }

    pub fn caret_color(mut self, color: UiColor) -> Self {
        self.caret_color = color;
        self
    }

    pub fn selection_color(mut self, color: UiColor) -> Self {
        self.selection_color = color;
        self
    }

    pub fn max_length(mut self, max_length: usize) -> Self {
        self.max_length = Some(max_length);
        self
    }
}

/// An empty text input showing `placeholder`, looking like `style`.
pub fn text_input(app: &mut App, placeholder: &str, style: &TextInputStyle) -> UiNode {
    let font_system = &mut app.ui.text.font_system;
    let label = UiNode::label(font_system, placeholder, None, None)
        .set_text_style(font_system, &style.text)
        .set_text_color(style.placeholder_color)
        .set_size(SizeValue::Grow, SizeValue::Fit)
        .on_disabled(|disabled| disabled.set_text_color(UiColor::Rgba(255, 255, 255, 60)));
    let caret_height = style.text.size * 1.2;
    let caret = UiNode::container()
        .set_size(SizeValue::Pixels(CARET_WIDTH), SizeValue::Pixels(caret_height))
        .set_position(PositionValue::Start(0.0), PositionValue::Center(0.0))
        .set_background_color(style.caret_color)
        .active(false);
    // Behind the selected text - placed and sized every frame (see `sync`).
    let selection = UiNode::container()
        .set_size(SizeValue::Pixels(1.0), SizeValue::Pixels(caret_height))
        .set_position(PositionValue::Start(0.0), PositionValue::Center(0.0))
        .set_background_color(style.selection_color)
        .active(false);

    let state = TextInputState {
        value: String::new(),
        caret: 0,
        anchor: None,
        dragging: false,
        placeholder: placeholder.to_owned(),
        max_length: style.max_length,
        text_color: style.text.color,
        placeholder_color: style.placeholder_color,
        focus: Style::default().set_border_color(style.focus_border_color.clone()),
        focused: false,
        blink: 0.0,
    };

    UiNode::container()
        .set_size(SizeValue::Pixels(style.width), SizeValue::Fit)
        .set_padding(style.padding)
        .set_background_color(style.background.clone())
        .set_background_blur(style.background_blur)
        .set_border_color(style.border_color.clone())
        .set_border_width(style.border_width)
        .set_corner_radius(style.corner_radius)
        .set_clip_children(true)
        .set_cursor(CursorIcon::Text)
        .on_disabled(|disabled| disabled.set_border_color(UiColor::Rgba(255, 255, 255, 30)))
        .set_child("selection", selection)
        .set_child("text", label)
        .set_child("caret", caret)
        .with_text_input(state)
}

/// What a text input holds - kept on its `UiNode` (see `UiNode::input_value`),
/// updated every frame in its layout pass.
pub struct TextInputState {
    pub(crate) value: String,
    /// Where the caret is, in characters from the start.
    caret: usize,
    /// Where the selection started - it runs from here to `caret`. None (or
    /// the same as `caret`): nothing selected.
    anchor: Option<usize>,
    /// The left button went down on it and is still held - moving selects.
    dragging: bool,
    placeholder: String,
    max_length: Option<usize>,
    text_color: UiColor,
    placeholder_color: UiColor,
    /// Layered over its style while it has focus.
    pub(crate) focus: Style,
    pub(crate) focused: bool,
    /// Seconds since the caret last restarted its blink.
    blink: f32,
}

impl TextInputState {
    /// Replaces the text, the caret going to its end.
    pub(crate) fn set_value(&mut self, value: &str) {
        self.value = match self.max_length {
            Some(max) => value.chars().take(max).collect(),
            None => value.to_owned(),
        };
        self.caret = self.value.chars().count();
        self.anchor = None;
    }

    /// One frame: focus and the mouse (`clicked_inside`: the left button went
    /// down over the box), this frame's typing if it has focus, then its text,
    /// caret and selection into its children. `mouse_x`: the mouse from where
    /// the text starts; `text_room`: how wide the text's room is - both in UI
    /// units.
    pub(crate) fn update(&mut self, content: &mut UiNodeContent, font_system: &mut FontSystem, clicked_inside: bool, delta_time: f32, mouse_x: f32, text_room: f32) {
        let Some(metrics) = TextMetrics::of(content) else { return };

        if input::mouse_left_button_just_pressed() {
            if clicked_inside {
                let at = self.char_at_x(font_system, &metrics, mouse_x, text_room);
                let extend = input::shift_down() && self.focused;
                if !extend {
                    self.anchor = Some(at);
                }
                self.caret = at;
                self.dragging = true;
                self.blink = 0.0;
            }
            self.focused = clicked_inside;
        }
        if self.dragging {
            if input::mouse_left_button_down() {
                self.caret = self.char_at_x(font_system, &metrics, mouse_x, text_room);
            } else {
                self.dragging = false;
            }
        }

        if self.focused {
            self.blink += delta_time;
            for typed in input::typed_input() {
                if self.handle_typing(typed) {
                    // Typing or moving shows the caret straight away.
                    self.blink = 0.0;
                }
            }
        } else {
            self.anchor = None;
            self.dragging = false;
        }

        self.sync(content, font_system, &metrics, text_room);
    }

    /// One key or piece of text - true if it did something.
    fn handle_typing(&mut self, typed: TypedInput) -> bool {
        use sdl2::keyboard::Keycode;
        match typed {
            TypedInput::Text(text) => {
                let text: String = text.chars().filter(|c| !c.is_control()).collect();
                self.insert(&text);
            }
            TypedInput::Key { key, pressed: true, ctrl, shift, .. } => {
                let length = self.value.chars().count();
                match key {
                    Keycode::A if ctrl => {
                        self.anchor = Some(0);
                        self.caret = length;
                    }
                    Keycode::C if ctrl => {
                        if let Some(selected) = self.selected_text() {
                            set_clipboard(selected);
                        }
                    }
                    Keycode::X if ctrl => {
                        if let Some(selected) = self.selected_text() {
                            set_clipboard(selected);
                            self.delete_selection();
                        }
                    }
                    Keycode::V if ctrl => {
                        if let Some(pasted) = clipboard() {
                            // One line - a pasted line break would be invisible.
                            let pasted: String = pasted.chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c }).filter(|c| !c.is_control()).collect();
                            self.insert(&pasted);
                        }
                    }
                    Keycode::Backspace => {
                        if !self.delete_selection() && self.caret > 0 {
                            self.caret -= 1;
                            let at = self.byte_at(self.caret);
                            self.value.remove(at);
                        }
                    }
                    Keycode::Delete => {
                        if !self.delete_selection() && self.caret < length {
                            let at = self.byte_at(self.caret);
                            self.value.remove(at);
                        }
                    }
                    Keycode::Left | Keycode::Right | Keycode::Home | Keycode::End => {
                        let target = match key {
                            // Without Shift, an arrow on a selection goes to
                            // its edge on that side.
                            Keycode::Left if !shift && self.selection().is_some() => self.selection().unwrap().0,
                            Keycode::Right if !shift && self.selection().is_some() => self.selection().unwrap().1,
                            Keycode::Left => self.caret.saturating_sub(1),
                            Keycode::Right => (self.caret + 1).min(length),
                            Keycode::Home => 0,
                            _ => length,
                        };
                        if shift {
                            self.anchor.get_or_insert(self.caret);
                        } else {
                            self.anchor = None;
                        }
                        self.caret = target;
                    }
                    Keycode::Return | Keycode::KpEnter | Keycode::Escape => self.focused = false,
                    _ => return false,
                }
            }
            _ => return false,
        }
        true
    }

    /// The selected characters' range (start, end), if any are.
    fn selection(&self) -> Option<(usize, usize)> {
        let anchor = self.anchor?;
        (anchor != self.caret).then(|| (anchor.min(self.caret), anchor.max(self.caret)))
    }

    fn selected_text(&self) -> Option<&str> {
        let (start, end) = self.selection()?;
        Some(&self.value[self.byte_at(start)..self.byte_at(end)])
    }

    /// Removes the selected text, the caret where it was - false if nothing
    /// was selected.
    fn delete_selection(&mut self) -> bool {
        let Some((start, end)) = self.selection() else { return false };
        let range = self.byte_at(start)..self.byte_at(end);
        self.value.replace_range(range, "");
        self.caret = start;
        self.anchor = None;
        true
    }

    /// Types `text` at the caret, over the selection if there is one - as
    /// much of it as `max_length` leaves room for.
    fn insert(&mut self, text: &str) {
        self.delete_selection();
        self.anchor = None;
        let room = self.max_length.map_or(usize::MAX, |max| max.saturating_sub(self.value.chars().count()));
        let text: String = text.chars().take(room).collect();
        let at = self.byte_at(self.caret);
        self.value.insert_str(at, &text);
        self.caret += text.chars().count();
    }

    /// How far the text is drawn shifted left - a text longer than its box
    /// shows its end (see Label::text_area).
    fn shift(&self, font_system: &mut FontSystem, metrics: &TextMetrics, text_room: f32) -> f32 {
        (text_width(font_system, &self.value, metrics.size, metrics.font) - text_room).max(0.0)
    }

    /// Where the caret goes for a click `x` pixels from the text's left edge
    /// - the character boundary nearest it.
    fn char_at_x(&self, font_system: &mut FontSystem, metrics: &TextMetrics, x: f32, text_room: f32) -> usize {
        let x = x + self.shift(font_system, metrics, text_room);
        let mut previous = 0.0;
        for (index, (at, character)) in self.value.char_indices().enumerate() {
            let next = text_width(font_system, &self.value[..at + character.len_utf8()], metrics.size, metrics.font);
            if x < (previous + next) / 2.0 {
                return index;
            }
            previous = next;
        }
        self.value.chars().count()
    }

    /// The x (from the text's left edge, after its shift) of the boundary
    /// before character `index`.
    fn x_of(&self, font_system: &mut FontSystem, metrics: &TextMetrics, index: usize, shift: f32) -> f32 {
        text_width(font_system, &self.value[..self.byte_at(index)], metrics.size, metrics.font) - shift
    }

    /// Puts the text (or the placeholder) into the "text" label, and the
    /// caret and the selection where they belong.
    fn sync(&self, content: &mut UiNodeContent, font_system: &mut FontSystem, metrics: &TextMetrics, text_room: f32) {
        let UiNodeContent::Container(container) = content else { return };
        let empty = self.value.is_empty();
        let shift = if self.focused && !empty { self.shift(font_system, metrics, text_room) } else { 0.0 };
        let caret_x = if self.focused && !empty {
            self.x_of(font_system, metrics, self.caret, shift).clamp(0.0, (text_room - CARET_WIDTH).max(0.0))
        } else {
            0.0
        };
        let selection = self.selection().filter(|_| self.focused).map(|(start, end)| {
            let left = self.x_of(font_system, metrics, start, shift).max(0.0);
            let right = self.x_of(font_system, metrics, end, shift).min(text_room);
            (left, (right - left).max(0.0))
        });
        let caret_on = self.focused && self.blink % (CARET_BLINK * 2.0) < CARET_BLINK;

        for (id, child) in &mut container.children {
            match id.as_str() {
                "text" => {
                    child.style.text_color = Some(if empty { self.placeholder_color } else { self.text_color });
                    if let Some(label) = child.as_label_mut() {
                        label.set_text(font_system, if empty { &self.placeholder } else { &self.value }, false);
                    }
                }
                "caret" => {
                    child.set_active(caret_on);
                    child.set_pending_position(PositionValue::Start(caret_x), PositionValue::Center(0.0));
                }
                "selection" => {
                    child.set_active(selection.is_some());
                    if let Some((left, width)) = selection {
                        child.transform.width = width.max(1.0);
                        child.set_pending_position(PositionValue::Start(left), PositionValue::Center(0.0));
                    }
                }
                _ => {}
            }
        }
    }

    /// The byte offset of character `index` in `value`.
    fn byte_at(&self, index: usize) -> usize {
        self.value.char_indices().nth(index).map_or(self.value.len(), |(at, _)| at)
    }
}

/// The size and font the input's text is drawn with - read from its "text"
/// label.
struct TextMetrics {
    size: f32,
    font: Font,
}

impl TextMetrics {
    fn of(content: &mut UiNodeContent) -> Option<Self> {
        let UiNodeContent::Container(container) = content else { return None };
        let (_, label) = container.children.iter_mut().find(|(id, _)| id == "text")?;
        let size = label.style.font_size.unwrap_or(DEFAULT_FONT_SIZE);
        let font = label.as_label_mut()?.font();
        Some(Self { size, font })
    }
}

