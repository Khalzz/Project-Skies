use std::sync::Mutex;

use serde::Deserialize;

use crate::app::App;
use crate::engine::input::input;
use crate::engine::rendering::ui::ui::Ui;
use crate::engine::ui::color::{Fill, UiColor};
use crate::engine::ui::layer::Layer;
use crate::game::game_settings::{GameSettings, HeadTrackingSource, COCKPIT_VIEW_PITCH_MAX, GAME_SETTINGS, HEAD_G_REACTION_MAX};
use crate::game::head_tracking;
use crate::engine::ui::ui_node::UiNode;
use crate::engine::ui::ui_transform::{Anchor, BorderEdges, Orientation, PositionValue, SizeValue};
use crate::game::selected_level::{SelectedLevel, SELECTED_LEVEL};
use crate::game::ui::{button, label, main_fade_gradient, set_slider, set_toggle_switch, slider, slider_fraction_at, toggle_switch, with_left_accent};
use super::rebind_modal;

const SETTINGS_TABS: [&str; 4] = ["Video", "Controller", "Audio", "Head Tracking"];
const CONNECTED_COLOR: UiColor = UiColor::Rgb(80, 220, 120);
const DISCONNECTED_COLOR: UiColor = UiColor::Rgb(160, 160, 160);
const METHOD_TEXT: UiColor = UiColor::Rgb(155, 155, 155);
/// Same border the video and head-tracking rows use while hovered. A selected
/// controller row keeps it after the cursor leaves.
const SELECTED_ROW: UiColor = UiColor::Rgba(255, 255, 255, 160);
const CHIP_TEXT: UiColor = UiColor::Rgb(220, 220, 220);

/// The controller row chosen for Add input / Clear input. None until a row is clicked.
static SELECTED_CONTROLLER: Mutex<Option<String>> = Mutex::new(None);

#[cfg(windows)]
const HEAD_SOURCES: [HeadTrackingSource; 2] = [HeadTrackingSource::TrackIr, HeadTrackingSource::OpenTrack];
#[cfg(not(windows))]
const HEAD_SOURCES: [HeadTrackingSource; 1] = [HeadTrackingSource::OpenTrack];
const SETTINGS_BACKDROP: UiColor = UiColor::Rgba(0, 0, 0, 250);
const SETTINGS_BLUR: f32 = 64.0;

pub(crate) fn backdrop() -> UiNode {
    UiNode::container()
        .set_size(SizeValue::Percent(100.0), SizeValue::Percent(100.0))
        .set_position(PositionValue::Start(0.0), PositionValue::Start(0.0))
        .set_background_color(main_fade_gradient())
        .set_transition(400.0)
}

fn show_panel(app: &mut App, id: &str) {
    for panel_id in ["Main Menu", "Settings", "Play Select"] {
        if let Some(panel) = Ui::get_ui_node(&mut app.ui.renderizable_elements, panel_id) {
            panel.set_active(panel_id == id);
        }
    }
    if let Some(backdrop) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "Backdrop") {
        // Same flat dark backdrop for Play Select as Settings - same reasoning
        // (SETTINGS_BACKDROP's own doc comment): both have enough of their own
        // sidebar+content layout that a flat backdrop reads better than the
        // main menu's left-to-transparent gradient repeating behind it.
        let dark_panel = id == "Settings" || id == "Play Select";
        let color = if dark_panel { Fill::Solid(SETTINGS_BACKDROP) } else { main_fade_gradient() };
        let blur = if dark_panel { SETTINGS_BLUR } else { 0.0 };
        backdrop.update_style(|s| s.set_background_color(color).set_background_blur(blur));
    }
}

/// Shows one section to the right of the name list. The list, and its Back
/// button at the bottom, stay where they are.
fn open_settings_section(app: &mut App, id: &str) {
    if let Some(content) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "Settings/Content") {
        content.set_active(true);
    }
    for tab_id in SETTINGS_TABS {
        if let Some(tab) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &format!("Settings/Content/{tab_id}")) {
            tab.set_active(tab_id == id);
        }
        if let Some(button) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &format!("Settings/List/{tab_id}")) {
            let selected = tab_id == id;
            let border = if selected { UiColor::WHITE } else { UiColor::TRANSPARENT };
            let text = if selected { UiColor::WHITE } else { UiColor::Rgb(200, 200, 200) };
            button.update_style(|style| style.set_border_color(border).set_text_color(text));
        }
    }
    app.ui.has_changed = true;
}

/// A `with_left_accent` button sized for a sidebar row - `button()`'s own
/// default padding, same height as the main menu's own top-level Play/
/// Settings/Quit buttons (see `build`), not overridden to something taller.
pub(crate) fn sidebar_button(app: &mut App, text: &str, on_click: impl Fn(&mut App) + 'static) -> UiNode {
    with_left_accent(button(app, text, on_click).set_size(SizeValue::Grow, SizeValue::Fit))
}

/// An invisible spacer that claims *all* the leftover main-axis space in its
/// container (see `SizeValue::Grow`) - used to pin whatever comes after it to
/// the far edge instead of stacking directly under whatever comes before,
/// CSS flexbox's `justify-content: space-between` for two groups. See the
/// sidebar below: Title/tabs stay pinned to the top, "Back" gets pushed all
/// the way down to the bottom instead of floating in the middle of the column.
pub fn grow_spacer() -> UiNode {
    UiNode::container().set_size(SizeValue::Grow, SizeValue::Grow).set_background_color(UiColor::TRANSPARENT)
}

