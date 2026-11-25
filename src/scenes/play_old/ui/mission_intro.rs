//! The mission intro - the "mission_intro" element: a black screen and the
//! mission's title, place and date over it, faded in and out by the level's
//! tracks ("mission_intro/backdrop", "mission_intro/card/title"...) before
//! the flight starts.

use pankarta_software::app::App;
use pankarta_software::prelude::Scene;
use pankarta_software::ui::color::UiColor;
use pankarta_software::ui::ui_node::UiNode;
use pankarta_software::ui::ui_transform::{Anchor, Orientation, PositionValue, SizeValue};

use crate::ext::ui_layers::{self, GameUi};
use crate::selected_level::SELECTED_LEVEL;
use crate::ui::label;

/// The mission intro's node.
pub const MISSION_INTRO: &str = "mission_intro";

/// Puts the selected level's intro up - shown from the start, over
/// everything but the subtitles. Its text starts see-through.
pub(super) fn spawn(scene: &mut Scene, app: &mut App) {
    let selected = SELECTED_LEVEL.lock().unwrap().clone();
    let (title, location, date) = match selected {
        Some(level) => (level.mission_title, level.location, level.mission_date),
        None => ("Unknown Mission".to_owned(), String::new(), String::new()),
    };
    let line = |text: &str, size: f32, color: UiColor| {
        label(text).set_font_size(size).set_text_color(color.with_alpha(0.0))
    };

    let backdrop = UiNode::container()
        .set_size(SizeValue::Percent(100.0), SizeValue::Percent(100.0))
        .set_position(PositionValue::Start(0.0), PositionValue::Start(0.0))
        .set_background_color(UiColor::Rgba(0, 0, 0, 255));
    let card = UiNode::container()
        .set_orientation(Orientation::Vertical)
        .set_size(SizeValue::Fit, SizeValue::Fit)
        .set_position(PositionValue::Start(60.0), PositionValue::Center(0.0))
        .set_background_color(UiColor::TRANSPARENT)
        .set_child_anchor(Anchor::Start, Anchor::Start)
        .set_gap(8.0)
        .set_child("title", line(&title, 34.0, UiColor::WHITE))
        .set_child("location", line(&location, 20.0, UiColor::Rgb(210, 210, 210)))
        .set_child("date", line(&date, 16.0, UiColor::Rgb(170, 170, 170)));

    let intro = ui_layers::screen_root()
        .set_child("backdrop", backdrop)
        .set_child("card", card);
    ui_layers::add(scene, app, MISSION_INTRO, intro);
    scene.ui_on_top(MISSION_INTRO);
}
