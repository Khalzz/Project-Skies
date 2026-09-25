//! Player-facing settings - written by the Settings menu (see
//! `main_menu::ui::settings_ui`, shared with the in-game pause menu), read
//! by gameplay code wherever it needs them. A static for the same reason as
//! `SELECTED_LEVEL`: the menu and the scenes that read it are separate
//! scenes, with no other channel between them.
//!
//! In-memory only for now - resets to the defaults below on every launch
//! (same as rebound controls).

use std::sync::Mutex;

pub struct GameSettings {
    /// Video: the free camera orbits in the plane's own frame (GTA-vehicle
    /// style - behind the plane by default, turning and rolling with it)
    /// instead of the world's (see `play::camera::camera::Camera`'s Free mode).
    pub free_camera_follows_plane: bool,
}

pub static GAME_SETTINGS: Mutex<GameSettings> = Mutex::new(GameSettings {
    free_camera_follows_plane: false,
});
