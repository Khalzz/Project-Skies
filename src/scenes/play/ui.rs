//! The play scene's screen UI - each element one node with a `UiLayer`
//! holding its whole tree (see ext::ui_layers), built here once when the
//! scene opens (`spawn`):
//! - `hud`: the flight HUD ("hud");
//! - `pause_menu`: Escape's menu ("pause_menu", and main_menu's "Settings");
//! - `death_screen`: "death_screen";
//! - `mission_intro`: the black screen and title before the flight;
//! - `captions`: "subtitles" and "skip_prompt";
//! - `kill_feed`: who downed whom, top right.
//!
//! The rest of the scene's UI lives with what it's for: the quick list's
//! with the plane (plane::quick_list, spawned here too), each aircraft's
//! name tag on the aircraft's own node (see flight_manager), F3's debug
//! panel with the other debug tools (see play::debug_tools).
//!
//! Everything is re-exported here: `play_ui::{HUD, open_pause_menu, ...}`.

mod captions;
mod death_screen;
mod hud;
mod kill_feed;
mod mission_intro;
mod pause_menu;

pub use captions::*;
pub use death_screen::*;
pub use hud::*;
pub use kill_feed::*;
pub use mission_intro::*;
pub use pause_menu::*;

use pankarta_software::app::App;
use pankarta_software::prelude::Scene;

use crate::ext::ui_layers;
use crate::prefabs::plane::quick_list;

/// Puts up every element of the play scene's UI.
pub fn spawn(scene: &mut Scene, app: &mut App) {
    hud::spawn(scene, app);
    kill_feed::spawn(scene, app);
    captions::spawn(scene, app);
    ui_layers::add(scene, app, quick_list::QUICK_LIST_UI, quick_list::build_quick_list_ui());
    pause_menu::spawn(scene, app);
    death_screen::spawn(scene, app);
    mission_intro::spawn(scene, app);
}