/// Settings opens on a list of names. Choosing one shows that section to the
/// right; the list and its Back button stay put at the bottom. Back is
/// `on_back` (main menu, or the pause menu - see `play::ui`).
///
/// The section pages stay direct children of "Settings/Content/{tab}", which
/// is what the slider, toggle and controller rows look up. Safe across scenes
/// because only one scene's UI tree is alive at a time.
pub(crate) fn settings_ui(app: &mut App, on_back: impl Fn(&mut App) + 'static) -> UiNode {
    UiNode::container()
        .set_orientation(Orientation::Horizontal)
        .set_background_color(UiColor::TRANSPARENT)
        .set_size(SizeValue::Grow, SizeValue::Grow)
        .set_child_anchor(Anchor::Start, Anchor::Start)
        .set_child("List",
            UiNode::container()
                .set_orientation(Orientation::Vertical)
                .set_background_color(UiColor::TRANSPARENT)
                .set_size(SizeValue::Pixels(240.0), SizeValue::Grow)
                .set_padding([60.0, 60.0, 40.0, 40.0])
                .set_child_anchor(Anchor::Start, Anchor::Start)
                .set_gap(8.0)
                .set_child("Title", label(app, "Settings").set_font_size(&mut app.ui.text.font_system, 32.0).set_text_color(UiColor::WHITE))
                .set_child("Controller", sidebar_button(app, "Controller", |app| open_settings_section(app, "Controller")))
                .set_child("Video", sidebar_button(app, "Video", |app| open_settings_section(app, "Video")))
                .set_child("Audio", sidebar_button(app, "Audio", |app| open_settings_section(app, "Audio")))
                .set_child("Head Tracking", sidebar_button(app, "Head Tracking", |app| open_settings_section(app, "Head Tracking")))
                .set_child("BackSpacer", grow_spacer())
                .set_child("Back", sidebar_button(app, "Back", on_back))
        )
        .set_child("Divider",
            UiNode::container()
                .set_size(SizeValue::Pixels(1.0), SizeValue::Grow)
                .set_background_color(UiColor::Rgba(255, 255, 255, 40)))
        .set_child("Content",
            UiNode::container()
                .set_background_color(UiColor::TRANSPARENT)
                .set_size(SizeValue::Grow, SizeValue::Grow)
                .set_padding([60.0, 60.0, 30.0, 30.0])
                .set_child_anchor(Anchor::Start, Anchor::Start)
                .set_gap(20.0)
                .set_child("Controller",
                    UiNode::container()
                        .set_orientation(Orientation::Vertical)
                        .set_background_color(UiColor::TRANSPARENT)
                        // Grow, not Fit - needs to fill Content's full available
                        // height so List (below) has a real bounded remainder to
                        // scroll within, instead of auto-growing to fit however
                        // tall List's own content ends up being.
                        .set_size(SizeValue::Grow, SizeValue::Grow)
                        .set_gap(20.0)
                        .set_child("Title", label(app, "Controller").set_font_size(&mut app.ui.text.font_system, 26.0).set_text_color(UiColor::WHITE))
                        .set_child("List", controller_settings_list(app))
                        .set_child("Actions", controller_action_buttons(app))
                        .active(false))
                .set_child("Video", video_settings(app).active(false))
                .set_child("Audio", label(app, "Audio Settings").set_font_size(&mut app.ui.text.font_system, 26.0).set_text_color(UiColor::WHITE).active(false))
                .set_child("Head Tracking", head_tracking_settings(app).active(false))
                .active(false))
        .active(false)
}

/// The Video tab - one `settings_toggle_row` per on/off setting.
fn video_settings(app: &mut App) -> UiNode {
    UiNode::container()
        .set_background_color(UiColor::TRANSPARENT)
        .set_size(SizeValue::Grow, SizeValue::Fit)
        .set_gap(20.0)
        .set_child("Title", label(app, "Video").set_font_size(&mut app.ui.text.font_system, 26.0).set_text_color(UiColor::WHITE))
        .set_child("FreeCameraFollowsPlane", settings_toggle_row(app, "Video", "FreeCameraFollowsPlane", "Free camera follows plane",
            |settings| settings.free_camera_follows_plane,
            |settings| settings.free_camera_follows_plane = !settings.free_camera_follows_plane))
        .set_child("HeadGReaction", settings_slider_row(app, "Video", "HeadGReaction", "Cockpit head G reaction",
            SliderRange { min: 0.0, max: HEAD_G_REACTION_MAX, step: 0.01, format: percent_text },
            |settings| settings.head_g_reaction,
            |settings, value| settings.head_g_reaction = value))
        .set_child("CockpitViewPitch", settings_slider_row(app, "Video", "CockpitViewPitch", "Cockpit view angle",
            SliderRange { min: -COCKPIT_VIEW_PITCH_MAX, max: COCKPIT_VIEW_PITCH_MAX, step: 0.1, format: degrees_text },
            |settings| settings.cockpit_view_pitch_deg,
            |settings, value| settings.cockpit_view_pitch_deg = value))
        .set_child("ShowAircraftNames", settings_toggle_row(app, "Video", "ShowAircraftNames", "Show aircraft names",
            |settings| settings.show_aircraft_names,
            |settings| settings.show_aircraft_names = !settings.show_aircraft_names))
        .set_child("HudStaysWithJet", settings_toggle_row(app, "Video", "HudStaysWithJet", "HUD stays with the jet",
            |settings| settings.hud_stays_with_jet,
            |settings| settings.hud_stays_with_jet = !settings.hud_stays_with_jet))
}

