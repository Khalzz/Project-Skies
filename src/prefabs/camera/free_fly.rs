//! A free-flying camera - not following anything: the mouse turns it, the
//! debug camera keys fly it where it looks (WASD, Space up, Left Ctrl down,
//! Left Shift faster), the wheel changes how fast.
//!
//! - `FreeFly`: the flying itself - where it is, which way it looks, how
//!   fast it goes - for anything that flies a view this way (F10's editor
//!   camera, see Camera::update_editor);
//! - `FreeCamera`: a node's behavior that's just that - a free camera of its
//!   own (the sandbox's). Escape frees the mouse, a click takes it back.
//!
//! It turns the engine's walking/free camera examples' way: the mouse's
//! movement (`Input::mouse_look`, the player's sensitivity in it), lightly
//! smoothed, adds to a yaw and a pitch, and the rotation is built straight
//! from those two every frame - yaw about the vertical, then pitch.

use nalgebra::{UnitQuaternion, Vector2, Vector3};

use pankarta_software::app::App;
use pankarta_software::prelude::{Input, SceneAccess};
use pankarta_software::scene_manager::behavior::Behavior;
use pankarta_software::scene_manager::node::Node;
use pankarta_software::scene_manager::node::properties::{CameraLens, Transform3D};

/// What one count of mouse movement turns it (rad), before the player's
/// sensitivity - at the game's default of 0.5, the editor camera's old
/// 0.0035 rad a count.
const RADIANS_PER_COUNT: f32 = 0.007;
/// Seconds the mouse's movement is smoothed over - hides its uneven counts
/// when it moves slowly (see the camera behavior's MOUSE_SMOOTHING_SECONDS).
const MOUSE_SMOOTHING_SECONDS: f32 = 0.015;
/// How far up or down it can look (rad) - short of straight up, where yaw
/// would stop meaning anything.
const MAX_PITCH: f32 = 1.55;
/// How fast it flies to start with, and the least and most (m/s), how much
/// faster with the sprint key, and how much one wheel notch changes it.
pub const START_SPEED: f32 = 60.0;
const MIN_SPEED: f32 = 2.0;
const MAX_SPEED: f32 = 20_000.0;
const SPRINT: f32 = 4.0;
const SPEED_STEP: f32 = 1.25;
/// How far a wheel notch moves it while not looking (see `dolly`), in
/// seconds of its speed.
const DOLLY_SECONDS: f32 = 0.6;

/// A view flying freely: where it is, which way it looks, how fast it goes.
pub struct FreeFly {
    pub position: Vector3<f32>,
    /// Round the vertical (rad, 0 looking along +Z), and up from level.
    pub yaw: f32,
    pub pitch: f32,
    /// m/s.
    pub speed: f32,
    /// The mouse's movement so far smoothed, in counts.
    smoothed_mouse: Vector2<f32>,
}

impl FreeFly {
    /// At `position`, looking along `forward`.
    pub fn new(position: Vector3<f32>, forward: Vector3<f32>) -> Self {
        let mut fly = Self { position, yaw: 0.0, pitch: 0.0, speed: START_SPEED, smoothed_mouse: Vector2::zeros() };
        fly.look_along(forward);
        fly
    }

    /// Turned to look along `forward`.
    pub fn look_along(&mut self, forward: Vector3<f32>) {
        let forward = forward.try_normalize(1e-6).unwrap_or_else(Vector3::z);
        self.yaw = forward.x.atan2(forward.z);
        self.pitch = forward.y.clamp(-1.0, 1.0).asin().clamp(-MAX_PITCH, MAX_PITCH);
    }

    /// Which way it looks.
    pub fn forward(&self) -> Vector3<f32> {
        self.rotation() * Vector3::z()
    }

    /// How it's turned: yaw about the vertical, then pitch about its own
    /// sideways axis (+Z is where it looks; a turn about X down is
    /// positive, so up is -pitch).
    pub fn rotation(&self) -> UnitQuaternion<f32> {
        UnitQuaternion::from_axis_angle(&Vector3::y_axis(), self.yaw) * UnitQuaternion::from_axis_angle(&Vector3::x_axis(), -self.pitch)
    }

