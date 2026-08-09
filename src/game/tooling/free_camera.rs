use std::f32::consts::PI;
use std::sync::Mutex;

use glyphon::{cosmic_text::Align, Color};
use nalgebra::Vector3;

use crate::app::App;
use crate::engine::input::input;
use crate::engine::ui::ui_node::{UiNode, UiNodeContent};

const FREE_CAMERA_NAME: &str = "__free_camera_tool";

// One key per line rather than a single multi-line label - Label's line height is
// hardcoded smaller than its glyph size (see Label::new), so multi-line text in one
// buffer renders with lines overlapping each other. Stacking separate single-line
// labels (same pattern game_ui.ron's HUD elements already use) sidesteps that
// entirely, and lets us position each line's box exactly.
const HUD_LINE_KEYS: [&str; 4] = [
    "__free_camera_hud_title",
    "__free_camera_hud_pos",
    "__free_camera_hud_angles",
    "__free_camera_hud_fov",
];
const HUD_X: f32 = 16.0;
const HUD_Y: f32 = 16.0;
const HUD_WIDTH: f32 = 360.0;
const HUD_LINE_BOX_HEIGHT: f32 = 28.0;
const HUD_LINE_SPACING: f32 = 32.0;

// Toggle hint - always shown (enabled or not), so it lives below where the 4-line
// HUD block ends instead of overlapping it.
const HINT_KEY: &str = "__free_camera_hint";
const HINT_Y: f32 = HUD_Y + HUD_LINE_SPACING * 4.0;

const MOVE_SPEED: f32 = 15.0;
const SPRINT_MULTIPLIER: f32 = 3.0;
const MAX_PITCH_DEG: f32 = 89.0;
const FOV_SCROLL_SPEED: f32 = 5.0;
const FOV_MIN: f32 = 10.0;
const FOV_MAX: f32 = 120.0;
// input::mouse_sensitivity() is tuned for the Free/Cockpit chase-cam modes, which
// tolerate much bigger per-pixel angle jumps than a direct look-camera does - scaled
// down here so a normal mouse swipe doesn't turn the view 90 degrees in one sample.
const LOOK_SENSITIVITY_SCALE: f32 = 0.5;
// How long handing control back to the previous camera takes to blend, instead of
// an instant cut - see CameraHandler::transition_to. Purely cosmetic, so no
// particular reasoning behind this exact number beyond "feels quick but not jarring".
const HANDOFF_TRANSITION_SECS: f32 = 0.75;

struct FreeCameraState {
    enabled: bool,
    // Which camera to hand control back to once this tool is toggled off.
    previous_camera: Option<String>,
    // Cursor visibility to restore once this tool is toggled off, in case it wasn't
    // showing to begin with (e.g. relative mouse mode already hides it in gameplay).
    cursor_was_showing: Option<bool>,
}

static STATE: Mutex<FreeCameraState> = Mutex::new(FreeCameraState { enabled: false, previous_camera: None, cursor_was_showing: None });