/// The Head Tracking tab. The method row is a `settings_row`; the picker and
/// Retry sit in it.
fn head_tracking_settings(app: &mut App) -> UiNode {
    head_tracking::ensure_started(app);

    UiNode::container()
        .set_background_color(UiColor::TRANSPARENT)
        .set_size(SizeValue::Grow, SizeValue::Fit)
        .set_gap(20.0)
        .set_child("Title", label(app, "Head Tracking").set_font_size(&mut app.ui.text.font_system, 26.0).set_text_color(UiColor::WHITE))
        .set_child("Row", head_tracking_row(app))
}

/// One settings line: gray name on the left, whatever the caller adds on the
/// right. Hovering the line draws a border around it and turns its labels
/// white. The name and the spacer are already in place; callers append the
/// control (`Control`/`Retry`, a switch, a slider).
fn settings_row(app: &mut App, name: &str) -> UiNode {
    UiNode::container()
        .set_orientation(Orientation::Horizontal)
        .set_background_color(UiColor::TRANSPARENT)
        .set_border_width(1.0)
        .set_border_color(UiColor::TRANSPARENT)
        .set_corner_radius(6.0)
        .set_padding(14.0)
        .set_size(SizeValue::Grow, SizeValue::Fit)
        .set_child_anchor(Anchor::Start, Anchor::Center)
        .set_gap(16.0)
        .set_transition(80.0)
        .on_hover(|style| style.set_border_color(UiColor::Rgba(255, 255, 255, 160)).set_text_color(UiColor::WHITE))
        .set_child("Name", label(app, name).set_size(SizeValue::Pixels(260.0), SizeValue::Fit).set_text_color(METHOD_TEXT))
        .set_child("Spacer", grow_spacer())
}

fn head_tracking_row(app: &mut App) -> UiNode {
    settings_row(app, "Tracking method")
        .set_child("Control", head_tracking_control(app))
        .set_child("Retry", retry_button(app))
}

fn head_tracking_control(app: &mut App) -> UiNode {
    let selected = GAME_SETTINGS.lock().unwrap().head_tracking;
    UiNode::container()
        .set_orientation(Orientation::Horizontal)
        .set_background_color(UiColor::TRANSPARENT)
        .set_size(SizeValue::Fit, SizeValue::Fit)
        .set_child_anchor(Anchor::Start, Anchor::Center)
        .set_gap(10.0)
        .set_child("Left", step_triangle(true, |app| step_head_tracking(app, -1)))
        .set_child("Middle", head_tracking_value(app, selected))
        .set_child("Right", step_triangle(false, |app| step_head_tracking(app, 1)))
}

/// The method name with its status dot sitting just to the right of it.
fn head_tracking_value(app: &mut App, selected: HeadTrackingSource) -> UiNode {
    UiNode::container()
        .set_orientation(Orientation::Horizontal)
        .set_background_color(UiColor::TRANSPARENT)
        .set_size(SizeValue::Fit, SizeValue::Fit)
        .set_child_anchor(Anchor::Start, Anchor::Center)
        .set_gap(8.0)
        .set_child("Name", label(app, selected.label()).set_text_color(METHOD_TEXT))
        .set_child("Dot", status_dot(head_tracking::source_connected(selected)))
}

/// A gray triangle in a rounded square, the same 22px and corner as Retry.
/// The square stays clear until hover, then fills white while the triangle
/// turns black. A negative corner radius is what asks the UI shader to draw
/// the triangle (see `text_shader.wgsl`); `points_left` picks the direction.
fn step_triangle(points_left: bool, on_click: impl Fn(&mut App) + 'static) -> UiNode {
    UiNode::container()
        .set_size(SizeValue::Pixels(22.0), SizeValue::Pixels(22.0))
        .set_child_anchor(Anchor::Center, Anchor::Center)
        .set_background_color(UiColor::TRANSPARENT)
        .set_border_width(0.0)
        .set_corner_radius(4.0)
        .set_transition(50.0)
        .on_hover(|style| style.set_background_color(UiColor::WHITE).set_text_color(UiColor::BLACK))
        .on_click(on_click)
        .set_child("Mark", UiNode::container()
            .set_size(SizeValue::Pixels(8.0), SizeValue::Pixels(10.0))
            .set_background_color(METHOD_TEXT)
            .set_border_width(0.0)
            .set_border_color(METHOD_TEXT)
            .set_corner_radius(if points_left { -100.0 } else { -1.0 }))
}

fn step_head_tracking(app: &mut App, delta: isize) {
    let current = GAME_SETTINGS.lock().unwrap().head_tracking;
    let index = HEAD_SOURCES.iter().position(|source| *source == current).unwrap_or(0);
    let len = HEAD_SOURCES.len() as isize;
    let next = (index as isize + delta).rem_euclid(len) as usize;
    GAME_SETTINGS.lock().unwrap().head_tracking = HEAD_SOURCES[next];
    head_tracking::selection_changed();
    refresh_head_tracking(app);
}

/// White label on a clear square. Hover fills the square white and turns the
/// label black, same as the method arrows beside it - the row's own hover
/// would otherwise keep the word white and hide it on that fill.
fn retry_button(app: &mut App) -> UiNode {
    UiNode::container()
        .set_size(SizeValue::Fit, SizeValue::Pixels(22.0))
        .set_child_anchor(Anchor::Center, Anchor::Center)
        .set_padding((8.0, 0.0))
        .set_background_color(UiColor::TRANSPARENT)
        .set_border_width(0.0)
        .set_corner_radius(4.0)
        .set_transition(50.0)
        .on_hover(|style| style.set_background_color(UiColor::WHITE).set_text_color(UiColor::BLACK))
        .on_click(|app| {
            head_tracking::retry(app);
            refresh_head_tracking(app);
        })
        .set_child("Label", label(app, "Retry")
            .set_text_color(UiColor::WHITE)
            .set_font_size(&mut app.ui.text.font_system, 14.0)
            .set_align(glyphon::cosmic_text::Align::Center)
            .set_size(SizeValue::Fit, SizeValue::Pixels(22.0)))
}

