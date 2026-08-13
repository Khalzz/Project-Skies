//! A modal for viewing/adding/removing an action's key bindings at runtime,
//! built entirely on the existing input-capture system (see
//! `engine::input::input`'s `begin_capture`/`captured_binding`/`rebind`/`unbind` -
//! this module is the "future rebinding menu" that doc comment already
//! anticipated) and the same function-based UI patterns as the rest of
//! `main_menu` (see `main_menu::ui`).
//!
//! Opened via `open(app, action)` from any settings row (see
//! `main_menu::ui::controller_settings_list`); `update(app)` has to run every
//! frame (see `main_menu::scene::update`) since it's polling for the captured
//! input, not reacting to a click.

use std::sync::Mutex;

use crate::app::App;
use crate::engine::input::action::Binding;
use crate::engine::input::input;
use crate::engine::rendering::ui::ui::Ui;
use crate::engine::ui::color::UiColor;
use crate::engine::ui::ui_node::UiNode;
use crate::engine::ui::ui_transform::{Anchor, Orientation, SizeValue};
use crate::game::ui::{button, card, label, with_left_accent};

const MODAL_KEY: &str = "RebindModal";
const LIST_PATH: &str = "RebindModal/Card/Body/List";
const TITLE_PATH: &str = "RebindModal/Card/Header/Title";
const ADD_BUTTON_PATH: &str = "RebindModal/Card/Body/AddBindButton";
const CAPTURE_PROMPT_PATH: &str = "RebindModal/Card/Body/CapturePrompt";

struct RebindModalState {
    // Which action's bindings this modal is currently showing/editing - None
    // means the modal is closed. Read back by update()/on_click closures instead
    // of each of them capturing their own copy, since which action is "current"
    // changes every time a different settings row opens this same modal.
    open_action: Option<String>,
    // True while waiting on input::captured_binding() after the user clicked
    // "Add Bind" - see update().
    awaiting_capture: bool,
}

static STATE: Mutex<RebindModalState> = Mutex::new(RebindModalState { open_action: None, awaiting_capture: false });

/// "throttle_up" -> "Throttle Up" - action names are the snake_case ids from
/// settings/input.ron, this is purely for the modal's title/settings-row text -
/// also used directly by main_menu::ui::controller_settings_list to label each
/// row.
pub fn display_name(action: &str) -> String {
    action.split('_').map(|word| {
        let mut chars = word.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => String::new(),
        }
    }).collect::<Vec<_>>().join(" ")
}

/// "Unbound", or every current binding's label joined with ", " - used both for
/// the settings row's own text and could be reused anywhere else that wants a
/// one-line summary of an action's bindings.
pub fn binding_summary(action: &str) -> String {
    let bindings = input::action_bindings(action);
    if bindings.is_empty() {
        "Unbound".to_owned()
    } else {
        bindings.iter().map(Binding::label).collect::<Vec<_>>().join(", ")
    }
}

fn erase_binding(app: &mut App, action: &str, index: usize) {
    input::unbind(action, index);
    refresh(app, action);
}

