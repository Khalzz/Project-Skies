use std::f32::consts::PI;

use nalgebra::Vector3;

use crate::app::App;
use crate::engine::input::input;
use crate::engine::rendering::camera::handler::SceneCameras;
use crate::engine::scene_manager::behavior::Behavior;
use crate::engine::scene_manager::node::Node;
use crate::transform::Transform;

pub struct Speed {
  pub base_speed: f32,
  pub sprint_multiplier: f32
}

pub struct Fov {
  pub max: f32,
  pub min: f32,
  pub speed: f32
}

/// Everything needed to spawn a `Camera` behavior, gathered into one struct
/// instead of a long positional parameter list - see the design discussion
/// this came out of. `transform` uses the same type `data.ron`-sourced
/// objects do (`crate::transform::Transform`) and is handed straight through
/// to `CameraHandler::create_camera`, which does the actual rotation-to-yaw/
/// pitch decomposition - this behavior doesn't duplicate that math itself,
/// it's just a thin spawner around the engine's own base camera API.
pub struct CameraConfig {
    pub camera_name: String,
    pub transform: Transform,
    pub speed: Speed,
    pub fov: Fov,
    pub look_sensitivity_scale: f32,
    pub max_pitch_deg: f32,
    // If false, on_spawn still creates+selects the camera at transform's
    // pose, but update() never touches it again afterward - a fixed camera.
    // Not the same thing as main_menu's own scripted drift_camera or play's
    // flight-follow-cam, which are their own bespoke per-scene logic on top
    // of a camera, not something this flag turns on.
    pub free: bool,
}

pub struct Camera {
  camera_name: String,
  origin_transform: Transform,
  speed: Speed,
  fov: Fov,
  look_sensitivity_scale: f32,
  max_pitch_deg: f32,
  free: bool,
}

impl Camera {
    pub fn new(config: CameraConfig) -> Self {
        Camera {
            camera_name: config.camera_name,
            origin_transform: config.transform,
            speed: config.speed,
            fov: config.fov,
            look_sensitivity_scale: config.look_sensitivity_scale,
            max_pitch_deg: config.max_pitch_deg,
            free: config.free,
        }
    }
}

impl Behavior for Camera {
    // Same setup sandbox::scene::GameLogic::new used to do directly - a
    // camera is created and selected explicitly (same convention data.ron's
    // named cameras used to use) from origin_transform directly, rather than
    // mutating whatever "active" already happened to be. The cursor is only
    // hidden for a free camera - a static one (e.g. main_menu's) still needs
    // the cursor visible to click UI.
    fn on_spawn(&mut self, _node: &mut Node, cameras: &mut SceneCameras, app: &mut App) {
        cameras.create_camera(&self.camera_name, self.origin_transform, self.fov.max);
        cameras.select_camera(&self.camera_name);
        if self.free {
            app.window_manager.context.mouse().show_cursor(false);
        }
    }

    // A plain, always-on fly-around camera (mouse look + WASD/Space/Left
    // Ctrl, Left Shift to sprint, scroll for fov) when free - a no-op
    // otherwise, so a static Camera just sits at its spawned pose forever.
    fn update(&mut self, _node: &mut Node, cameras: &mut SceneCameras, _app: &mut App, dt: f32) {
        if !self.free {
            return;
        }

        let sens = input::mouse_sensitivity();
        let scroll = input::mouse_scroll_y();
        let active = cameras.active_mut();

        let mut yaw = active.camera.yaw() + (input::mouse_rel_x() as f32 * sens.0 * self.look_sensitivity_scale).to_radians();
        let pitch = (active.camera.pitch() - (input::mouse_rel_y() as f32 * sens.1 * self.look_sensitivity_scale).to_radians())
            .clamp(-self.max_pitch_deg.to_radians(), self.max_pitch_deg.to_radians());

        // Keep yaw from growing unbounded over a long session.
        if yaw > PI {
            yaw -= 2.0 * PI;
        } else if yaw < -PI {
            yaw += 2.0 * PI;
        }
        active.camera.set_yaw_pitch(yaw, pitch);

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
            let speed = if input::is_action_pressed("debug_cam_sprint") { self.speed.base_speed * self.speed.sprint_multiplier } else { self.speed.base_speed };
            active.camera.set_position(active.camera.position() + direction.normalize() * speed * dt);
        }

        if scroll != 0.0 {
            active.projection.fovy = (active.projection.fovy - scroll * self.fov.speed).clamp(self.fov.min, self.fov.max);
        }
    }

    fn fixed_update(&mut self, _node: &mut Node, _cameras: &mut SceneCameras, _app: &mut App, _dt: f32) {

    }
}