fn status_dot(connected: bool) -> UiNode {
    UiNode::container()
        .set_size(SizeValue::Pixels(8.0), SizeValue::Pixels(8.0))
        .set_corner_radius(4.0)
        .set_background_color(if connected { CONNECTED_COLOR } else { DISCONNECTED_COLOR })
}

/// Rewrites the method name, its dot, and the Retry label when the tracker
/// changed them. No-op until something has changed, and when this section
/// isn't in the tree.
pub(crate) fn refresh_head_tracking_if_needed(app: &mut App) {
    // The countdown has to repaint every second, not only on the click, so
    // a wait in progress refreshes even when nothing else about the tracker
    // changed this frame.
    let counting = head_tracking::retry_caption() != "Retry";
    if head_tracking::take_changed() || counting {
        refresh_head_tracking(app);
    }
}

/// Rewrites the method name, its dot, and the Retry label. No-op when this
/// section isn't in the tree.
fn refresh_head_tracking(app: &mut App) {
    let selected = GAME_SETTINGS.lock().unwrap().head_tracking;
    let name_path = "Settings/Content/Head Tracking/Row/Control/Middle/Name";
    if let Some(name) = Ui::get_ui_node(&mut app.ui.renderizable_elements, name_path) {
        if let Some(label) = name.as_label_mut() {
            label.set_text(&mut app.ui.text.font_system, selected.label(), true);
        }
    }
    let dot_path = "Settings/Content/Head Tracking/Row/Control/Middle/Dot";
    if let Some(dot) = Ui::get_ui_node(&mut app.ui.renderizable_elements, dot_path) {
        let color = if head_tracking::source_connected(selected) { CONNECTED_COLOR } else { DISCONNECTED_COLOR };
        dot.update_style(|style| style.set_background_color(color));
    }
    let retry_path = "Settings/Content/Head Tracking/Row/Retry/Label";
    if let Some(retry) = Ui::get_ui_node(&mut app.ui.renderizable_elements, retry_path) {
        if let Some(label) = retry.as_label_mut() {
            label.set_text(&mut app.ui.text.font_system, head_tracking::retry_caption(), true);
        }
    }
    app.ui.has_changed = true;
}

/// A `settings_slider_row`'s values: `min` at the left end, `max` at the
/// right, snapped to `step`, and shown by `format`.
#[derive(Clone, Copy)]
struct SliderRange {
    min: f32,
    max: f32,
    step: f32,
    format: fn(f32) -> String,
}

impl SliderRange {
    fn fraction(&self, value: f32) -> f32 {
        (value - self.min) / (self.max - self.min)
    }

    /// The value at `fraction` along the slider, snapped to `step` - so the
    /// readout and the stored value agree.
    fn value_at(&self, fraction: f32) -> f32 {
        let value = self.min + fraction * (self.max - self.min);
        ((value / self.step).round() * self.step).clamp(self.min, self.max)
    }
}

/// A setting a `settings_slider_row` is being dragged on: where its slider
/// and readout are, its range, and how to store what it's dragged to.
struct SliderDrag {
    row_path: String,
    range: SliderRange,
    write: fn(&mut GameSettings, f32),
}

thread_local! {
    // The slider the mouse went down on, while the button stays held - see
    // `update_settings_sliders`.
    static SLIDER_DRAG: std::cell::RefCell<Option<SliderDrag>> = const { std::cell::RefCell::new(None) };
}

/// One setting over `range`, on a `settings_row`: a `slider` and the value
/// (as `range.format` shows it) after the name. Pressing on the slider jumps
/// it to the mouse and starts a drag that `update_settings_sliders` follows
/// every frame until the button's let go. `tab`/`key` are this row's own
/// place in the tree, as for `settings_toggle_row`. `Slider` and `Value`
/// stay direct children so that drag can find them.
fn settings_slider_row(app: &mut App, tab: &str, key: &str, name: &str, range: SliderRange, read: fn(&GameSettings) -> f32, write: fn(&mut GameSettings, f32)) -> UiNode {
    let row_path = format!("Settings/Content/{tab}/{key}");
    let value = read(&GAME_SETTINGS.lock().unwrap());

    settings_row(app, name)
        .set_child("Slider", slider(range.fraction(value)).on_click(move |app| {
            SLIDER_DRAG.with(|drag| *drag.borrow_mut() = Some(SliderDrag { row_path: row_path.clone(), range, write }));
            update_settings_sliders(app);
        }))
        .set_child("Value", label(app, &(range.format)(value)).set_size(SizeValue::Pixels(80.0), SizeValue::Fit).set_text_color(UiColor::WHITE))
}

fn percent_text(value: f32) -> String {
    format!("{:.0}%", value * 100.0)
}

fn degrees_text(value: f32) -> String {
    format!("{:+.1} deg", value)
}