/// Rebuilds the bindings list and the settings row's summary text for `action` -
/// called after anything that changes its bindings (open, add, erase).
/// Container::children is a plain Vec (see its own doc comment), so this is a
/// straightforward "clear and rebuild from the current list" rather than a
/// diffing update - the list is always small (a handful of bindings at most).
fn refresh(app: &mut App, action: &str) {
    let bindings = input::action_bindings(action);

    // Read the list container's own already-resolved content box before
    // borrowing app mutably again to build new rows (label()/button() need
    // &mut app.ui.text.font_system) - can't hold the Ui::get_ui_node borrow
    // across that.
    let (content_w, content_h) = Ui::get_ui_node(&mut app.ui.renderizable_elements, LIST_PATH)
        .map(|list| {
            let t = &list.transform;
            let p = &list.padding;
            ((t.width - p.left - p.right).max(0.0), (t.height - p.top - p.bottom).max(0.0))
        })
        .unwrap_or((0.0, 0.0));

    let mut rows = Vec::with_capacity(bindings.len());
    for (index, binding) in bindings.iter().enumerate() {
        let action = action.to_owned();
        let mut row = UiNode::container()
            .set_orientation(Orientation::Horizontal)
            .set_size(SizeValue::Grow, SizeValue::Fit)
            .set_gap(10.0)
            .set_padding((8.0, 4.0))
            .set_background_color(UiColor::Rgba(255, 255, 255, 15))
            .set_child("Label", label(app, &binding.label()).set_size(SizeValue::Grow, SizeValue::Fit).set_text_color(UiColor::Rgb(220, 220, 220)))
            .set_child("Erase", with_left_accent(button(app, "X", move |app| erase_binding(app, &action, index))));
        // resolve() only ever runs once, top-down from Layer::build - these rows
        // are added long after that, directly into an already-built container's
        // children, so they need the same treatment by hand (see
        // UiNode::resolve's own doc comment on why this step exists at all).
        row.resolve(content_w, content_h);
        rows.push((format!("row_{index}"), row));
    }

    if let Some(list) = Ui::get_ui_node(&mut app.ui.renderizable_elements, LIST_PATH) {
        if let Some(container) = list.as_container_mut() {
            container.children = rows;
        }
    }

    if let Some(title) = Ui::get_ui_node(&mut app.ui.renderizable_elements, TITLE_PATH).and_then(|n| n.as_label_mut()) {
        title.set_text(&mut app.ui.text.font_system, &display_name(action), false);
    }

    sync_settings_row(app, action);

    app.ui.has_changed = true;
}

/// Every action listed in settings/controller.ron gets a row at this same
/// predictable path, keyed by its own action id, under whichever of the 2
/// side-by-side columns it landed in (see main_menu::ui::
/// controller_settings_list, which builds them, and splits which column a
/// given action ends up under - not tracked here, so both are just checked) -
/// a no-op if `action` isn't one of them (e.g. it has no settings row at all),
/// rather than needing to know the full curated list itself.
fn sync_settings_row(app: &mut App, action: &str) {
    for column in ["Left", "Right"] {
        let path = format!("Settings/Content/Controller/List/{column}/{action}");
        if let Some(row) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &path).and_then(|n| n.as_label_mut()) {
            row.set_text(&mut app.ui.text.font_system, &format!("{}: {}", display_name(action), binding_summary(action)), false);
            return;
        }
    }
}

fn set_capture_ui_active(app: &mut App, awaiting: bool) {
    if let Some(n) = Ui::get_ui_node(&mut app.ui.renderizable_elements, ADD_BUTTON_PATH) {
        n.set_active(!awaiting);
    }
    if let Some(n) = Ui::get_ui_node(&mut app.ui.renderizable_elements, CAPTURE_PROMPT_PATH) {
        n.set_active(awaiting);
    }
    app.ui.has_changed = true;
}

fn start_capture(app: &mut App) {
    input::begin_capture();
    STATE.lock().unwrap().awaiting_capture = true;
    set_capture_ui_active(app, true);
}

fn cancel_capture(app: &mut App) {
    input::cancel_capture();
    STATE.lock().unwrap().awaiting_capture = false;
    set_capture_ui_active(app, false);
}

/// Opens the modal for `action` - safe to call again with a different action
/// while already open (e.g. clicking a different settings row without closing
/// first), since it fully resets state rather than assuming it starts closed.
pub fn open(app: &mut App, action: &str) {
    let was_awaiting = {
        let mut state = STATE.lock().unwrap();
        let was_awaiting = state.awaiting_capture;
        state.open_action = Some(action.to_owned());
        state.awaiting_capture = false;
        was_awaiting
    };
    if was_awaiting {
        input::cancel_capture();
    }
    set_capture_ui_active(app, false);
    refresh(app, action);
    if let Some(modal) = Ui::get_ui_node(&mut app.ui.renderizable_elements, MODAL_KEY) {
        modal.set_active(true);
    }
    app.ui.has_changed = true;
}

pub fn close(app: &mut App) {
    let was_awaiting = {
        let mut state = STATE.lock().unwrap();
        let was_awaiting = state.awaiting_capture;
        state.open_action = None;
        state.awaiting_capture = false;
        was_awaiting
    };
    if was_awaiting {
        input::cancel_capture();
    }
    if let Some(modal) = Ui::get_ui_node(&mut app.ui.renderizable_elements, MODAL_KEY) {
        modal.set_active(false);
    }
    app.ui.has_changed = true;
}

