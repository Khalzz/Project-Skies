//! The game's frame plans (see engine::rendering::frame) and the line and
//! text sets they draw - what a frame of each scene shows, in order.

use crate::app::App;
use crate::engine::rendering::frame::FramePlan;
use crate::engine::rendering::render_pipeline::render_pass::{color_attachment, UiLayer};
use crate::engine::rendering::render_sets::RenderSets;
use crate::engine::rendering::shader_library::Overlay;

/// Every scene without a plan of its own (the menu, the loading screen...):
/// the engine's default - the world, the UI over it - and the game's debug
/// windows over everything.
pub fn standard() -> FramePlan {
    FramePlan::engine_default()
        .custom("debug_overlay", App::render_aero_debug_overlay_pass)
}

/// Flying: the world, then the flight HUD into its own image, then the
/// composite copying the world and that HUD onto the screen with the G-force
/// effects over both; menus after, so a blackout doesn't cover them.
pub fn flight() -> FramePlan {
    FramePlan::new()
        .scene("opaque")
        .copy_depth()
        .scene("water")
        .scene("transparent")
        .particles()
        .target("hud", true)
            .lines("hud_lines")
            .lines("hud_screen_lines")
            .lines("hud_horizon_lines")
            .text("hud_text")
            .text("hmd_text")
            .ui(UiLayer::Main)
        .end_target()
        .fullscreen(crate::game::screen_effects::G_VISION)
        // Heat blobs sit on the graded picture, so desaturation doesn't
        // turn them grey.
        .lines("heat_screen_lines")
        .lines("editor_lines")
        .ui(UiLayer::OnTop)
        // Last, so it's always on top of the game's own HUD too.
        .custom("debug_overlay", App::render_aero_debug_overlay_pass)
}

/// The M map: its lines on a dark screen, the UI over them.
pub fn tactical_map() -> FramePlan {
    FramePlan::new()
        .custom("tactical_map", App::render_tactical_map_pass)
        .ui(UiLayer::All)
        .custom("debug_overlay", App::render_aero_debug_overlay_pass)
}

/// The sets the play scene's plans draw - registered when it starts.
pub fn register_flight_sets(app: &mut App) {
    // In the world.
    for name in ["hud_lines", "hud_horizon_lines", "editor_lines", "tactical_lines"] {
        app.add_line_set(name, false);
    }
    app.add_text_set("hud_text", false);
    // In screen pixels.
    for name in ["hud_screen_lines", "heat_screen_lines"] {
        app.add_line_set(name, true);
    }
    app.add_text_set("hmd_text", true);
}

impl App {
    /// The play scene's F10 tactical map instead of the world: a dark screen
    /// and the lines the game put in the "tactical_lines" set (terrain contours,
    /// aircraft, trails...) - the 3D passes are skipped altogether.
    pub(crate) fn render_tactical_map_pass(&mut self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        const BACKGROUND: wgpu::Color = wgpu::Color { r: 0.010, g: 0.018, b: 0.030, a: 1.0 };
        let camera_position = self.scene_manager.cameras().map(|c| c.active().camera.position()).unwrap_or_else(nalgebra::Point3::origin);
        let full = [self.window_manager.pixel_size.width as f32, self.window_manager.pixel_size.height as f32];
        // Only in the editor view's middle, when its panels are up - in the
        // drawable's pixels, kept inside it.
        let to_pixels = full[0] / (self.window_manager.size.width as f32).max(1.0);
        let viewport = self.tactical_viewport.map(|[x, y, width, height]| {
            let (x, y) = ((x * to_pixels).clamp(0.0, full[0] - 1.0), (y * to_pixels).clamp(0.0, full[1] - 1.0));
            [x, y, (width * to_pixels).clamp(1.0, full[0] - x), (height * to_pixels).clamp(1.0, full[1] - y)]
        });
        let screen = viewport.map(|[_, _, width, height]| [width, height]).unwrap_or(full);
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Tactical map pass"),
            color_attachments: &[color_attachment(view, wgpu::LoadOp::Clear(BACKGROUND))],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        if let Some([x, y, width, height]) = viewport {
            render_pass.set_viewport(x, y, width, height, 0.0, 1.0);
        }
        let Some(lines) = self.render_sets.get_lines_mut("tactical_lines") else { return };
        // With its material (see ShaderLibrary::prepare_overlay).
        let key = RenderSets::lines_key("tactical_lines", lines);
        let Some(material) = self.overlay_materials.get(&key).and_then(|key| self.shaders.overlay(key, Overlay::Lines)) else { return };
        lines.draw(
            &self.renderer.device,
            &self.renderer.queue,
            &mut render_pass,
            material,
            &self.camera_resources.bind_group,
            &self.light.rendering_data.bind_group,
            [camera_position.x, camera_position.y, camera_position.z],
            screen,
        );
    }
}