/// Follows a `settings_slider_row` drag: while the click that started it is
/// held, stores the value under the mouse and moves the slider/readout to
/// it; once it's let go, the drag ends. Call every frame the settings can be
/// open (main menu, and the play scene while paused).
pub(crate) fn update_settings_sliders(app: &mut App) {
    SLIDER_DRAG.with(|drag| {
        let mut drag = drag.borrow_mut();
        let Some(active) = drag.as_ref() else { return };
        if !input::is_action_pressed("ui_click") {
            *drag = None;
            return;
        }

        let Some(slider_node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &format!("{}/Slider", active.row_path)) else {
            *drag = None;
            return;
        };
        let value = active.range.value_at(slider_fraction_at(slider_node, input::mouse_x() as f32));
        set_slider(slider_node, active.range.fraction(value));
        (active.write)(&mut GAME_SETTINGS.lock().unwrap(), value);

        if let Some(readout) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &format!("{}/Value", active.row_path)).and_then(|n| n.as_label_mut()) {
            readout.set_text(&mut app.ui.text.font_system, &(active.range.format)(value), true);
        }
        app.ui.has_changed = true;
    });
}

/// One on/off setting on a `settings_row`: its name beside a `toggle_switch`.
/// Clicking anywhere on the row flips it in `GAME_SETTINGS` via `flip`, then
/// re-reads it with `read` to move the switch - so the switch always shows
/// what's actually stored. The switch has no click of its own, so the row
/// still receives it. `tab`/`key` must be this row's own place in the tree
/// ("Settings/Content/{tab}/{key}"), which is how the click finds its switch.
fn settings_toggle_row(app: &mut App, tab: &str, key: &str, name: &str, read: fn(&GameSettings) -> bool, flip: fn(&mut GameSettings)) -> UiNode {
    let switch_path = format!("Settings/Content/{tab}/{key}/Switch");
    let on = read(&GAME_SETTINGS.lock().unwrap());

    settings_row(app, name)
        .set_child("Switch", toggle_switch(on))
        .on_click(move |app| {
            let on = {
                let mut settings = GAME_SETTINGS.lock().unwrap();
                flip(&mut settings);
                read(&settings)
            };
            if let Some(switch) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &switch_path) {
                set_toggle_switch(switch, on);
            }
            app.ui.has_changed = true;
        })
}

/// Which actions get a row here, grouped by job - see settings/controller.ron
/// (a curated subset of settings/input.ron's full action list: debug/UI-internal
/// actions aren't meant to be player-rebindable, so they're left out of this file
/// entirely rather than filtered here).
const CONTROLLER_ACTIONS_RON: &str = include_str!("../../../../settings/controller.ron");

#[derive(serde::Deserialize)]
struct ControllerSection {
    name: String,
    actions: Vec<String>,
}

fn controller_sections() -> Vec<ControllerSection> {
    ron::from_str(CONTROLLER_ACTIONS_RON).expect("Failed to parse settings/controller.ron")
}

/// Where `action`'s settings row lives, once the page is built.
pub(crate) fn controller_action_path(action: &str) -> Option<String> {
    controller_sections().into_iter().find(|section| section.actions.iter().any(|name| name == action)).map(|section| format!("Settings/Content/Controller/List/{}/{action}", section.name))
}

/// One block per group (Flying, Camera, ...), each a title and then one row
/// per action - see `controller_settings_section`.
fn controller_settings_list(app: &mut App) -> UiNode {
    let mut list = UiNode::container()
        .set_background_color(UiColor::TRANSPARENT)
        // Grow, not Fit - a bounded viewport for scroll to actually happen
        // within (see set_scrollable's own doc comment), rather than always
        // growing tall enough to fit every row. Right padding reserves a
        // gutter for the scrollbar so it doesn't overlap each row's own
        // right edge/content.
        .set_size(SizeValue::Grow, SizeValue::Grow)
        .set_padding([0.0, 0.0, 0.0, 16.0])
        .set_scrollable(true);

    for section in controller_sections() {
        let mut block = UiNode::container()
            .set_orientation(Orientation::Vertical)
            .set_background_color(UiColor::TRANSPARENT)
            .set_size(SizeValue::Grow, SizeValue::Fit)
            .set_gap(4.0)
            .set_child(
                "Title",
                label(app, &section.name)
                    .set_font_size(&mut app.ui.text.font_system, 18.0)
                    .set_text_color(UiColor::WHITE)
                    .set_padding([16.0, 8.0, 4.0, 0.0]),
            );
        for action in section.actions {
            block = block.set_child(&action, controller_settings_section(app, &action));
        }
        list = list.set_child(section.name, block);
    }

    list
}

/// Writes each action row's live reading (see the "State" label on
/// `controller_settings_section`). A button reads off or its strength; an
/// axis reads how far it's leaned, so a binding can be tried without leaving
/// the page. No-op when the settings page isn't built.
pub(crate) fn refresh_controller_states(app: &mut App) {
    for section in controller_sections() {
        for action in section.actions {
            let path = format!("Settings/Content/Controller/List/{}/{action}/State", section.name);
            let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &path) else { continue };
            let strength = input::action_strength(&action);
            let live = strength > 0.02 || input::is_action_pressed(&action);
            node.style.text_color = Some(if live { UiColor::Rgb(80, 220, 120) } else { UiColor::Rgb(130, 130, 130) });
            if let Some(label) = node.as_label_mut() {
                let text = if live { format!("{strength:.2}") } else { "off".to_owned() };
                label.set_text(&mut app.ui.text.font_system, &text, true);
            }
        }
    }
}

/// A single action's row, in the same hover treatment as the video and
/// head-tracking lines. Clicking it selects that action for the Add input
/// and Clear input buttons at the bottom of the page. The chips on the right
/// are the keys already bound to it; they don't open the rebind modal.
fn controller_settings_section(app: &mut App, action: &str) -> UiNode {
    let mut chips = UiNode::container()
        .set_orientation(Orientation::Horizontal)
        .set_background_color(UiColor::TRANSPARENT)
        .set_size(SizeValue::Fit, SizeValue::Fit)
        .set_child_anchor(Anchor::Start, Anchor::Center)
        .set_gap(6.0);
    for (key, chip) in controller_chip_nodes(app, action) {
        chips = chips.set_child(key, chip);
    }

    let action_name = action.to_owned();
    settings_row(app, &rebind_modal::display_name(action))
        .set_child("State", label(app, "off").set_size(SizeValue::Pixels(48.0), SizeValue::Fit).set_align(glyphon::cosmic_text::Align::Center).set_text_color(UiColor::Rgb(130, 130, 130)))
        .set_child("Chips", chips)
        .on_click(move |app| select_controller_action(app, &action_name))
}

