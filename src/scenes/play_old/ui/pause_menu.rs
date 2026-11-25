//! The pause menu (Escape) - the "pause_menu" element: main_menu's gradient
//! backdrop and the buttons over it - plus "Settings", main_menu's settings
//! panel (its own element: its code reaches it as "Settings/...").

use pankarta_software::app::App;
use pankarta_software::prelude::Scene;
use pankarta_software::ui::color::UiColor;
use pankarta_software::ui::ui_node::UiNode;
use pankarta_software::ui::ui_transform::{Orientation, PositionValue, SizeValue};

use super::hud::HUD;
use crate::ext::ui_layers::{self, GameUi};
use crate::scenes::main_menu::ui as main_menu_ui;
use crate::ui::{button, label, with_left_accent};

/// The pause menu's node.
pub const PAUSE_MENU: &str = "pause_menu";
/// Its buttons' panel.
const BUTTONS: &str = "pause_menu/Buttons";
/// main_menu's settings panel, shown from the pause menu.
const SETTINGS: &str = "Settings";

/// Shows the buttons or the settings - the backdrop stays up for both.
fn show_pause_panel(ui: &mut impl GameUi, _app: &mut App, panel: &str) {
    for id in [BUTTONS, SETTINGS] {
        if let Some(node) = ui.ui_node(id) {
            node.set_active(id == panel);
        }
    }
}

/// Brings the pause menu up - time stops (time scale 0: no physics, no
/// Plane::update), the cursor's freed to click, the flight HUD's hidden.
/// Called from the "toggle_pause_menu" key (see Play::update).
pub fn open_pause_menu(ui: &mut impl GameUi, app: &mut App) {
    app.time.time_scale = 0.0;
    app.window.set_cursor_locked(false);
    ui.set_ui_visible(PAUSE_MENU, true);
    show_pause_panel(ui, app, BUTTONS);
    // The gradient fades out toward the right, where the HUD's boxes sit.
    // The layer is hidden, not its nodes, so whichever parts were showing
    // come back as they were.
    ui.set_ui_visible(HUD, false);
}

/// Closes the pause menu, whichever panel was showing - Resume, the key
/// again, and Restart/Back to menu (closed first, so the next scene doesn't
/// start with time stopped).
pub fn close_pause_menu(ui: &mut impl GameUi, app: &mut App) {
    app.time.time_scale = 1.0;
    // Mouse-look back (the play scene frees it again next frame if a debug
    // window wants it).
    app.window.set_cursor_locked(true);
    ui.set_ui_visible(PAUSE_MENU, false);
    if let Some(settings) = ui.ui_node(SETTINGS) {
        settings.set_active(false);
    }
    ui.set_ui_visible(HUD, true);
}

/// Puts the pause menu up, hidden.
pub(super) fn spawn(scene: &mut Scene, app: &mut App) {
    let buttons = UiNode::container()
        .set_orientation(Orientation::Vertical)
        // Fixed width, not Fit - a Fit container sizes itself around its
        // children's natural size, and the Grow buttons don't give one, so
        // it would be as narrow as the title, clipping "Back to Main Menu".
        .set_size(SizeValue::Pixels(320.0), SizeValue::Fit)
        .set_position(PositionValue::Start(40.0), PositionValue::Center(0.0))
        .set_background_color(UiColor::TRANSPARENT)
        .set_child("title", label("Paused").set_font_size(50.0))
        .set_child("Resume", with_left_accent(button("Resume", |ui, app| close_pause_menu(ui, app)).set_size(SizeValue::Grow, SizeValue::Fit)))
        .set_child("Restart", with_left_accent(button("Restart", |ui, app| {
            close_pause_menu(ui, app);
            app.scene_manager.open_scene("playing");
        }).set_size(SizeValue::Grow, SizeValue::Fit)))
        .set_child("Settings", with_left_accent(button("Settings", |ui, app| show_pause_panel(ui, app, SETTINGS)).set_size(SizeValue::Grow, SizeValue::Fit)))
        .set_child("MainMenu", with_left_accent(button("Back to Main Menu", |ui, app| {
            close_pause_menu(ui, app);
            app.scene_manager.open_scene("main_menu");
        }).set_size(SizeValue::Grow, SizeValue::Fit)));
    let menu = ui_layers::screen_root()
        .set_child("Backdrop", main_menu_ui::backdrop())
        .set_child("Buttons", buttons);
    ui_layers::add(scene, app, PAUSE_MENU, menu);
    scene.ui_on_top(PAUSE_MENU);
    scene.set_ui_visible(PAUSE_MENU, false);

    let settings = main_menu_ui::settings_ui(app, |ui, app| show_pause_panel(ui, app, BUTTONS)).active(false);
    ui_layers::add(scene, app, SETTINGS, settings);
    // Over the backdrop.
    if let Some(layer) = scene.ui_layer(SETTINGS) {
        layer.set_z_index(ui_layers::ON_TOP + 1);
    }
}
