//! The game's frame plans (see the engine's rendering::frame) and the line
//! and text sets they draw - what a frame of each scene shows, in order.
//!
//! The game's debug windows (egui) aren't a step any more - the engine draws
//! its `DebugUi` over everything itself (see each scene's
//! `SceneBehaviour::debug_ui`).

use std::sync::Mutex;

use pankarta_software::app::App;
use pankarta_software::rendering::frame::FramePlan;
use pankarta_software::rendering::render_pipeline::render_pass::UiPass;
use pankarta_software::rendering::render_sets::RenderSets;
use pankarta_software::rendering::shader_library::Overlay;

/// Every scene without a plan of its own (the menu, the loading screen...):
/// the engine's default - the world, the UI over it.
pub fn standard() -> FramePlan {
    FramePlan::engine_default()
        .insert_after_scene("opaque", |steps| steps.copy_depth().scene("water"))
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
            .ui(UiPass::Main)
        .end_target()
        .fullscreen(crate::game::screen_effects::G_VISION)
        // Heat blobs sit on the graded picture, so desaturation doesn't
        // turn them grey.
        .lines("heat_screen_lines")
        .lines("editor_lines")
        .nodes_3d()
        .nodes_2d()
        .ui(UiPass::OnTop)
}

/// The M map: its lines on a dark screen, the UI over them.
pub fn tactical_map() -> FramePlan {
    FramePlan::new()
        .custom("tactical_map", render_tactical_map_pass)
        .ui(UiPass::All)
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

/// Where F10's map is drawn (logical px: x, y, width, height) - the middle
/// of the editor view's layout, between its panels. None: the whole screen.
/// Set by the play scene's debug UI each frame, read by the map's pass.
static TACTICAL_VIEWPORT: Mutex<Option<[f32; 4]>> = Mutex::new(None);

pub fn set_tactical_viewport(viewport: Option<[f32; 4]>) {
    if let Ok(mut slot) = TACTICAL_VIEWPORT.lock() {
        *slot = viewport;
    }
}

pub fn tactical_viewport() -> Option<[f32; 4]> {
    TACTICAL_VIEWPORT.lock().ok().and_then(|slot| *slot)
}

/// Where the map's camera is - its lines are drawn relative to it. Set by
/// the play scene once its cameras are placed (the engine keeps the active
/// scene to itself while it draws).
static TACTICAL_CAMERA: Mutex<[f32; 3]> = Mutex::new([0.0; 3]);

pub fn set_tactical_camera_position(position: nalgebra::Vector3<f32>) {
    if let Ok(mut slot) = TACTICAL_CAMERA.lock() {
        *slot = [position.x, position.y, position.z];
    }
}

fn tactical_camera_position() -> nalgebra::Point3<f32> {
    let [x, y, z] = TACTICAL_CAMERA.lock().map(|slot| *slot).unwrap_or_default();
    nalgebra::Point3::new(x, y, z)
}

/// The play scene's tactical map instead of the world: a dark screen and the
/// lines the game put in the "tactical_lines" set (terrain contours,
/// aircraft, trails...) - the 3D passes are skipped altogether.
fn render_tactical_map_pass(app: &mut App, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
    const BACKGROUND: wgpu::Color = wgpu::Color { r: 0.010, g: 0.018, b: 0.030, a: 1.0 };
    let camera_position = tactical_camera_position();
    let full = [app.window.pixel_size.width as f32, app.window.pixel_size.height as f32];
    // Only in the editor view's middle, when its panels are up - in the
    // drawable's pixels, kept inside it.
    let to_pixels = full[0] / (app.window.size.width as f32).max(1.0);
    let viewport = tactical_viewport().map(|[x, y, width, height]| {
        let (x, y) = ((x * to_pixels).clamp(0.0, full[0] - 1.0), (y * to_pixels).clamp(0.0, full[1] - 1.0));
        [x, y, (width * to_pixels).clamp(1.0, full[0] - x), (height * to_pixels).clamp(1.0, full[1] - y)]
    });
    let screen = viewport.map(|[_, _, width, height]| [width, height]).unwrap_or(full);
    let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Tactical map pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            resolve_target: None,
            ops: wgpu::Operations { load: wgpu::LoadOp::Clear(BACKGROUND), store: wgpu::StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        occlusion_query_set: None,
        timestamp_writes: None,
    });
    if let Some([x, y, width, height]) = viewport {
        render_pass.set_viewport(x, y, width, height, 0.0, 1.0);
    }
    let renderer = &mut app.renderer;
    let Some(lines) = renderer.render_sets.get_lines_mut("tactical_lines") else { return };
    // With its material (see the engine's ShaderLibrary::prepare_overlay).
    let key = RenderSets::lines_key("tactical_lines", lines);
    let Some(material) = renderer.overlay_materials.get(&key).and_then(|key| renderer.shaders.overlay(key, Overlay::Lines)) else { return };
    lines.draw(
        &renderer.device,
        &renderer.queue,
        &mut render_pass,
        material,
        &renderer.camera_resources.bind_group,
        &renderer.light.rendering_data.bind_group,
        [camera_position.x, camera_position.y, camera_position.z],
        screen,
    );
}
