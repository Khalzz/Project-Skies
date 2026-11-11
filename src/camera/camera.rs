use std::f32::consts::PI;

use nalgebra::{UnitQuaternion, Vector3};

use pankarta_software::prelude::*;

pub struct Speed {
  pub base_speed: f32,
  pub sprint_multiplier: f32
}

pub struct Fov {
  pub max: f32,
  pub min: f32,
  pub speed: f32
}

/// Everything needed to spawn a `Camera` - the node it goes on is the camera
/// (`camera_node`): a `CameraLens` and `Transform3D` at `transform`, moved by
/// this behavior.
pub struct CameraConfig {
    pub camera_name: String,
    pub transform: Transform3D,
    pub speed: Speed,
    pub fov: Fov,
    pub look_sensitivity_scale: f32,
    pub max_pitch_deg: f32,
    // If false, the camera just sits at `transform` - a fixed camera.
    pub free: bool,
    // Input action that toggles this camera between free-fly and
    // orbit-a-target mode (pivots around a point and always looks at it).
    // `None`: always free-fly, no toggle. Orbit mode does nothing until the
    // owning scene feeds it a pivot via `Camera::set_orbit_target` (call it
    // every frame while the target can move). Only meaningful when `free`.
    pub orbit_toggle_action: Option<String>,
}

pub struct Camera {
  speed: Speed,
  fov: Fov,
  look_sensitivity_scale: f32,
  max_pitch_deg: f32,
  free: bool,
  orbit_toggle_action: Option<String>,
  // Free-fly look, radians: yaw around +Y from +X toward +Z, pitch up.
  yaw: f32,
  pitch: f32,
  // Runtime orbit-mode state (see CameraConfig::orbit_toggle_action).
  orbiting: bool,
  orbit_target: Option<Vector3<f32>>,
  orbit_yaw: f32,
  orbit_pitch: f32,
  orbit_distance: f32,
}

/// The way a camera looking at `yaw`/`pitch` (radians - see `Camera::yaw`)
/// faces: forward.
fn forward_of(yaw: f32, pitch: f32) -> Vector3<f32> {
    Vector3::new(yaw.cos() * pitch.cos(), pitch.sin(), yaw.sin() * pitch.cos())
}

/// A camera node's rotation looking along `forward`, level (+Y up).
pub fn looking_along(forward: Vector3<f32>) -> UnitQuaternion<f32> {
    UnitQuaternion::face_towards(&forward, &Vector3::y())
}

impl Camera {
    /// The camera's node: its lens and transform, and this behavior.
    pub fn camera_node(config: CameraConfig) -> Node {
        let forward = config.transform.rotation * Vector3::z();
        let yaw = forward.z.atan2(forward.x);
        let pitch = forward.y.clamp(-1.0, 1.0).asin();
        let fov = config.fov.max;
        let camera = Camera {
            speed: config.speed,
            fov: config.fov,
            look_sensitivity_scale: config.look_sensitivity_scale,
            max_pitch_deg: config.max_pitch_deg,
            free: config.free,
            orbit_toggle_action: config.orbit_toggle_action,
            yaw,
            pitch,
            orbiting: false,
            orbit_target: None,
            orbit_yaw: 0.0,
            orbit_pitch: 18.0,
            orbit_distance: 45.0,
        };
        Node::new(config.camera_name)
            .with_property(config.transform)
            .with_property(CameraLens::new(fov))
            .with_behavior(camera)
    }

    /// Point the orbit-mode camera pivots around and looks at. Call every
    /// frame from the owning scene while the target can move; ignored
    /// entirely while in free-fly mode.
    pub fn set_orbit_target(&mut self, position: Vector3<f32>) {
        self.orbit_target = Some(position);
    }

    /// Whether the camera is currently in orbit-a-target mode.
    pub fn is_orbiting(&self) -> bool {
        self.orbiting
    }