/// Add input and Clear input, pinned under the scrolling list. They act on
/// whichever row was clicked last.
fn controller_action_buttons(app: &mut App) -> UiNode {
    UiNode::container()
        .set_orientation(Orientation::Horizontal)
        .set_background_color(UiColor::TRANSPARENT)
        .set_size(SizeValue::Grow, SizeValue::Fit)
        .set_child_anchor(Anchor::Start, Anchor::Center)
        .set_gap(10.0)
        .set_child("Add", keycap_style(button(app, "Add input", add_selected_input)))
        .set_child("Clear", keycap_style(button(app, "Clear input", clear_selected_input)))
}

fn select_controller_action(app: &mut App, action: &str) {
    *SELECTED_CONTROLLER.lock().unwrap() = Some(action.to_owned());
    paint_controller_selection(app);
}

fn add_selected_input(app: &mut App) {
    let Some(action) = SELECTED_CONTROLLER.lock().unwrap().clone() else { return };
    rebind_modal::open(app, &action, None);
}

fn clear_selected_input(app: &mut App) {
    let Some(action) = SELECTED_CONTROLLER.lock().unwrap().clone() else { return };
    input::clear_bindings(&action);
    refresh_controller_chips(app, &action);
}

/// Lights the selected row the way a video row looks while the cursor is on it,
/// and puts the others back to rest.
fn paint_controller_selection(app: &mut App) {
    let selected = SELECTED_CONTROLLER.lock().unwrap().clone();
    for section in controller_sections() {
        for action in section.actions {
            let path = format!("Settings/Content/Controller/List/{}/{action}", section.name);
            let on = selected.as_deref() == Some(action.as_str());
            if let Some(row) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &path) {
                let border = if on { SELECTED_ROW } else { UiColor::TRANSPARENT };
                row.update_style(|style| style.set_border_color(border));
            }
            if let Some(name) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &format!("{path}/Name")) {
                let color = if on { UiColor::WHITE } else { METHOD_TEXT };
                name.update_style(|style| style.set_text_color(color));
            }
            if let Some(chips) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &format!("{path}/Chips")) {
                if let Some(container) = chips.as_container_mut() {
                    let color = if on { UiColor::WHITE } else { CHIP_TEXT };
                    for (_, chip) in &mut container.children {
                        chip.update_style(|style| style.set_text_color(color));
                    }
                }
            }
        }
    }
    app.ui.has_changed = true;
}

/// Rebuilds `action`'s chip row after a binding is added or cleared.
pub(crate) fn refresh_controller_chips(app: &mut App, action: &str) {
    let chips = controller_chip_nodes(app, action);
    let Some(path) = controller_action_path(action).map(|path| format!("{path}/Chips")) else { return };
    if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &path) {
        if let Some(container) = node.as_container_mut() {
            container.children = chips;
        }
        app.ui.has_changed = true;
    }
    paint_controller_selection(app);
}

/// The "keycap" color/hover treatment shared by the settings chips (see
/// `chip_style`, below) and the rebind modal's own "Bind"/"Delete Bind"/
/// "Cancel"/"Save" buttons (see `main_menu::rebind_modal::build`) - one
/// consistent button language across every clickable element in the
/// controller-rebinding flow. Resting state reads as a "keycap": a faint
/// outline over a barely-there fill. Hover is a full inversion rather than a
/// token brightness bump - solid near-white fill, near-black text - a "this
/// key is lit up" look, not just `button()`'s own baked-in hover (a black
/// darken, which reads backward here: darkening an already-faint light-tint
/// button just fades it toward invisible instead of highlighting it) nudged
/// slightly brighter. This call's own fields win over that baked-in hover
/// wherever it sets something (see `UiNode::on_hover`'s own doc comment on
/// the merge) - here that's every field `button()`'s hover touches, so
/// nothing bleeds through from it. Its own `.set_transition(50.0)` is
/// overridden too - too fast a fade for this much bigger a color jump reads
/// as a harsh flash rather than a smooth light-up (see
/// `controller_settings_section`'s own transition-duration comment for the
/// same reasoning in more detail).
///
/// Doesn't size the node at all (beyond the padding baked into the look
/// itself) - callers that want a compact, roughly-square chip layer
/// `.set_min_width_to_height()` on top (see `chip_style`); callers that want
/// their own size (the modal's Grow-width "Bind" button) just set it directly.
pub fn keycap_style(node: UiNode) -> UiNode {
    node
        .set_padding((10.0, 6.0))
        .set_background_color(UiColor::Rgba(255, 255, 255, 28))
        .set_border_color(UiColor::Rgba(255, 255, 255, 45))
        .set_text_color(UiColor::Rgb(220, 220, 220))
        .set_align(glyphon::cosmic_text::Align::Center)
        .set_corner_radius(8.0)
        .on_hover(|s| s
            .set_background_color(UiColor::Rgba(255, 255, 255, 225))
            .set_border_color(UiColor::WHITE)
            .set_text_color(UiColor::Rgb(20, 20, 20)))
        .set_transition(120.0)
}

