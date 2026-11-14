//! The play scene's debug tools - everything for looking under the hood,
//! none of it part of the game itself:
//! - F3: `debug_panel` - fps, the player's position and bank, with the
//!   debug console;
//! - F5: the engine's physics view - colliders and forces;
//! - F7: `overlay` - the egui debug windows (flight rates, the aero and
//!   head-tracking tools...), with the mouse freed to click them;
//! - F8: `heat_view` - what an IR seeker sees.
//!
//! Their state is `DebugTools`, the scene's `debug`. Everything is
//! re-exported here.

mod debug_panel;
mod heat_view;
mod overlay;

pub use debug_panel::DEBUG_PANEL;
pub use overlay::*;

use super::*;

/// Which debug tools are on.
#[derive(Default)]
pub struct DebugTools {
    // F7 - the debug windows are up, with the mouse freed for them.
    pub(super) mouse_free: bool,
    // The F7 aero overlay is up.
    pub(super) aero_overlay: bool,
    // Which of the debug windows are open.
    pub(super) windows: DebugWindows,
    // F8's heat view is on - see update_heat_view.
    pub(super) heat_view: bool,
}

/// Puts up the debug tools' UI (F3's panel), hidden.
pub(super) fn spawn(scene: &mut Scene, app: &mut App) {
    debug_panel::spawn(scene, app);
}

impl Play {
    /// The debug keys - every frame, paused or not: F5's physics view,
    /// F3's panel with the console, F7's windows and the mouse for them.
    pub(super) fn update_debug_tools(&mut self, scene: &mut Scene, app: &mut App) {
        if app.input.is_action_just_pressed("toggle_debug") {
            app.renderer.debug.physics = !app.renderer.debug.physics;
        }
        self.show_debug_panel(scene);
        // The pause menu owns the cursor while it's up - close_pause_menu
        // puts back whichever mode this left.
        if app.input.is_action_just_pressed("toggle_debug_mouse") {
            self.debug.mouse_free = !self.debug.mouse_free;
            if !app.time.is_paused() {
                app.window.set_cursor_locked(!self.mouse_free());
            }
        }
    }

    /// Whether the F7 debug overlay is up.
    pub(super) fn debug_overlay_visible(&self) -> bool {
        self.debug.aero_overlay || self.debug.mouse_free
    }

    /// Whether any egui window is up - F7's, F10's editor or map, F11's HUD
    /// layout, the spectating telemetry. The engine's debug UI is shown (and
    /// `debug_ui` run) only while one is.
    pub(super) fn any_debug_window(&self) -> bool {
        self.debug_overlay_visible()
            || self.tactical.tactical_map_active
            || self.editor.editor_active
            || self.editor.spectate_telemetry.is_some()
            || self.hud.hud_layout.open
    }

    /// The scene's egui windows - see `overlay`.
    pub(super) fn draw_debug_ui(&mut self, ctx: &egui::Context, scene: &mut Scene, app: &mut App) {
        let was_hud_panel_open = self.hud.hud_layout.open;
        let player = self.flight_manager.player_id().to_owned();
        let view = DebugView {
            show_debug: self.debug_overlay_visible(),
            mouse_free: self.debug.mouse_free,
            player: &player,
            windows: &mut self.debug.windows,
            flight_editor: &mut self.editor.flight_editor,
            tactical_ui: &mut self.tactical.tactical_ui,
            hud_layout: &mut self.hud.hud_layout,
            editor_active: self.editor.editor_active,
            tactical_map_active: self.tactical.tactical_map_active,
            spectate_telemetry: self.editor.spectate_telemetry.clone(),
        };
        overlay::draw(ctx, view, scene, app);
        // The HUD layout panel closed from its own button: mouse-look back.
        if was_hud_panel_open && !self.hud.hud_layout.open && !app.time.is_paused() {
            app.window.set_cursor_locked(!self.mouse_free());
        }
    }
}
