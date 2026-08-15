//! A small modal for capturing a single key/button/axis binding, built on the
//! existing input-capture system (see `engine::input::input`'s
//! `begin_capture`/`captured_binding`/`cancel_capture` - this module is the
//! "future rebinding menu" that doc comment already anticipated).
//!
//! Opened via `open(app, action, edit_index)` from a settings chip or its "+"
//! button (see `main_menu::ui::controller_chip_nodes`) - `edit_index: Some(i)`
//! replaces that action's existing binding `i` in place (and shows "Delete
//! Bind"); `None` appends a brand new one instead (nothing to delete yet, so
//! that button stays hidden).
//!
//! Opening the modal does NOT start capturing by itself - it shows a "Bind"
//! button/chip (see `BIND_PATH`) displaying either the existing binding
//! (`edit_index: Some`) or a "Click here to bind" placeholder (`None`).
//! Clicking THAT is what calls `input::begin_capture()` (see `start_capture`).
//! This one extra step matters: capturing intercepts every raw input event,
//! including the mouse click that would otherwise land on Cancel (see
//! `InputSubsystem::update`'s `if self.capturing { ... continue; }` branch,
//! which swallows that click as "the captured binding" instead of letting it
//! reach the UI's normal click handling) - auto-capturing the instant the
//! modal opened meant clicking Cancel right away registered as a Mouse Left
//! binding first and only closed the modal on the click *after* that. Leaving
//! capturing off until the player deliberately opts into it keeps Cancel (and
//! every other modal button) safely clickable the rest of the time.
//!
//! Capturing only *stages* a binding (in `RebindModalState::captured`) - it's
//! not written back to the input settings until "Save" is clicked (see
//! `save_current`). "Cancel" (or clicking a different chip, or closing the
//! modal any other way) discards the staged binding instead, leaving whatever
//! was bound before untouched. `InputSubsystem`'s own capture already stops
//! listening for further input the instant one binding comes in (see
//! `capturing = false` in `InputSubsystem::update`), so `update(app)` only
//! needs to poll while `RebindModalState::capturing` is true.

use std::sync::Mutex;

use crate::app::App;
use crate::engine::input::action::Binding;
use crate::engine::input::input;
use crate::engine::rendering::ui::ui::Ui;
use crate::engine::ui::color::UiColor;
use crate::engine::ui::ui_node::UiNode;
use crate::engine::ui::ui_transform::{Anchor, Orientation, SizeValue};
use crate::game::ui::{button, card, label};
use super::ui::{controller_chip_nodes, keycap_style};

const MODAL_KEY: &str = "RebindModal";
const TITLE_PATH: &str = "RebindModal/Card/Title";
const BIND_PATH: &str = "RebindModal/Card/Bind";
const DELETE_PATH: &str = "RebindModal/Card/Actions/Delete";
const SAVE_PATH: &str = "RebindModal/Card/Actions/Save";

const CLICK_TO_BIND: &str = "Click here to bind";
const LISTENING_LABEL: &str = "Press any key, button or axis...";

struct RebindModalState {
    // Which action this modal is currently capturing a binding for - None means
    // the modal is closed. Read back by update()/the Delete button's on_click
    // instead of each of them capturing their own copy, since which action is
    // "current" changes every time a different chip opens this same modal.
    open_action: Option<String>,
    // Some(i) = capturing to replace that action's existing binding i in place;
    // None = capturing to append a brand new binding instead. See this module's
    // own doc comment.
    edit_index: Option<usize>,
    // True from a "Bind" click (see `start_capture`) until a binding comes in
    // (see `update`) - gates whether `update` bothers polling
    // `input::captured_binding()` at all, see this module's own doc comment.
    capturing: bool,
    // The binding captured since the last `start_capture` (if any yet) -
    // staged, not yet written back to the input settings. See this module's
    // own doc comment.
    captured: Option<Binding>,
}

static STATE: Mutex<RebindModalState> = Mutex::new(RebindModalState { open_action: None, edit_index: None, capturing: false, captured: None });

/// Sets the "Bind" button/chip's displayed text and color in one place - used
/// for all three of its states (bound/placeholder at open, "listening" during
/// capture, "New binding: ..." once one's staged), see this module's own doc
/// comment.
fn set_bind_display(app: &mut App, text: &str, color: UiColor) {
    if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, BIND_PATH) {
        node.update_style(|s| s.set_text_color(color));
        if let Some(label_ref) = node.as_label_mut() {
            label_ref.set_text(&mut app.ui.text.font_system, text, false);
        }
    }
}