/// Shared look for a binding chip (see `controller_chip_nodes`) - `keycap_style`
/// squared into a compact pill. Display only: the row click selects the action,
/// and Add input / Clear input at the bottom of the page change its bindings.
fn chip_style(node: UiNode) -> UiNode {
    keycap_style(node)
        .set_border_width(1.0)
        .set_min_width_to_height()
}

/// The chip nodes for a controller row's "Chips" child - one per current
/// binding, keyed `"chip_{index}"`. Also called by `refresh_controller_chips`
/// after a binding is added or cleared.
pub fn controller_chip_nodes(app: &mut App, action: &str) -> Vec<(String, UiNode)> {
    let bindings = input::action_bindings(action);
    let mut nodes = Vec::with_capacity(bindings.len());

    for (index, binding) in bindings.iter().enumerate() {
        let chip = chip_style(label(app, &binding.label().to_uppercase()));
        nodes.push((format!("chip_{index}"), chip));
    }

    nodes
}

/// One entry in the Play screen's level list - see `assets/levels.ron`, the
/// only data source (no code path adds/edits these at runtime, unlike the
/// controller bindings). `scene` is what `SceneManager::open_scene` expects
/// (registered in `main.rs`) - not necessarily the same string as `id`,
/// which is purely this list's own stable key (RON authoring convenience,
/// UI node paths, `SELECTED_LEVEL`). `mission_title`/`location`/
/// `mission_date` are for the in-scene mission-intro card (see `play::scene::
/// GameLogic`, the only other reader - via `SELECTED_LEVEL`), not this list's
/// own `name`/`description` - see `assets/levels.ron`'s own doc comment.
#[derive(Deserialize, Clone)]
struct LevelInfo {
    id: String,
    name: String,
    description: String,
    scene: String,
    mission_title: String,
    location: String,
    mission_date: String,
}

const LEVELS_RON: &str = include_str!("../../../../assets/levels.ron");

fn levels() -> Vec<LevelInfo> {
    ron::from_str(LEVELS_RON).expect("Failed to parse assets/levels.ron")
}

/// Selects `id` as the current level: stages it for the "Play" button (see
/// `SELECTED_LEVEL`, which `play::scene::GameLogic::new` reads back once the
/// chosen scene actually opens - see its own doc comment for why this static
/// exists at all), fills in Content's name/description text, and updates
/// every sidebar button's persistent selected-highlight with a direct style
/// write, because it needs to stay showing regardless of hover/press state.
/// No-op if `id`
/// isn't in `assets/levels.ron`.
fn show_level(app: &mut App, id: &str) {
    let Some(level) = levels().into_iter().find(|l| l.id == id) else { return };
    *SELECTED_LEVEL.lock().unwrap() = Some(SelectedLevel {
        scene: level.scene.clone(),
        mission_title: level.mission_title.clone(),
        location: level.location.clone(),
        mission_date: level.mission_date.clone(),
    });

    if let Some(name) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "Play Select/Main/Content/Name").and_then(|n| n.as_label_mut()) {
        name.set_text(&mut app.ui.text.font_system, &level.name, false);
    }
    if let Some(description) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "Play Select/Main/Content/Description").and_then(|n| n.as_label_mut()) {
        description.set_text(&mut app.ui.text.font_system, &level.description, false);
    }
    for other in levels() {
        if let Some(button) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &format!("Play Select/Main/Sidebar/List/{}", other.id)) {
            let selected = other.id == id;
            let border = if selected { UiColor::WHITE } else { UiColor::TRANSPARENT };
            let text = if selected { UiColor::WHITE } else { UiColor::Rgb(200, 200, 200) };
            button.update_style(|s| s.set_border_color(border).set_text_color(text));
        }
    }
    app.ui.has_changed = true;
}

