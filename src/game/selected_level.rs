//! The one piece of state that has to cross from `main_menu` (where the
//! player picks a level in Play Select) into `play` (where the chosen scene
//! actually opens) - `SceneManager::open_scene` only takes a bare scene name
//! (see `engine::scene_manager::scene::SceneManager::open_scene`), with no way
//! to pass extra data into the new scene's constructor, so this static is the
//! only channel `play::scene::GameLogic::new` has for "which level did the
//! player actually pick, and what should its mission-intro card show".

use std::sync::Mutex;

#[derive(Clone)]
pub struct SelectedLevel {
    pub scene: String,
    pub mission_title: String,
    pub location: String,
    pub mission_date: String,
}

/// Set by `main_menu::ui::show_level` every time the player picks a different
/// level in Play Select; read (and left alone - not cleared) by
/// `play::scene::GameLogic::new` when the chosen scene actually opens.
pub static SELECTED_LEVEL: Mutex<Option<SelectedLevel>> = Mutex::new(None);