/// Polls for the captured binding while waiting on one - call every frame
/// regardless of whether the modal is open (see main_menu::scene::update),
/// mirroring free_camera::update's own "cheap early-return when inactive" shape.
pub fn update(app: &mut App) {
    if !STATE.lock().unwrap().awaiting_capture {
        return;
    }
    let Some(binding) = input::captured_binding() else { return };

    let action = STATE.lock().unwrap().open_action.clone();
    if let Some(action) = action {
        input::rebind(&action, binding);
        STATE.lock().unwrap().awaiting_capture = false;
        set_capture_ui_active(app, false);
        refresh(app, &action);
    }
}

/// Builds the modal's shell once - call from main_menu::ui::build and add the
/// result as a top-level Layer child (inactive by default, see open()/close()).
/// The bindings list itself starts empty; refresh() fills it in whenever the
/// modal opens or its contents change.
///
/// The outer node IS the dimming scrim (full-screen, dark, blurred) rather than
/// a separate sibling behind a separately-positioned card: this layout model
/// has no absolute/overlay positioning for a container's children (each
/// container flow-stacks its active children, non-overlapping - a sibling
/// "Scrim" and "Card" would have ended up stacked one above the other instead
/// of overlapping), only `set_child_anchor(...)` to say where a child sits
/// within its parent's leftover space. With one non-Grow, fixed-size child and
/// Center/Center anchoring, that child just IS centered - no separate overlay
/// mechanism needed. Without the scrim at all, this was just another dark
/// translucent panel floating over an already-dark translucent Settings panel
/// behind it - same tone, same blur treatment, nothing to make it read as
/// elevated above the rest of the screen. (An earlier version of this dropped
/// the blur here because the Settings panel behind it seemed to vanish - that
/// was actually Ui::renderizable_elements' HashMap iteration order randomly
/// drawing Settings' own background over the modal's, see Ui::always_on_top;
/// now that render order is guaranteed, the blur is back.)
pub fn build(app: &mut App) -> UiNode {
    UiNode::container()
        .set_size(SizeValue::Grow, SizeValue::Grow)
        .set_padding(0.0)
        .set_background_color(UiColor::Rgba(0, 0, 0, 204))
        .set_child_anchor(Anchor::Center, Anchor::Center)
        .set_child("Card",
            card()
                .set_size(SizeValue::Pixels(520.0), SizeValue::Pixels(420.0))
                .set_background_color(UiColor::Rgba(23, 26, 33, 250))
                .set_background_blur(20.0)
                .set_border_color(UiColor::Rgba(255, 255, 255, 46))
                .set_border_width(1.0)
                .set_orientation(Orientation::Vertical)
                .set_gap(15.0)
                .set_child("Header",
                    UiNode::container()
                        .set_orientation(Orientation::Horizontal)
                        .set_size(SizeValue::Grow, SizeValue::Fit)
                        .set_background_color(UiColor::TRANSPARENT)
                        .set_child("Title", label(app, "Action").set_size(SizeValue::Grow, SizeValue::Fit).set_font_size(&mut app.ui.text.font_system, 24.0).set_text_color(UiColor::WHITE))
                        .set_child("Close", with_left_accent(button(app, "Close", |app| close(app))))
                )
                .set_child("Body",
                    UiNode::container()
                        .set_orientation(Orientation::Vertical)
                        .set_size(SizeValue::Grow, SizeValue::Grow)
                        .set_background_color(UiColor::TRANSPARENT)
                        .set_gap(10.0)
                        .set_child("List",
                            UiNode::container()
                                .set_orientation(Orientation::Vertical)
                                .set_size(SizeValue::Grow, SizeValue::Grow)
                                .set_background_color(UiColor::TRANSPARENT)
                                .set_gap(5.0)
                        )
                        .set_child("AddBindButton", with_left_accent(button(app, "Add Bind", |app| start_capture(app)).set_size(SizeValue::Grow, SizeValue::Fit)))
                        .set_child("CapturePrompt",
                            with_left_accent(button(app, "Press any key, button or axis... (click to cancel)", |app| cancel_capture(app)))
                                .set_size(SizeValue::Grow, SizeValue::Fit)
                                .set_text_color(UiColor::Rgb(255, 210, 120))
                                .active(false)
                        )
                )
        )
        .active(false)
}