/// "throttle_up" -> "Throttle Up" - action names are the snake_case ids from
/// settings/input.ron, this is purely for the modal's title/settings-row text.
pub fn display_name(action: &str) -> String {
    action.split('_').map(|word| {
        let mut chars = word.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => String::new(),
        }
    }).collect::<Vec<_>>().join(" ")
}

fn delete_current(app: &mut App) {
    let (action, index) = {
        let state = STATE.lock().unwrap();
        match (state.open_action.clone(), state.edit_index) {
            (Some(action), Some(index)) => (action, index),
            _ => return, // Delete is hidden whenever edit_index is None - shouldn't be reachable, but nothing to delete either way.
        }
    };
    input::unbind(&action, index);
    refresh_chips(app, &action);
    close(app);
}

/// Writes the staged `captured` binding back to the input settings - the only
/// place that actually happens (see this module's own doc comment). No-op if
/// nothing's been captured yet (Save is `.active(false)` until then, so
/// shouldn't be reachable, but nothing to save either way).
fn save_current(app: &mut App) {
    let (action, edit_index, binding) = {
        let state = STATE.lock().unwrap();
        match (state.open_action.clone(), state.captured.clone()) {
            (Some(action), Some(binding)) => (action, state.edit_index, binding),
            _ => return,
        }
    };
    match edit_index {
        Some(index) => input::rebind_at(&action, index, binding),
        None => input::rebind(&action, binding),
    }
    refresh_chips(app, &action);
    close(app);
}

/// Rebuilds `action`'s chip row (see `main_menu::ui::controller_chip_nodes`)
/// after anything that changes its bindings (add/replace/delete), leaving the
/// rest of its section (the name label, the section's own hover/background)
/// untouched.
fn refresh_chips(app: &mut App, action: &str) {
    let chips = controller_chip_nodes(app, action);
    let path = format!("Settings/Content/Controller/List/{action}/Chips");
    if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &path) {
        if let Some(container) = node.as_container_mut() {
            container.children = chips;
        }
        app.ui.has_changed = true;
    }
}

/// Opens the modal for `action` without starting capture yet - see this
/// module's own doc comment for what `edit_index` means and why capturing is
/// deferred to an explicit "Bind" click. Safe to call again while already
/// open (e.g. clicking a different chip without closing first).
pub fn open(app: &mut App, action: &str, edit_index: Option<usize>) {
    {
        let mut state = STATE.lock().unwrap();
        state.open_action = Some(action.to_owned());
        state.edit_index = edit_index;
        state.capturing = false;
        state.captured = None;
    }
    // A capture from whatever chip was open before this one shouldn't bleed
    // into this one - harmless no-op if nothing was actually in progress.
    input::cancel_capture();

    if let Some(title) = Ui::get_ui_node(&mut app.ui.renderizable_elements, TITLE_PATH).and_then(|n| n.as_label_mut()) {
        title.set_text(&mut app.ui.text.font_system, &display_name(action), false);
    }
    let existing = edit_index.and_then(|index| input::action_bindings(action).get(index).cloned());
    match existing {
        Some(binding) => set_bind_display(app, &binding.label(), UiColor::Rgb(220, 220, 220)),
        None => set_bind_display(app, CLICK_TO_BIND, UiColor::Rgb(160, 160, 160)),
    }
    if let Some(delete_button) = Ui::get_ui_node(&mut app.ui.renderizable_elements, DELETE_PATH) {
        delete_button.set_active(edit_index.is_some());
    }
    if let Some(save_button) = Ui::get_ui_node(&mut app.ui.renderizable_elements, SAVE_PATH) {
        save_button.set_active(false);
    }
    if let Some(modal) = Ui::get_ui_node(&mut app.ui.renderizable_elements, MODAL_KEY) {
        modal.set_active(true);
    }
    app.ui.has_changed = true;
}

pub fn close(app: &mut App) {
    input::cancel_capture();
    {
        let mut state = STATE.lock().unwrap();
        state.open_action = None;
        state.edit_index = None;
        state.capturing = false;
        state.captured = None;
    }
    if let Some(modal) = Ui::get_ui_node(&mut app.ui.renderizable_elements, MODAL_KEY) {
        modal.set_active(false);
    }
    app.ui.has_changed = true;
}

/// The "Bind" button/chip's `on_click` - starts (or restarts, if the player
/// wants to try a different key before saving) listening for the next input
/// event. Discards anything already staged, since a fresh capture is about to
/// replace it.
fn start_capture(app: &mut App) {
    {
        let mut state = STATE.lock().unwrap();
        if state.open_action.is_none() {
            return;
        }
        state.capturing = true;
        state.captured = None;
    }
    input::begin_capture();
    if let Some(save_button) = Ui::get_ui_node(&mut app.ui.renderizable_elements, SAVE_PATH) {
        save_button.set_active(false);
    }
    set_bind_display(app, LISTENING_LABEL, UiColor::Rgb(255, 210, 120));
}