    // Orbit-a-target update - mouse yaw/pitch swing the camera around
    // `target` on a boom of `orbit_distance`, scroll reels the boom in/out,
    // and the camera always looks straight at `target`.
    fn update_orbit(&mut self, transform: &mut Transform3D, target: Vector3<f32>, app: &App) {
        let sens = app.input.mouse_sensitivity();

        let scroll = app.input.mouse_scroll_y();
        if scroll != 0.0 {
            self.orbit_distance = (self.orbit_distance - scroll * 4.0).clamp(6.0, 500.0);
        }

        self.orbit_yaw -= app.input.mouse_rel_x() as f32 * sens.0 * self.look_sensitivity_scale;
        self.orbit_pitch = (self.orbit_pitch - app.input.mouse_rel_y() as f32 * sens.1 * self.look_sensitivity_scale)
            .clamp(-self.max_pitch_deg, self.max_pitch_deg);

        // Spherical offset from the target - +orbit_pitch lifts the eye up
        // and over, so it looks down at the target.
        let offset = forward_of(self.orbit_yaw.to_radians(), self.orbit_pitch.to_radians()) * self.orbit_distance;
        transform.position = target + offset;
        transform.rotation = looking_along(-offset);
    }
}

impl Behavior for Camera {
    // The cursor is only hidden for a free camera - a static one still needs
    // the cursor visible to click UI.
    fn on_spawn(&mut self, node: &mut Node, scene: &mut SceneAccess, app: &mut App) {
        scene.cameras_mut().select_camera(&node.id);
        if self.free {
            app.window.set_cursor_locked(true);
        }
    }

    // A plain, always-on fly-around camera (mouse look + WASD/Space/Left
    // Ctrl, Left Shift to sprint, scroll for fov) when free - a no-op
    // otherwise, so a static Camera just sits at its spawned pose forever.
    fn update(&mut self, node: &mut Node, _scene: &mut SceneAccess, app: &mut App) {
        if !self.free {
            return;
        }
        let dt = app.time.delta_time;

        // Free-fly <-> orbit-a-target toggle (see CameraConfig::orbit_toggle_action).
        if let Some(action) = &self.orbit_toggle_action {
            if app.input.is_action_just_pressed(action) {
                self.orbiting = !self.orbiting;
            }
        }
        if self.orbiting {
            if let (Some(target), Some(mut transform)) = (self.orbit_target, node.transform_3d().copied()) {
                self.update_orbit(&mut transform, target, app);
                node.set_property(transform);
            }
            return;
        }

        let sens = app.input.mouse_sensitivity();
        let scroll = app.input.mouse_scroll_y();

        self.yaw += (app.input.mouse_rel_x() as f32 * sens.0 * self.look_sensitivity_scale).to_radians();
        self.pitch = (self.pitch - (app.input.mouse_rel_y() as f32 * sens.1 * self.look_sensitivity_scale).to_radians())
            .clamp(-self.max_pitch_deg.to_radians(), self.max_pitch_deg.to_radians());
        // Keep yaw from growing unbounded over a long session.
        if self.yaw > PI {
            self.yaw -= 2.0 * PI;
        } else if self.yaw < -PI {
            self.yaw += 2.0 * PI;
        }

        let world_up = Vector3::new(0.0, 1.0, 0.0);
        let forward = forward_of(self.yaw, self.pitch);
        let right = forward.cross(&world_up).normalize();

        let mut direction = Vector3::new(0.0, 0.0, 0.0);
        if app.input.is_action_pressed("debug_cam_forward") { direction += forward; }
        if app.input.is_action_pressed("debug_cam_back") { direction -= forward; }
        if app.input.is_action_pressed("debug_cam_right") { direction += right; }
        if app.input.is_action_pressed("debug_cam_left") { direction -= right; }
        if app.input.is_action_pressed("debug_cam_up") { direction += world_up; }
        if app.input.is_action_pressed("debug_cam_down") { direction -= world_up; }
        let speed = if app.input.is_action_pressed("debug_cam_sprint") { self.speed.base_speed * self.speed.sprint_multiplier } else { self.speed.base_speed };

        if let Some(transform) = node.transform_3d_mut() {
            transform.rotation = looking_along(forward);
            if direction.norm_squared() > 0.0 {
                transform.position += direction.normalize() * speed * dt;
            }
        }

        if scroll != 0.0 {
            let (min, max, step) = (self.fov.min, self.fov.max, self.fov.speed);
            if let Some(lens) = node.get_property_mut::<CameraLens>() {
                lens.fovy = (lens.fovy - scroll * step).clamp(min, max);
            }
        }
    }
}