    /// One frame of looking and flying, `delta_time` long: the mouse turns
    /// it, the wheel changes its speed, the debug camera keys fly it.
    pub fn steer(&mut self, input: &Input, delta_time: f32) {
        let ease = 1.0 - (-delta_time.max(0.0) / MOUSE_SMOOTHING_SECONDS).exp();
        self.smoothed_mouse += (input.mouse_look() - self.smoothed_mouse) * ease;
        let turn = self.smoothed_mouse * RADIANS_PER_COUNT;
        // Mouse right turns right (a negative turn about the vertical),
        // mouse down looks down.
        self.yaw -= turn.x;
        self.pitch = (self.pitch - turn.y).clamp(-MAX_PITCH, MAX_PITCH);

        let scroll = input.mouse_scroll_y();
        if scroll != 0.0 {
            self.speed = (self.speed * SPEED_STEP.powf(scroll)).clamp(MIN_SPEED, MAX_SPEED);
        }

        let forward = self.forward();
        let right = forward.cross(&Vector3::y()).try_normalize(1e-6).unwrap_or_else(Vector3::x);
        let mut direction = Vector3::zeros();
        for (action, way) in [
            ("debug_cam_forward", forward),
            ("debug_cam_back", -forward),
            ("debug_cam_right", right),
            ("debug_cam_left", -right),
            ("debug_cam_up", Vector3::y()),
            ("debug_cam_down", -Vector3::y()),
        ] {
            if input.is_action_pressed(action) {
                direction += way;
            }
        }
        let sprint = if input.is_action_pressed("debug_cam_sprint") { SPRINT } else { 1.0 };
        if let Some(direction) = direction.try_normalize(1e-6) {
            self.position += direction * self.speed * sprint * delta_time;
        }
    }

    /// Not looking round: the mouse's smoothing let go, so looking again
    /// starts still.
    pub fn rest(&mut self) {
        self.smoothed_mouse = Vector2::zeros();
    }

    /// Moved along where it looks by `scroll` wheel notches.
    pub fn dolly(&mut self, scroll: f32) {
        self.position += self.forward() * scroll * self.speed * DOLLY_SECONDS;
    }

    /// Puts `node` (a camera) where it is, turned as it is, seeing through
    /// `lens`.
    pub fn place(&self, node: &mut Node, lens: CameraLens) {
        self.move_node(node);
        node.set_property(lens);
    }

    /// Puts `node` where it is, turned as it is - its lens left alone.
    pub fn move_node(&self, node: &mut Node) {
        let scale = node.get_property::<Transform3D>().map(|transform| transform.scale).unwrap_or_else(|| Vector3::new(1.0, 1.0, 1.0));
        node.set_property(Transform3D { position: self.position, rotation: self.rotation(), scale });
    }
}

/// A free camera node's behavior - see the module doc. Starts where its
/// node is, looking the way it does; its lens is the node's own
/// (`CameraLens`), left as it's set.
#[derive(Default)]
pub struct FreeCamera {
    fly: Option<FreeFly>,
}

impl FreeCamera {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Behavior for FreeCamera {
    fn on_spawn(&mut self, node: &mut Node, _scene: &mut SceneAccess, app: &mut App) {
        let (position, rotation) = node
            .get_property::<Transform3D>()
            .map(|transform| (transform.position, transform.rotation))
            .unwrap_or((Vector3::zeros(), UnitQuaternion::identity()));
        self.fly = Some(FreeFly::new(position, rotation * Vector3::z()));
        app.window.set_cursor_locked(true);
    }

    fn update(&mut self, node: &mut Node, _scene: &mut SceneAccess, app: &mut App) {
        let Some(fly) = self.fly.as_mut() else { return };
        if app.input.is_just_pressed(pankarta_software::prelude::InputSource::key(pankarta_software::prelude::Key::Escape)) {
            app.window.set_cursor_locked(false);
        } else if app.input.mouse_left_button_just_pressed() {
            app.window.set_cursor_locked(true);
        }
        // The mouse is a pointer while it's free - flying and looking wait.
        if app.window.is_cursor_locked() {
            fly.steer(&app.input, app.time.unscaled_delta_time);
        } else {
            fly.rest();
        }
        fly.move_node(node);
    }
}