/// Polls for a newly captured binding while the modal's open - call every
/// frame regardless (see `main_menu::scene::update`), mirroring free_camera::
/// update's own "cheap early-return when inactive" shape. Only actually polls
/// `input::captured_binding()` while `capturing` is true (see `start_capture`)
/// - before the player's clicked "Bind" there's nothing to catch, and once a
/// binding's been staged, capture already stopped listening on its own (see
/// this module's own doc comment), so there's nothing left to poll either way.
pub fn update(app: &mut App) {
    let capturing = {
        let state = STATE.lock().unwrap();
        if state.open_action.is_none() {
            return;
        }
        state.capturing
    };
    if !capturing {
        return;
    }
    let Some(binding) = input::captured_binding() else { return };

    {
        let mut state = STATE.lock().unwrap();
        state.capturing = false;
        state.captured = Some(binding.clone());
    }
    set_bind_display(app, &format!("New binding: {}", binding.label()), UiColor::WHITE);
    if let Some(save_button) = Ui::get_ui_node(&mut app.ui.renderizable_elements, SAVE_PATH) {
        save_button.set_active(true);
    }
    app.ui.has_changed = true;
}

/// Builds the modal's shell once - call from main_menu::ui::build and add the
/// result as a top-level Layer child (inactive by default, see open()/close()).
///
/// The outer node IS the dimming scrim (full-screen, dark, blurred) rather than
/// a separate sibling behind a separately-positioned card: this layout model
/// has no absolute/overlay positioning for a container's children (each
/// container flow-stacks its active children, non-overlapping - a sibling
/// "Scrim" and "Card" would have ended up stacked one above the other instead
/// of overlapping), only `set_child_anchor(...)` to say where a child sits
/// within its parent's leftover space. With one non-Grow, fixed-size child and
/// Center/Center anchoring, that child just IS centered - no separate overlay
/// mechanism needed.
pub fn build(app: &mut App) -> UiNode {
    UiNode::container()
        .set_size(SizeValue::Grow, SizeValue::Grow)
        .set_padding(0.0)
        .set_background_color(UiColor::Rgba(0, 0, 0, 204))
        .set_child_anchor(Anchor::Center, Anchor::Center)
        .set_child("Card",
            card()
                .set_size(SizeValue::Pixels(480.0), SizeValue::Fit)
                // Same dark-glass language as the Settings backdrop (see
                // main_menu::ui's SETTINGS_BACKDROP/SETTINGS_BLUR own doc
                // comments for why alpha has to be this high for the blur to
                // actually read as darkening rather than washing out toward
                // whatever's behind it, and why the blur radius alone doesn't
                // control that).
                .set_background_color(UiColor::Rgba(0, 0, 0, 225))
                .set_background_blur(32.0)
                .set_border_color(UiColor::Rgba(255, 255, 255, 45))
                .set_border_width(1.0)
                .set_orientation(Orientation::Vertical)
                .set_child_anchor(Anchor::Start, Anchor::Center)
                .set_gap(15.0)
                .set_child("Title", label(app, "Action").set_size(SizeValue::Grow, SizeValue::Fit).set_align(glyphon::cosmic_text::Align::Center).set_font_size(&mut app.ui.text.font_system, 22.0).set_text_color(UiColor::WHITE))
                .set_child("Bind", keycap_style(
                    button(app, CLICK_TO_BIND, |app| start_capture(app))
                        .set_size(SizeValue::Grow, SizeValue::Fit)
                ))
                // Below Bind, right-aligned (Anchor::End on the horizontal
                // axis - the row itself is Grow-width, so End pushes the
                // group of buttons to the card's own right edge instead of
                // sitting flush against the left one).
                .set_child("Actions",
                    UiNode::container()
                        .set_orientation(Orientation::Horizontal)
                        .set_background_color(UiColor::TRANSPARENT)
                        .set_size(SizeValue::Grow, SizeValue::Fit)
                        .set_child_anchor(Anchor::End, Anchor::Center)
                        .set_gap(10.0)
                        .set_child("Delete", keycap_style(button(app, "Delete Bind", |app| delete_current(app))).active(false))
                        .set_child("Cancel", keycap_style(button(app, "Cancel", |app| close(app))))
                        // Inactive until a binding's actually been captured (see
                        // `update`) - nothing to write back before then, see this
                        // module's own doc comment on why Save/staging exists at all.
                        .set_child("Save", keycap_style(button(app, "Save", |app| save_current(app))).active(false))
                )
        )
        .active(false)
}
