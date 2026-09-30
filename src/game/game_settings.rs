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
    /// Video: how strongly the cockpit camera's head reacts to acceleration/G -
    /// 1.0 = 100% (the tuning in `play::camera::head_motion::HEAD_MOTION`),
    /// 0.0 = a fully static head. Set by a slider, 0..HEAD_G_REACTION_MAX.
    pub head_g_reaction: f32,
    /// Video: the cockpit view's resting up/down angle, degrees - positive
    /// looks up over the nose, negative down at the instruments. Set by a
    /// slider, +-COCKPIT_VIEW_PITCH_MAX.
    pub cockpit_view_pitch_deg: f32,
}

/// The ends of the "Cockpit view angle" slider (degrees, either way).
pub const COCKPIT_VIEW_PITCH_MAX: f32 = 20.0;

/// The top of the "Cockpit head G reaction" slider (2.0 = 200%).
pub const HEAD_G_REACTION_MAX: f32 = 2.0;

pub static GAME_SETTINGS: Mutex<GameSettings> = Mutex::new(GameSettings {
    free_camera_follows_plane: false,
    head_g_reaction: 1.0,
    cockpit_view_pitch_deg: -6.0,
});