/// A scrollable list of level buttons on the left (see `set_scrollable`'s own
/// doc comment - same pattern as the Controller settings list, just Pixels-
/// width instead of a sidebar-wide Grow) and a details panel on the right:
/// a placeholder preview box (no real level art exists yet - see
/// `assets/levels.ron`'s own doc comment), name, and description. Below both
/// (see `footer_button`), a single full-width "Actions" bar holds "Back" and
/// "Play" together - separated from the Sidebar/Content pair above it by a
/// top border, rather than "Back" living in the Sidebar and "Play" pinned to
/// Content's own bottom-right corner as if they belonged to two unrelated
/// sections.
fn play_select_ui(app: &mut App) -> UiNode {
    let mut level_list = UiNode::container()
        .set_orientation(Orientation::Vertical)
        .set_background_color(UiColor::TRANSPARENT)
        .set_size(SizeValue::Grow, SizeValue::Grow)
        .set_gap(6.0)
        .set_scrollable(true);
    for level in levels() {
        let id = level.id.clone();
        let row = sidebar_button(app, &level.name, move |app| show_level(app, &id));
        level_list = level_list.set_child(level.id, row);
    }

    UiNode::container()
        .set_orientation(Orientation::Vertical)
        .set_background_color(UiColor::TRANSPARENT)
        .set_size(SizeValue::Grow, SizeValue::Grow)
        .set_child("Main",
            UiNode::container()
                .set_background_color(UiColor::TRANSPARENT)
                .set_size(SizeValue::Grow, SizeValue::Grow)
                .set_orientation(Orientation::Horizontal)
                .set_child("Sidebar",
                    UiNode::container()
                        .set_orientation(Orientation::Vertical)
                        .set_background_color(UiColor::TRANSPARENT)
                        // A bit wider than Settings' own 240px sidebar - level
                        // names run longer than a settings tab label.
                        .set_size(SizeValue::Pixels(320.0), SizeValue::Grow)
                        .set_padding([60.0, 20.0, 40.0, 20.0])
                        .set_child_anchor(Anchor::Start, Anchor::Start)
                        .set_gap(16.0)
                        .set_child("Title", label(app, "Select Mission").set_font_size(&mut app.ui.text.font_system, 32.0).set_text_color(UiColor::WHITE))
                        .set_child("List", level_list)
                )
                .set_child("Content",
                    UiNode::container()
                        .set_orientation(Orientation::Vertical)
                        .set_background_color(UiColor::TRANSPARENT)
                        .set_size(SizeValue::Grow, SizeValue::Grow)
                        .set_padding([60.0, 20.0, 20.0, 60.0])
                        .set_child_anchor(Anchor::Start, Anchor::Start)
                        .set_gap(16.0)
                        .set_child("Image",
                            UiNode::container()
                                .set_background_color(UiColor::Rgba(255, 255, 255, 10))
                                .set_border_color(UiColor::Rgba(255, 255, 255, 30))
                                .set_border_width(1.0)
                                .set_corner_radius(10.0)
                                .set_size(SizeValue::Grow, SizeValue::Pixels(260.0))
                                .set_child_anchor(Anchor::Center, Anchor::Center)
                                .set_child("Placeholder", label(app, "No preview available").set_text_color(UiColor::Rgb(140, 140, 140))))
                        .set_child("Name", label(app, "").set_size(SizeValue::Grow, SizeValue::Fit).set_font_size(&mut app.ui.text.font_system, 28.0).set_text_color(UiColor::WHITE))
                        .set_child("Description", label(app, "").set_size(SizeValue::Grow, SizeValue::Fit).set_text_color(UiColor::Rgb(200, 200, 200)))
                )
        )
        .set_child("Actions",
            UiNode::container()
                .set_orientation(Orientation::Horizontal)
                .set_background_color(UiColor::TRANSPARENT)
                .set_border_edges(BorderEdges::TOP)
                .set_border_width(1.0)
                .set_border_color(UiColor::Rgba(255, 255, 255, 25))
                .set_size(SizeValue::Grow, SizeValue::Fit)
                .set_padding([20.0, 20.0, 60.0, 60.0])
                .set_child_anchor(Anchor::Start, Anchor::Center)
                .set_child("Back", footer_button(app, "Back", |app| show_panel(app, "Main Menu")))
                .set_child("Spacer", grow_spacer())
                .set_child("Play", footer_button(app, "Play", |app| {
                    if let Some(selected) = SELECTED_LEVEL.lock().unwrap().clone() {
                        app.scene_manager.open_scene(&selected.scene);
                    }
                }))
        )
        .active(false)
}

/// "Back"/"Play" in the Play screen's shared bottom "Actions" bar (see
/// `play_select_ui`) - one shared style so the two read as equally-weighted
/// peers instead of two differently-sized buttons that happen to be near
/// each other. Generous horizontal padding (wider than `sidebar_button`'s
/// own) - "Play" specifically was clipping its own leading "P" against
/// `with_left_accent`'s left border at the tighter padding this used to have
/// - but only vertical padding actually affects button *height*, so that
/// stays close to `sidebar_button`'s own (6.0) rather than matching the
/// horizontal bump 1:1.
fn footer_button(app: &mut App, text: &str, on_click: impl Fn(&mut App) + 'static) -> UiNode {
    with_left_accent(button(app, text, on_click).set_padding((28.0, 7.0)))
}

pub fn build(app: &mut App) {
    let main_menu = UiNode::container()
        .set_size(SizeValue::Grow, SizeValue::Grow)
        .set_corner_radius(0.0)
        .set_background_color(UiColor::TRANSPARENT)
        .set_child(
            "Buttons",
            UiNode::container()
                .set_orientation(Orientation::Vertical)
                .set_size(SizeValue::Fit, SizeValue::Fit)
                .set_position(PositionValue::Start(40.0), PositionValue::Center(0.0))
                .set_child("title", label(app, "Project Skies").set_font_size(&mut app.ui.text.font_system, 50.0))
                .set_child("Play", with_left_accent(button(app, "Play", |app| show_panel(app, "Play Select")).set_size(SizeValue::Grow, SizeValue::Fit)))
                .set_child("Settings", with_left_accent(button(app, "Settings", |app| show_panel(app, "Settings")).set_size(SizeValue::Grow, SizeValue::Fit)))
                // Sets App::should_quit rather than std::process::exit(0)
                // directly - see that field's own doc comment for why: a
                // raw exit skipped App::run's own graceful shutdown (physics
                // thread notified, SDL2/wgpu resources actually dropped) and
                // any code main.rs has after its own `app.run()` call.
                .set_child("Quit", with_left_accent(button(app, "Quit", |app| { app.should_quit = true; }).set_size(SizeValue::Grow, SizeValue::Fit)))
        );

    Layer::new(app)
        .set_child("Backdrop", backdrop())
        .set_child("Main Menu", main_menu)
        .set_child("Settings", settings_ui(app, |app| show_panel(app, "Main Menu")))
        .set_child("Play Select", play_select_ui(app))
        .set_child("RebindModal", rebind_modal::build(app))
        .build(app);
    app.ui.always_on_bottom.push("Backdrop".to_owned());
    app.ui.always_on_top.push("RebindModal".to_owned());
    // Settings opens on the name list (List is active, Content is not). A
    // section is shown only after its name is chosen.
    // Selects the first level so Content/Play aren't empty/dead the first time
    // the player opens the Play screen, before ever clicking a level button.
    if let Some(first) = levels().first() {
        show_level(app, &first.id);
    }
}