/// Dev-only free-fly camera tool for building/testing this game - toggled with the
/// "toggle_camera_debug" action (F4), opt-in per scene by calling this once per frame
/// from that scene's own update (see main_menu.rs). Doesn't touch whatever camera the
/// scene already has active: it generates its own dedicated camera and switches to it
/// while enabled (seeded from the current view, so the switch itself is invisible),
/// then smoothly blends control back to whatever was active before once toggled back
/// off (see CameraHandler::transition_to/HANDOFF_TRANSITION_SECS) instead of an
/// instant cut - the scene's real camera is never mutated. While active: mouse look + WASD/Space/Left
/// Ctrl fly the camera (hold Left Shift to sprint), scroll adjusts fov, and a HUD
/// shows its live position/yaw/pitch/fov. The system cursor is hidden while active
/// and restored to its prior visibility on toggle-off. A one-line hint showing the
/// toggle button is always on screen, regardless of enabled state.
pub fn update(app: &mut App) {
    let mut state = STATE.lock().unwrap();

    if input::is_action_just_pressed("toggle_camera_debug") {
        state.enabled = !state.enabled;
        if state.enabled {
            let previous_name = app.camera.active_name().to_owned();
            let (pos, yaw_deg, pitch_deg, fovy) = {
                let active = app.camera.active();
                (active.camera.position, active.camera.yaw.to_degrees(), active.camera.pitch.to_degrees(), active.projection.fovy)
            };
            app.camera.create_camera(FREE_CAMERA_NAME, pos, yaw_deg, pitch_deg, fovy);
            app.camera.select_camera(FREE_CAMERA_NAME);
            state.previous_camera = Some(previous_name);

            let mouse = app.window_manager.context.mouse();
            state.cursor_was_showing = Some(mouse.is_cursor_showing());
            mouse.show_cursor(false);
        } else {
            if let Some(previous) = state.previous_camera.take() {
                app.camera.transition_to(&previous, HANDOFF_TRANSITION_SECS);
            }
            if let Some(was_showing) = state.cursor_was_showing.take() {
                app.window_manager.context.mouse().show_cursor(was_showing);
            }
            for key in HUD_LINE_KEYS {
                app.ui.renderizable_elements.remove(key);
            }
            app.ui.has_changed = true;
        }
    }

    set_hint(app, state.enabled);

    if !state.enabled {
        return;
    }

    let delta_time = app.time.delta_time;
    let sens = input::mouse_sensitivity();
    let scroll = input::mouse_scroll_y();
    let active_name = app.camera.active_name().to_owned();
    let active = app.camera.active_mut();

    // Mouse look - mutates yaw/pitch (radians) directly, no smoothing.
    active.camera.yaw += (input::mouse_rel_x() as f32 * sens.0 * LOOK_SENSITIVITY_SCALE).to_radians();
    active.camera.pitch = (active.camera.pitch - (input::mouse_rel_y() as f32 * sens.1 * LOOK_SENSITIVITY_SCALE).to_radians())
        .clamp(-MAX_PITCH_DEG.to_radians(), MAX_PITCH_DEG.to_radians());

    // Keep yaw from growing unbounded over a long session.
    if active.camera.yaw > PI {
        active.camera.yaw -= 2.0 * PI;
    } else if active.camera.yaw < -PI {
        active.camera.yaw += 2.0 * PI;
    }

    // Movement relative to look direction, plus world-up for vertical.
    let world_up = Vector3::new(0.0, 1.0, 0.0);
    let forward = active.camera.calc_forward_direction();
    let right = forward.cross(&world_up).normalize();

    let mut direction = Vector3::new(0.0, 0.0, 0.0);
    if input::is_action_pressed("debug_cam_forward") { direction += forward; }
    if input::is_action_pressed("debug_cam_back") { direction -= forward; }
    if input::is_action_pressed("debug_cam_right") { direction += right; }
    if input::is_action_pressed("debug_cam_left") { direction -= right; }
    if input::is_action_pressed("debug_cam_up") { direction += world_up; }
    if input::is_action_pressed("debug_cam_down") { direction -= world_up; }

    if direction.norm_squared() > 0.0 {
        let speed = if input::is_action_pressed("debug_cam_sprint") { MOVE_SPEED * SPRINT_MULTIPLIER } else { MOVE_SPEED };
        active.camera.position += direction.normalize() * speed * delta_time;
    }

    if scroll != 0.0 {
        active.projection.fovy = (active.projection.fovy - scroll * FOV_SCROLL_SPEED).clamp(FOV_MIN, FOV_MAX);
    }

    let lines = [
        format!("CAMERA DEBUG ({})", active_name),
        format!("pos: ({:.2}, {:.2}, {:.2})", active.camera.position.x, active.camera.position.y, active.camera.position.z),
        format!("yaw: {:.1}\u{b0}  pitch: {:.1}\u{b0}", active.camera.yaw.to_degrees(), active.camera.pitch.to_degrees()),
        format!("fov: {:.1}\u{b0}", active.projection.fovy),
    ];

    set_hud(app, &lines);
}

fn set_hud(app: &mut App, lines: &[String; 4]) {
    for (i, key) in HUD_LINE_KEYS.into_iter().enumerate() {
        let text = &lines[i];
        if let Some(node) = app.ui.renderizable_elements.get_mut(key) {
            if let UiNodeContent::Text(label) = &mut node.content {
                label.set_text(&mut app.ui.text.font_system, text, false);
            }
        } else {
            let node = UiNode::label(&mut app.ui.text.font_system, text, Some(HUD_WIDTH), Some(HUD_LINE_BOX_HEIGHT))
                .at(HUD_X, HUD_Y + i as f32 * HUD_LINE_SPACING)
                .set_text_color(Color::rgba(0, 255, 120, 255))
                .set_align(Align::Left)
                .set_background_color([0.0, 0.0, 0.0, 0.55]);
            app.ui.add_to_ui(key.to_owned(), node);
        }
    }

    app.ui.has_changed = true;
}

fn set_hint(app: &mut App, enabled: bool) {
    let text = if enabled { "F4: Exit Free Camera  (Shift: Sprint)" } else { "F4: Free Camera" };

    if let Some(node) = app.ui.renderizable_elements.get_mut(HINT_KEY) {
        if let UiNodeContent::Text(label) = &mut node.content {
            label.set_text(&mut app.ui.text.font_system, text, false);
        }
    } else {
        let node = UiNode::label(&mut app.ui.text.font_system, text, Some(HUD_WIDTH), Some(HUD_LINE_BOX_HEIGHT))
            .at(HUD_X, HINT_Y)
            .set_text_color(Color::rgba(255, 255, 255, 220))
            .set_align(Align::Left)
            .set_background_color([0.0, 0.0, 0.0, 0.55]);
        app.ui.add_to_ui(HINT_KEY.to_owned(), node);
    }

    app.ui.has_changed = true;
}
