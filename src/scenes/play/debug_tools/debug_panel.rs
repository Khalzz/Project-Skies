//! F3's debug panel - the "debug_panel" element: fps, the player's position
//! and bank, shown with the debug console.
//!
//! Single-line labels stacked in a box, not one multi-line label - a label
//! centers around one line's height, so a multi-line one pushes its first
//! lines out of its own bounds.

use glyphon::cosmic_text::Align;
use pankarta_software::ui::ui_transform::{PositionValue, SizeValue};

use super::super::*;
use crate::ui::label;

/// The debug panel's node.
pub const DEBUG_PANEL: &str = "debug_panel";

/// Puts it up, hidden.
pub(super) fn spawn(scene: &mut Scene, app: &mut App) {
    let stats_line = |text: &str| {
        label(text)
            .set_size(SizeValue::Pixels(320.0), SizeValue::Pixels(20.0))
            .set_font_size(14.0)
            .set_text_color(UiColor::Rgb(0, 255, 0))
            .set_align(Align::Left)
    };
    let stats_box = UiNode::container()
        .set_size(SizeValue::Fit, SizeValue::Fit)
        .set_position(PositionValue::Start(10.0), PositionValue::Start(10.0))
        .set_padding(20.0)
        .set_gap(5.0)
        .set_background_color(UiColor::Rgba(0, 0, 0, 128))
        .set_background_blur(20.0)
        .set_child("fps", stats_line("0 FPS"))
        .set_child("position", stats_line("Player position: (0, 0, 0)"))
        .set_child("bank", stats_line("Bank: 0°"));
    let panel = ui_layers::screen_root()
        .set_child("stats_box", stats_box)
        .active(false);
    ui_layers::add(scene, app, DEBUG_PANEL, panel);
}

impl Play {
    /// Shown while the debug console is - every frame.
    pub(super) fn show_debug_panel(&self, scene: &mut Scene) {
        if let Some(node) = scene.ui_node(DEBUG_PANEL) {
            node.set_active(debug_console::is_console_visible());
        }
    }

    /// Its readouts - with the HUD's text (see Play::ui_control).
    pub(in crate::scenes::play) fn update_debug_panel(&self, scene: &mut Scene, app: &App) {
        let player_id = self.flight_manager.player_id();
        if let Some(label) = scene
            .ui_node("debug_panel/stats_box/fps")
            .and_then(|n| n.as_label_mut())
        {
            label.set_text(&format!("{:.0} FPS", app.time.fps()));
        }
        let Some(transform) = crate::ext::drawn::get(scene, player_id).map(|i| i.instance.transform)
        else {
            return;
        };
        let pos = transform.position;
        if let Some(label) = scene
            .ui_node("debug_panel/stats_box/position")
            .and_then(|n| n.as_label_mut())
        {
            label.set_text(&format!(
                "Player position: ({:.1}, {:.1}, {:.1})",
                pos.x, pos.y, pos.z
            ));
        }
        let text = match bank_angle(&transform.rotation) {
            Some(bank) => format!(
                "Bank: {bank:.1}° ({})",
                if bank.abs() < 0.5 {
                    "level"
                } else if bank > 0.0 {
                    "right wing down"
                } else {
                    "left wing down"
                }
            ),
            None => "Bank: - (nose straight up/down)".to_owned(),
        };
        if let Some(label) = scene
            .ui_node("debug_panel/stats_box/bank")
            .and_then(|n| n.as_label_mut())
        {
            label.set_text(&text);
        }
    }
}
