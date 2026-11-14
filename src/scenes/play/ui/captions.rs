//! The text shown over a scripted sequence, each its own element:
//! - "subtitles": the dialogue's lines (ext::subtitles writes them, and
//!   fades them - it rests see-through);
//! - "skip_prompt": "Hold ESC to skip", shown while the player's input is
//!   locked (see Play::update).

use glyphon::cosmic_text::Align;
use pankarta_software::app::App;
use pankarta_software::prelude::Scene;
use pankarta_software::ui::color::UiColor;
use pankarta_software::ui::ui_transform::{PositionValue, SizeValue};

use crate::ext::subtitles::SUBTITLES_PATH;
use crate::ext::ui_layers::{self, GameUi};
use crate::ui::label;

/// The skip prompt's node.
pub const SKIP_PROMPT: &str = "skip_prompt";

/// Puts both up - the subtitles over the mission intro.
pub(super) fn spawn(scene: &mut Scene, app: &mut App) {
    let subtitles = label("")
        .set_size(SizeValue::Pixels(800.0), SizeValue::Pixels(35.0))
        .set_position(PositionValue::Center(0.0), PositionValue::End(-150.0))
        .set_text_color(UiColor::Rgba(255, 255, 255, 0))
        .set_background_color(UiColor::TRANSPARENT)
        .set_align(Align::Center);
    ui_layers::add(scene, app, SUBTITLES_PATH, subtitles);
    scene.ui_on_top(SUBTITLES_PATH);

    let skip_prompt = label("Hold ESC to skip")
        .set_size(SizeValue::Pixels(300.0), SizeValue::Pixels(30.0))
        .set_position(PositionValue::Center(0.0), PositionValue::End(-40.0))
        .set_text_color(UiColor::Rgba(255, 255, 255, 180))
        .set_background_color(UiColor::TRANSPARENT)
        .set_align(Align::Center)
        .active(false);
    ui_layers::add(scene, app, SKIP_PROMPT, skip_prompt);
}
