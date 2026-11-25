//! The death screen - the "death_screen" element: the screen gone black,
//! "YOU DIED", Restart and Back to Main Menu. Not shown for now (see
//! Play::check_crash).

use pankarta_software::app::App;
use pankarta_software::prelude::Scene;
use pankarta_software::ui::color::UiColor;
use pankarta_software::ui::ui_node::UiNode;
use pankarta_software::ui::ui_transform::{Orientation, PositionValue, SizeValue};

use crate::ext::ui_layers::{self, GameUi};
use crate::ui::{button, label, with_left_accent};

/// The death screen's node.
pub const DEATH_SCREEN: &str = "death_screen";

/// Shows the death screen - at once, no fade: it should read as the screen
/// going black. Frees the cursor to click its buttons.
#[allow(dead_code)] // not shown for now - see play::Play::check_crash
pub fn open_death_screen(ui: &mut impl GameUi, app: &mut App) {
    app.window.set_cursor_locked(false);
    ui.set_ui_visible(DEATH_SCREEN, true);
}

/// Closes it - only right before its buttons switch scenes, to leave the
/// cursor as flying wants it.
fn close_death_screen(ui: &mut impl GameUi, app: &mut App) {
    app.window.set_cursor_locked(true);
    ui.set_ui_visible(DEATH_SCREEN, false);
}

/// Puts the death screen up, hidden. Solid black, not the pause menu's
/// see-through gradient.
pub(super) fn spawn(scene: &mut Scene, app: &mut App) {
    let backdrop = UiNode::container()
        .set_size(SizeValue::Percent(100.0), SizeValue::Percent(100.0))
        .set_position(PositionValue::Start(0.0), PositionValue::Start(0.0))
        .set_background_color(UiColor::Rgba(0, 0, 0, 255));

    let panel = UiNode::container()
        .set_orientation(Orientation::Vertical)
        .set_size(SizeValue::Pixels(360.0), SizeValue::Fit)
        .set_position(PositionValue::Center(0.0), PositionValue::Center(0.0))
        .set_background_color(UiColor::TRANSPARENT)
        .set_child("title", label("YOU DIED").set_font_size(50.0))
        .set_child("Restart", with_left_accent(button("Restart", |ui, app| {
            close_death_screen(ui, app);
            app.scene_manager.open_scene("playing");
        }).set_size(SizeValue::Grow, SizeValue::Fit)))
        .set_child("MainMenu", with_left_accent(button("Back to Main Menu", |ui, app| {
            close_death_screen(ui, app);
            app.scene_manager.open_scene("main_menu");
        }).set_size(SizeValue::Grow, SizeValue::Fit)));

    let screen = ui_layers::screen_root()
        .set_child("Backdrop", backdrop)
        .set_child("Panel", panel);
    ui_layers::add(scene, app, DEATH_SCREEN, screen);
    scene.ui_on_top(DEATH_SCREEN);
    scene.set_ui_visible(DEATH_SCREEN, false);
}
