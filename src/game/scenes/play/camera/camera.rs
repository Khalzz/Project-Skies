use nalgebra::{UnitQuaternion, Vector3};

use crate::app::App;
use crate::engine::game_nodes::game_object::Cameras;
use crate::engine::input::input;
use crate::engine::rendering::camera::handler::SceneCameras;
use crate::engine::scene_manager::behavior::Behavior;
use crate::engine::scene_manager::node::Node;
use crate::engine::scene_manager::properties::{CameraLens, Transform3D};
use crate::engine::scene_manager::scene_nodes::facing;
use crate::engine::utils::lerps::{lerp, lerp_quaternion, smoothstep};
use crate::game::game_settings::GAME_SETTINGS;

use super::head_motion::{HeadInput, HeadMotion};

pub const NORMAL_CAMERA_FOV: f32 = 45.0;

/// Looking behind in the cockpit: the pilot is strapped into the seat, so
/// to see behind they twist and look over their own shoulder - the eyes
/// move off the seat's center toward the side they're looking, up to clear
/// the seat/shoulder, and forward with the twist. How far along that they
/// are follows how far round the view is turned: 0 facing forward, 1 facing
/// straight behind (either side).
pub struct LookBehindTuning {
    /// At fully behind, meters: toward the side being looked at, up, and
    /// forward.
    pub side: f32,
    pub up: f32,
    pub forward: f32,
    /// How the offset builds from 0 (forward) to 1 (behind) - 1.0 = evenly,
    /// higher = barely moves looking to the side, most of it on the last
    /// stretch toward behind.
    pub curve: f32,
}

pub const COCKPIT_LOOK_BEHIND: LookBehindTuning = LookBehindTuning {
    side: 0.15,
    up: 0.05,
    forward: 0.08,
    curve: 2.0,
};

/// How fast the cockpit view eases to a new resting angle (Settings'
/// "Cockpit view angle"), per second - so changing it slides the view there
/// instead of snapping.
const COCKPIT_VIEW_PITCH_EASE_RATE: f32 = 6.0;

// Right-stick camera look (see `LookInput`). Matches the "look_*" actions'
// own deadzone in settings/input.ron - the stick reads 0 below it, and
// `LookInput::read` rescales what's past it back to 0..1 so the camera
// starts moving from zero instead of snapping to the deadzone's angle.
const LOOK_STICK_DEADZONE: f32 = 0.1;
/// Fixed modes (ThirdPerson/Front): full sideways stick orbits this
/// far around the plane - 180 = all the way round to its nose, looking back.
const LOOK_MAX_YAW_DEG: f32 = 180.0;
/// Fixed modes: full up/down stick orbits to straight below/above the plane
/// (stops just short of 90 so the view never lines up with its own up axis).
const LOOK_MAX_PITCH_DEG: f32 = 89.0;
/// How fast the orbit follows the stick (per second) - quick, but eases
/// instead of snapping, and eases back behind the plane on release.
const LOOK_FOLLOW_RATE: f32 = 10.0;
/// Fixed modes' look dead zone on the raw stick, per axis (0.1 = 10% of its
/// travel): inside it on both axes the camera is STATIC - the mode's own
/// predefined position. Past it on either axis it's MOVING - orbiting the
/// plane with the stick, the swing ramping up from zero at the dead zone's
/// edge to full at full stick.
const LOOK_DEADZONE: f32 = 0.1;
/// While MOVING, the camera orbits this many world units farther from the
/// plane than its static position, level with the plane's center and
/// looking straight at it (so the plane sits in the middle of the screen).
const LOOK_MOVING_EXTRA_DISTANCE: f32 = 19.1;
/// How fast the camera eases out from the static distance/height to the
/// moving one (per second) - see `orbit_look`.
const LOOK_STATE_RATE: f32 = 8.0;
/// How fast the camera eases back once the stick is released (per second) -
/// the distance/height AND the angle, together, so it's back behind the
/// plane at its static pose in a fraction of a second. Higher = snappier.
const LOOK_RETURN_RATE: f32 = 20.0;
/// Free mode: turn rate at full stick, deg/s - slower than a mouse flick.
const FREE_LOOK_STICK_RATE_DEG_S: f32 = 130.0;
/// Free mode: how far from the plane it orbits.
const FREE_DISTANCE: f32 = 21.5;
/// After a crash (see `enter_wreck_view`): the Free orbit this far out, to
/// see the wreck and the water around it, eased out to at FREE_DISTANCE_RATE.
const WRECK_DISTANCE: f32 = 43.0;
const FREE_DISTANCE_RATE: f32 = 2.0;
/// After a crash: the camera stays at least this high above the water, and
/// looks at the wreck no lower than the surface - so once it sinks the view
/// holds on the spot where it went down instead of following it under.
const WRECK_MIN_HEIGHT: f32 = 3.0;

/// F10's tactical map view (see `Camera::update_tactical`): how far out it
/// starts and how far it zooms (m), how much one wheel notch zooms, the angle
/// it starts looking down at, its field of view, and how much it turns per
/// pixel of mouse (deg).
const TACTICAL_START_DISTANCE: f32 = 6000.0;
const TACTICAL_MIN_DISTANCE: f32 = 200.0;
const TACTICAL_MAX_DISTANCE: f32 = 400_000.0;
const TACTICAL_ZOOM_STEP: f32 = 1.15;
const TACTICAL_START_PITCH_DEG: f32 = 35.0;
const TACTICAL_FOV: f32 = 50.0;
const TACTICAL_TURN_PER_PIXEL_DEG: f32 = 0.25;

/// F10's editor camera (see `Camera::update_editor`): its field of view,
/// how far mouse look turns it per pixel (rad), how fast it flies to start
/// with and the most and least (m/s), how much faster with the sprint key,
/// how much one wheel notch changes its speed, and how far a wheel notch
/// moves it while not looking (in seconds of its speed).
const EDITOR_FOV: f32 = 60.0;
const EDITOR_TURN_PER_PIXEL: f32 = 0.0035;
const EDITOR_START_SPEED: f32 = 60.0;
const EDITOR_MIN_SPEED: f32 = 2.0;
const EDITOR_MAX_SPEED: f32 = 20_000.0;
const EDITOR_SPRINT: f32 = 4.0;
const EDITOR_SPEED_STEP: f32 = 1.25;
const EDITOR_DOLLY_SECONDS: f32 = 0.6;
/// How fast a locked view eases back to center (per second).
const LOOK_CENTER_RATE: f32 = 6.0;

/// F10's editor camera - free, like a game engine editor's: where it is,
/// which way it looks, how fast it flies.
struct EditorCamera {
    position: Vector3<f32>,
    /// Round the vertical (rad, 0 looking along +Z) and up from level.
    yaw: f32,
    pitch: f32,
    speed: f32,
    /// The right button went down off the editor's panels - looking and
    /// flying until it's let go, even over a panel.
    looking: bool,
}

impl EditorCamera {
    fn forward(&self) -> Vector3<f32> {
        Vector3::new(self.pitch.cos() * self.yaw.sin(), self.pitch.sin(), self.pitch.cos() * self.yaw.cos())
    }
}

/// F10's tactical map view - an orbit round a point on the map: the
/// target, while following it, or wherever it's been dragged to.
struct TacticalOrbit {
    /// What it orbits - `None` until the first frame (then the target).
    pivot: Option<Vector3<f32>>,
    /// Kept on the target (the map's center button) - otherwise it stays
    /// where it is, moved only by dragging.
    follow: bool,
    /// Round the target (deg), and above its level (deg).
    yaw: f32,
    pitch: f32,
    /// From the target (m).
    distance: f32,
}

/// The right stick, as look input - both axes -1..1, `x` positive = look
/// right, `y` positive = look up. `x`/`y` have the actions' small deadzone
/// removed and rescaled (Cockpit/Free); `raw_x`/`raw_y` are the stick as-is,
/// for the fixed modes' own, bigger LOOK_DEADZONE.
struct LookInput {
    x: f32,
    y: f32,
    raw_x: f32,
    raw_y: f32,
}

impl LookInput {
    /// The stick left alone.
    fn none() -> Self {
        Self { x: 0.0, y: 0.0, raw_x: 0.0, raw_y: 0.0 }
    }

    fn read() -> Self {
        let raw_x = input::get_axis("look_left", "look_right");
        let raw_y = input::get_axis("look_down", "look_up");
        Self { x: Self::rescale(raw_x), y: Self::rescale(raw_y), raw_x, raw_y }
    }

    fn rescale(value: f32) -> f32 {
        let magnitude = ((value.abs() - LOOK_STICK_DEADZONE) / (1.0 - LOOK_STICK_DEADZONE)).clamp(0.0, 1.0);
        magnitude * value.signum()
    }
}

/// This frame's mouse look and wheel - all zero while the mouse is freed
/// for the debug overlay (F7).
struct MouseLook {
    rel_x: f32,
    rel_y: f32,
    scroll: f32,
}

impl MouseLook {
    fn read(freed: bool) -> Self {
        if freed {
            return Self { rel_x: 0.0, rel_y: 0.0, scroll: 0.0 };
        }
        Self { rel_x: input::mouse_rel_x() as f32, rel_y: input::mouse_rel_y() as f32, scroll: input::mouse_scroll_y() }
    }
}

/// The views of an aircraft, in the order the camera key steps through
/// them - the cockpit first.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CameraState {
    Cockpit,
    ThirdPerson,
    Free,
    Front,
}

/// Each view's place on the aircraft - its name among the aircraft's
/// cameras (`data.ron`'s `cameras`, see play::plane::aircraft_spec), and
/// where it is when the aircraft doesn't say (m, in its frame; fov).
pub const COCKPIT_CAMERA: &str = "cockpit";
pub const THIRD_PERSON_CAMERA: &str = "third_person";
pub const FREE_CAMERA: &str = "free";
pub const FRONT_CAMERA: &str = "front";
pub const DEFAULT_THIRD_PERSON: (Vector3<f32>, f32) = (Vector3::new(0.0, 3.85, -23.77), NORMAL_CAMERA_FOV);
/// The free camera's: what it orbits round.
pub const DEFAULT_FREE: (Vector3<f32>, f32) = (Vector3::new(0.0, 0.0, 0.0), 60.0);
pub const DEFAULT_FRONT: (Vector3<f32>, f32) = (Vector3::new(0.0, 2.895, 16.84), 40.0);

pub struct CameraTarget {
    pub position: Vector3<f32>,
    pub rotation: UnitQuaternion<f32>,
    pub cameras: Option<Cameras>,
}

pub struct Camera {
    state: CameraState,
    target: Option<CameraTarget>,
    pub look_at: Option<Vector3<f32>>,
    pub next_look_at: Option<Vector3<f32>>,
    pub mod_quaternion: UnitQuaternion<f32>,
    pub debug_offset: Vector3<f32>,
    pub debug_mode_active: bool,
    // The current camera state's own plane-relative offset (what the camera
    // would sit at with debug_mode_active off), refreshed every frame. Purely
    // for the F7 Camera Editor to display alongside `debug_offset`.
    pub debug_base_offset: Vector3<f32>,
    pub cockpit_current_rotation: UnitQuaternion<f32>,
    pub cockpit_target_fov: f32,
    pub cockpit_current_fov: f32,
    pub cockpit_yaw: f32,
    pub cockpit_pitch: f32,
    // The pilot's neck - see `head_motion`. Fed by `set_head_input`.
    head_motion: HeadMotion,
    head_input: HeadInput,
    // The cockpit view's resting angle as currently shown, easing toward
    // the setting - see COCKPIT_VIEW_PITCH_EASE_RATE.
    cockpit_view_pitch: f32,
    pub free_yaw: f32,
    pub free_pitch: f32,
    pub free_current_rotation: UnitQuaternion<f32>,
    pub free_target_fov: f32,
    pub free_current_fov: f32,
    // Fixed modes' right-stick orbit around the plane, degrees, eased toward
    // the stick's absolute position - see `orbit_look`.
    look_yaw: f32,
    look_pitch: f32,
    // 0 = static pose, 1 = moving pose (eased between) - see `orbit_look`.
    look_moving: f32,
    // Free mode's current orbit distance, eased toward FREE_DISTANCE (or
    // WRECK_DISTANCE after a crash).
    free_distance: f32,
    // The water's height, once the plane has crashed - see `enter_wreck_view`.
    wreck_sea_level: Option<f32>,
    // M's map view, while it's up - see `update_tactical`.
    tactical: Option<TacticalOrbit>,
    // F10's editor camera, while it's up - see `update_editor`.
    editor: Option<EditorCamera>,
    /// The player's controls are locked (the intro, an animation) - no
    /// looking around or switching views meanwhile: the view eases back to
    /// its center.
    pub look_locked: bool,
}

impl Camera {
    pub fn new() -> Self {
        Self {
            state: CameraState::Cockpit,
            target: None,
            look_at: None,
            next_look_at: None,
            mod_quaternion: UnitQuaternion::identity(),
            // A delta from the active camera state's base offset (see
            // `update`), not an absolute offset - starts at zero so enabling
            // the editor doesn't jump the camera.
            debug_offset: Vector3::zeros(),
            debug_mode_active: false,
            debug_base_offset: Vector3::zeros(),
            cockpit_current_rotation: UnitQuaternion::identity(),
            cockpit_target_fov: 70.0,
            cockpit_current_fov: 70.0,
            cockpit_yaw: 0.0,
            cockpit_pitch: 0.0,
            head_motion: HeadMotion::new(),
            head_input: HeadInput::default(),
            cockpit_view_pitch: GAME_SETTINGS.lock().unwrap().cockpit_view_pitch_deg,
            free_yaw: 0.0,
            free_pitch: 0.0,
            free_current_rotation: UnitQuaternion::identity(),
            free_target_fov: 60.0,
            free_current_fov: 60.0,
            look_yaw: 0.0,
            look_pitch: 0.0,
            look_moving: 0.0,
            free_distance: FREE_DISTANCE,
            wreck_sea_level: None,
            tactical: None,
            editor: None,
            look_locked: false,
        }
    }

    /// Into or out of F10's editor camera (see `update_editor`) - it starts
    /// at `from` (position, and the way it looks), where the view was.
    pub fn set_editor_view(&mut self, on: bool, from: (Vector3<f32>, Vector3<f32>)) {
        if !on {
            self.editor = None;
            return;
        }
        if self.editor.is_some() {
            return;
        }
        let (position, forward) = from;
        let forward = forward.try_normalize(1e-6).unwrap_or_else(Vector3::z);
        self.editor = Some(EditorCamera {
            position,
            yaw: forward.x.atan2(forward.z),
            pitch: forward.y.clamp(-1.0, 1.0).asin(),
            speed: EDITOR_START_SPEED,
            looking: false,
        });
    }

    /// The editor camera looking at `target` from `distance` away, the way
    /// it already faces.
    pub fn editor_focus(&mut self, target: Vector3<f32>, distance: f32) {
        if let Some(editor) = &mut self.editor {
            editor.position = target - editor.forward() * distance;
        }
    }

    /// F10's editor camera: held right button (pressed off the panels) to
    /// look round with the mouse and fly with the debug camera keys (WASD,
    /// space/ctrl up and down, shift faster), the wheel changing its speed
    /// meanwhile; otherwise the wheel moves it forward and back (not over a
    /// panel) and the mouse is the editor's.
    ///
    /// While the animation camera is on screen the view stays there: the
    /// editor camera is parked on it, and the mouse and WASD don't drift it.
    /// Holding the right button takes that view over and flies from it at once.
    fn update_editor(&mut self, node: &mut Node, cameras: &SceneCameras, app: &mut App, delta_time: f32) {
        let cinematic = cameras.active_name() == "animation_camera";
        if cinematic {
            if let Some(instance) = cameras.get("animation_camera") {
                let forward = instance.camera.calc_forward_direction().try_normalize(1e-6).unwrap_or_else(Vector3::z);
                if let Some(editor) = self.editor.as_mut() {
                    editor.position = instance.camera.position().coords;
                    editor.yaw = forward.x.atan2(forward.z);
                    editor.pitch = forward.y.clamp(-1.0, 1.0).asin();
                }
            }
        }
        let Some(editor) = self.editor.as_mut() else { return };
        let over_ui = app.egui_overlay.ctx.is_pointer_over_area();
        let right = input::mouse_right_button_down();
        let take_over = cinematic && right && !over_ui && !app.is_paused;
        if take_over {
            app.flight_editor.timeline_preview = false;
        }
        if cinematic && !take_over {
            if editor.looking && !app.is_paused {
                app.window_manager.context.mouse().set_relative_mouse_mode(false);
            }
            editor.looking = false;
            let (position, forward) = (editor.position, editor.forward());
            set_pose(node, position, position + forward, Vector3::y(), CameraLens { fovy: EDITOR_FOV, znear: 0.5, viewport: None });
            return;
        }
        let was_looking = editor.looking;
        if !right {
            editor.looking = false;
        } else if !editor.looking && !over_ui {
            editor.looking = true;
        }
        // The cursor held while looking, so it never stops at the screen's
        // edge - let go with the button.
        if editor.looking != was_looking && !app.is_paused {
            app.window_manager.context.mouse().set_relative_mouse_mode(editor.looking);
        }
        let scroll = input::mouse_scroll_y();
        if editor.looking && !app.is_paused {
            editor.yaw -= input::mouse_rel_x() as f32 * EDITOR_TURN_PER_PIXEL;
            editor.pitch = (editor.pitch - input::mouse_rel_y() as f32 * EDITOR_TURN_PER_PIXEL).clamp(-1.55, 1.55);
            if scroll != 0.0 {
                editor.speed = (editor.speed * EDITOR_SPEED_STEP.powf(scroll)).clamp(EDITOR_MIN_SPEED, EDITOR_MAX_SPEED);
            }
            let forward = editor.forward();
            let right_dir = forward.cross(&Vector3::y()).try_normalize(1e-6).unwrap_or_else(Vector3::x);
            let mut direction = Vector3::zeros();
            if input::is_action_pressed("debug_cam_forward") { direction += forward; }
            if input::is_action_pressed("debug_cam_back") { direction -= forward; }
            if input::is_action_pressed("debug_cam_right") { direction += right_dir; }
            if input::is_action_pressed("debug_cam_left") { direction -= right_dir; }
            if input::is_action_pressed("debug_cam_up") { direction += Vector3::y(); }
            if input::is_action_pressed("debug_cam_down") { direction -= Vector3::y(); }
            let sprint = if input::is_action_pressed("debug_cam_sprint") { EDITOR_SPRINT } else { 1.0 };
            if let Some(direction) = direction.try_normalize(1e-6) {
                editor.position += direction * editor.speed * sprint * delta_time;
            }
        } else if scroll != 0.0 && !over_ui && !app.is_paused {
            editor.position += editor.forward() * scroll * editor.speed * EDITOR_DOLLY_SECONDS;
        }
        let (position, forward) = (editor.position, editor.forward());
        set_pose(node, position, position + forward, Vector3::y(), CameraLens { fovy: EDITOR_FOV, znear: 0.5, viewport: None });
    }

    /// Moves the editor camera by `delta` (m) - keeping up with what it
    /// follows.
    pub fn editor_shift(&mut self, delta: Vector3<f32>) {
        if let Some(editor) = &mut self.editor {
            editor.position += delta;
        }
    }

    /// The editor camera's flying with the mouse (its right button held) -
    /// the player's controls are kept off meanwhile.
    pub fn editor_looking(&self) -> bool {
        self.editor.as_ref().is_some_and(|editor| editor.looking)
    }

    /// Looking out of the cockpit.
    pub fn is_cockpit(&self) -> bool {
        matches!(self.state, CameraState::Cockpit)
    }

    pub fn state_name(&self) -> &'static str {
        match self.state {
            CameraState::Cockpit => "Cockpit",
            CameraState::ThirdPerson => "Third person",
            CameraState::Free => "Free",
            CameraState::Front => "Front",
        }
    }

    /// Switches to the view `state` - not after a crash, where the camera
    /// stays on the wreck (see `enter_wreck_view`).
    pub fn set_state(&mut self, state: CameraState) {
        if self.wreck_sea_level.is_none() {
            self.state = state;
        }
    }

    /// On to the next view (see `CameraState`), round to the cockpit again -
    /// not after a crash.
    pub fn next_camera(&mut self) {
        let next = match self.state {
            CameraState::Cockpit => CameraState::ThirdPerson,
            CameraState::ThirdPerson => CameraState::Free,
            CameraState::Free => CameraState::Front,
            CameraState::Front => CameraState::Cockpit,
        };
        self.set_state(next);
    }

    /// The aircraft's camera `name` - where it is on it (m, its frame) and
    /// its field of view - or `default` when it doesn't have one.
    fn named(target: &CameraTarget, name: &str, default: (Vector3<f32>, f32)) -> (Vector3<f32>, f32) {
        target.cameras.as_ref()
            .and_then(|cameras| cameras.get(name))
            .map(|camera| (camera.position, camera.fov))
            .unwrap_or(default)
    }

    pub fn set_target(&mut self, position: Vector3<f32>, rotation: UnitQuaternion<f32>, cameras: Option<Cameras>) {
        self.target = Some(CameraTarget { position, rotation, cameras });
    }

    /// What moves the cockpit view's head this frame - see `HeadInput`.
    pub fn set_head_input(&mut self, input: HeadInput) {
        self.head_input = input;
    }

    /// The plane crashed into the water at height `sea_level`: switches to
    /// Free (orbit the wreck freely with the mouse / right stick), backs out
    /// to WRECK_DISTANCE, keeps the view level with the world instead of
    /// the tumbling plane, and stays there - camera switching is off.
    pub fn enter_wreck_view(&mut self, sea_level: f32) {
        if self.wreck_sea_level.is_some() {
            return;
        }
        self.wreck_sea_level = Some(sea_level);
        self.state = CameraState::Free;
        // Start from behind the plane's heading, a little above it.
        if let Some(target) = &self.target {
            let forward = target.rotation * Vector3::z();
            self.free_yaw = forward.x.atan2(forward.z).to_degrees();
        }
        self.free_pitch = 20.0;
    }

    /// Back from `enter_wreck_view` - the player has a flying plane again
    /// (respawned, or switched to another one from the flight editor (F10's editor view)).
    /// Into or out of F10's tactical map view (see `update_tactical`) - it
    /// starts behind the target, looking down on it.
    pub fn set_tactical_view(&mut self, on: bool) {
        if !on {
            self.tactical = None;
            return;
        }
        if self.tactical.is_some() {
            return;
        }
        let yaw = self.target.as_ref().map(|target| {
            let forward = target.rotation * Vector3::z();
            // Behind it: the orbit's offset points the other way.
            (-forward.z).atan2(-forward.x).to_degrees()
        }).unwrap_or(0.0);
        self.tactical = Some(TacticalOrbit { pivot: None, follow: false, yaw, pitch: TACTICAL_START_PITCH_DEG, distance: TACTICAL_START_DISTANCE });
    }

    /// The tactical view kept on the target (the map's center button) - or
    /// left where it is.
    pub fn set_tactical_follow(&mut self, follow: bool) {
        if let Some(orbit) = &mut self.tactical {
            orbit.follow = follow;
        }
    }

    /// Drags the tactical view across the map by `delta` screen pixels (x
    /// right, y down) on a screen `screen_height` pixels tall - the map moves
    /// with the pointer, like dragging a sheet of paper. Stops following.
    pub fn pan_tactical(&mut self, delta: (f32, f32), screen_height: f32) {
        let Some(orbit) = &mut self.tactical else { return };
        let Some(pivot) = &mut orbit.pivot else { return };
        orbit.follow = false;
        // How far a pixel is at the pivot.
        let meters_per_pixel = 2.0 * orbit.distance * (TACTICAL_FOV.to_radians() * 0.5).tan() / screen_height.max(1.0);
        let yaw = orbit.yaw.to_radians();
        // Flat on the map: away from the camera, and to its right.
        let forward = -Vector3::new(yaw.cos(), 0.0, yaw.sin());
        let right = forward.cross(&Vector3::y());
        // Looking along the ground, a pixel up the screen covers more of it.
        let foreshortening = orbit.pitch.to_radians().sin().abs().max(0.3);
        *pivot -= right * delta.0 * meters_per_pixel;
        *pivot += forward * delta.1 * meters_per_pixel / foreshortening;
    }

    /// F10's tactical map view: orbits its pivot - the target while
    /// following it, otherwise wherever it was dragged to (see
    /// `pan_tactical`). The right mouse button turns it round, the wheel
    /// zooms from TACTICAL_MIN_DISTANCE out to TACTICAL_MAX_DISTANCE (not
    /// over a window, which they're for).
    fn update_tactical(&mut self, node: &mut Node, app: &mut App) {
        let Some(orbit) = self.tactical.as_mut() else { return };
        let Some(target) = self.target.take() else { return };

        let mouse_free = app.mouse_free();
        // Its own bar and flight plan window are egui too.
        let overlay_up = app.debug_overlay_visible() || app.tactical_map_active;
        let over_ui = overlay_up && app.egui_overlay.ctx.is_pointer_over_area();
        let turning = !app.is_paused && (!mouse_free || (input::mouse_right_button_down() && !over_ui));
        if turning {
            orbit.yaw += input::mouse_rel_x() as f32 * TACTICAL_TURN_PER_PIXEL_DEG;
            orbit.pitch = (orbit.pitch + input::mouse_rel_y() as f32 * TACTICAL_TURN_PER_PIXEL_DEG).clamp(-85.0, 89.0);
        }
        let scroll = input::mouse_scroll_y();
        if scroll != 0.0 && !app.is_paused && !over_ui {
            orbit.distance = (orbit.distance * TACTICAL_ZOOM_STEP.powf(-scroll)).clamp(TACTICAL_MIN_DISTANCE, TACTICAL_MAX_DISTANCE);
        }

        let pivot = if orbit.follow {
            *orbit.pivot.insert(target.position)
        } else {
            *orbit.pivot.get_or_insert(target.position)
        };

        let (yaw, pitch) = (orbit.yaw.to_radians(), orbit.pitch.to_radians());
        let offset = Vector3::new(yaw.cos() * pitch.cos(), pitch.sin(), yaw.sin() * pitch.cos()) * orbit.distance;
        // Shaped like where it's drawn - the editor view's middle, or the
        // whole screen.
        let (width, height) = app.tactical_viewport
            .map(|[_, _, width, height]| (width, height))
            .unwrap_or((app.window_manager.size.width as f32, app.window_manager.size.height as f32));
        let lens = CameraLens { fovy: TACTICAL_FOV, znear: 1.0, viewport: Some((width.max(1.0) as u32, height.max(1.0) as u32)) };
        set_pose(node, pivot + offset, pivot, Vector3::y(), lens);
        self.target = Some(target);
    }

    pub fn leave_wreck_view(&mut self) {
        if self.wreck_sea_level.take().is_none() {
            return;
        }
        self.state = CameraState::Cockpit;
        self.free_distance = FREE_DISTANCE;
    }

    /// Fixed modes' right-stick look, Ace Combat-style, with two states:
    /// - STATIC (stick inside LOOK_DEADZONE on both axes): the mode's own
    ///   predefined pose, `position`/`look_at`/`up`, untouched;
    /// - MOVING (past it on either axis): orbiting the plane, the stick's
    ///   position IS the angle - full sideways swings round to the plane's
    ///   nose (looking back), half is a side view, full up drops below the
    ///   plane to look up at it, full down lifts above to look down - at
    ///   LOOK_MOVING_EXTRA_DISTANCE farther than the static pose, level with
    ///   the plane's center and looking straight at it.
    /// The camera eases out to the moving distance/height (LOOK_STATE_RATE),
    /// and once the stick returns inside the dead zone it eases back to the
    /// static pose's own distance and height (LOOK_RETURN_RATE) while the
    /// angle swings back round behind the plane (LOOK_FOLLOW_RATE) - both at
    /// once, ending exactly on the static pose. All in the plane's own frame,
    /// so it follows the plane through any attitude.
    fn orbit_look(&mut self, look: &LookInput, target: &CameraTarget, position: Vector3<f32>, look_at: Vector3<f32>, up: Vector3<f32>, delta_time: f32) -> (Vector3<f32>, Vector3<f32>, Vector3<f32>) {
        let look_x = Self::past_look_deadzone(look.raw_x);
        let look_y = Self::past_look_deadzone(look.raw_y);

        // The orbit angle eases toward the stick (back to straight behind
        // once it's released). Negative yaw swings the view right, negative
        // pitch drops the camera below the plane (stick up looks up at it) -
        // same conventions as the mouse look in Cockpit/Free.
        let released = look_x == 0.0 && look_y == 0.0;
        let angle_rate = if released { LOOK_RETURN_RATE } else { LOOK_FOLLOW_RATE };
        let t = (delta_time * angle_rate).clamp(0.0, 1.0);
        self.look_yaw = lerp(self.look_yaw, -look_x * LOOK_MAX_YAW_DEG, t);
        self.look_pitch = lerp(self.look_pitch, -look_y * LOOK_MAX_PITCH_DEG, t);

        // Eases out to the moving distance/height while the stick is past the
        // dead zone, and back to the static one (quicker) once released.
        if !released {
            self.look_moving = lerp(self.look_moving, 1.0, (delta_time * LOOK_STATE_RATE).clamp(0.0, 1.0));
        } else {
            self.look_moving = lerp(self.look_moving, 0.0, (delta_time * LOOK_RETURN_RATE).clamp(0.0, 1.0));
            if self.look_moving < 0.001 {
                self.look_moving = 0.0;
            }
        }

        // The orbit, in the plane's own frame, around the static pose's
        // direction from the plane with its height dropped (`behind`) - pitch
        // turns about the horizontal axis across it, so stick up/down means
        // the same thing whichever side the static pose is on.
        let static_offset = target.rotation.inverse() * (position - target.position);
        let behind = Vector3::new(static_offset.x, 0.0, static_offset.z).try_normalize(1e-4).unwrap_or(-Vector3::z());
        let pitch = UnitQuaternion::from_axis_angle(&nalgebra::Unit::new_normalize(behind.cross(&Vector3::y())), self.look_pitch.to_radians());
        let orbit = UnitQuaternion::from_axis_angle(&Vector3::y_axis(), self.look_yaw.to_radians()) * pitch;
        let orbit_world = target.rotation * orbit * target.rotation.inverse();

        // The static pose swung round by the orbit - its own distance and
        // height, just seen from the current angle.
        let swung_position = target.position + orbit_world * (position - target.position);
        let swung_look_at = target.position + orbit_world * (look_at - target.position);
        let swung_up = orbit_world * up;
        if self.look_moving <= 0.0 {
            return (swung_position, swung_look_at, swung_up);
        }

        // The moving pose: level with the plane's center, LOOK_MOVING_EXTRA_
        // DISTANCE farther out, looking straight at the plane.
        let distance = static_offset.magnitude() + LOOK_MOVING_EXTRA_DISTANCE;
        let moving_position = target.position + target.rotation * (orbit * behind * distance);
        let moving_up = target.rotation * (orbit * Vector3::y());

        let blend = smoothstep(self.look_moving);
        (
            swung_position.lerp(&moving_position, blend),
            swung_look_at.lerp(&target.position, blend),
            swung_up.lerp(&moving_up, blend).try_normalize(1e-4).unwrap_or(swung_up),
        )
    }

    /// One raw stick axis with `LOOK_DEADZONE` removed: 0 inside it, then
    /// ramping 0..1 from its edge to full stick, sign kept.
    fn past_look_deadzone(value: f32) -> f32 {
        let magnitude = ((value.abs() - LOOK_DEADZONE) / (1.0 - LOOK_DEADZONE)).clamp(0.0, 1.0);
        magnitude * value.signum()
    }

}

impl Behavior for Camera {
    fn update(&mut self, node: &mut Node, _cameras: &mut SceneCameras, app: &mut App, delta_time: f32) {
        // F10 - the editor camera takes over while it's up.
        if self.editor.is_some() {
            self.update_editor(node, _cameras, app, delta_time);
            return;
        }
        // M - the map view takes over while it's up.
        if self.tactical.is_some() {
            self.update_tactical(node, app);
            return;
        }
        // The pause menu has the cursor. Looking stays where it was.
        if app.is_paused {
            return;
        }
        let cinematic = _cameras.active_name() == "animation_camera";
        // Its own lens, changed below and put back on the node at the end -
        // the whole screen's shape again (F10's editor view draws in only
        // part of it).
        let mut lens = node.get_property::<CameraLens>().copied().unwrap_or_default();
        lens.viewport = None;

        // Locked (the intro, an animation): no looking round - the view
        // eases back to its center.
        let locked = self.look_locked;
        let look = if locked || cinematic { LookInput::none() } else { LookInput::read() };
        // F7 (or the editor) frees the mouse - it's clicking buttons then, not
        // looking around (see App::mouse_free). The animation camera owns the
        // view meanwhile, so the player's own look stays put.
        let mouse = MouseLook::read(app.mouse_free() || locked || cinematic);
        if locked {
            let ease = (delta_time * LOOK_CENTER_RATE).clamp(0.0, 1.0);
            self.cockpit_yaw += (0.0 - self.cockpit_yaw) * ease;
            self.cockpit_pitch += (0.0 - self.cockpit_pitch) * ease;
        }
        // Runs in every mode, so switching into the cockpit picks the head
        // up wherever the G has it rather than snapping from rest.
        self.head_motion.update(self.head_input, delta_time);

        if let Some(target) = self.target.take() {
            // Calculate target camera position and look-at point
            let (target_position, target_look_at, target_up) = match self.state {
                CameraState::ThirdPerson => {
                    let (position, fov) = Self::named(&target, THIRD_PERSON_CAMERA, DEFAULT_THIRD_PERSON);
                    lens.znear = 0.1;
                    lens.fovy = fov;
                    let target_pos = target.position + (target.rotation * position);
                    let look_at = target.position + (target.rotation * Vector3::new(0.0, 0.0, 47.8));
                    (target_pos, look_at, target.rotation * *Vector3::y_axis())
                },
                CameraState::Cockpit => {
                    let base_pos = target.rotation * Self::named(&target, COCKPIT_CAMERA, (Vector3::new(0.0, 0.2, 1.3), 70.0)).0;
                    lens.znear = 0.01;

                    // Mouse wheel adjusts target FOV
                    let scroll = mouse.scroll;
                    if scroll != 0.0 {
                        self.cockpit_target_fov = (self.cockpit_target_fov - scroll * 5.0).clamp(20.0, 120.0);
                    }
                    // Lerp current FOV toward target
                    self.cockpit_current_fov = lerp(self.cockpit_current_fov, self.cockpit_target_fov, delta_time * 8.0);
                    let head = self.head_motion.pose(GAME_SETTINGS.lock().unwrap().head_g_reaction);
                    lens.fovy = self.cockpit_current_fov;

                    let max_yaw: f32 = 170.0;
                    let max_pitch: f32 = 70.0;
                    let sens = input::mouse_sensitivity();

                    // Update yaw/pitch from relative mouse, clamp immediately so no over-accumulation
                    self.cockpit_yaw = (self.cockpit_yaw - mouse.rel_x * sens.0).clamp(-max_yaw, max_yaw);
                    self.cockpit_pitch = (self.cockpit_pitch + mouse.rel_y * sens.1).clamp(-max_pitch, max_pitch);

                    // Right stick: same absolute mapping as the fixed
                    // modes' orbit, added on top of the mouse look and kept
                    // inside the same head-turn limits. Up is negative pitch
                    // here (mouse down = positive = look down).
                    let yaw = (self.cockpit_yaw - look.x * max_yaw).clamp(-max_yaw, max_yaw);
                    let pitch = (self.cockpit_pitch - look.y * max_pitch).clamp(-max_pitch, max_pitch);
                    // The resting view angle from Settings, on top of the
                    // look - positive looks up, and up is negative pitch here.
                    let view_pitch_target = GAME_SETTINGS.lock().unwrap().cockpit_view_pitch_deg;
                    self.cockpit_view_pitch += (view_pitch_target - self.cockpit_view_pitch) * (1.0 - (-delta_time * COCKPIT_VIEW_PITCH_EASE_RATE).exp());
                    let pitch = pitch - self.cockpit_view_pitch;

                    // Target rotation from mouse
                    let rotation_y = UnitQuaternion::from_axis_angle(&Vector3::y_axis(), yaw.to_radians());
                    let rotation_x = UnitQuaternion::from_axis_angle(&Vector3::x_axis(), pitch.to_radians());
                    let target_rotation = rotation_y * rotation_x;

                    // Lerp the rotation for smooth feel
                    self.cockpit_current_rotation = UnitQuaternion::new_normalize(lerp_quaternion(
                        self.cockpit_current_rotation.into_inner(),
                        target_rotation.into_inner(),
                        delta_time * 12.0,
                    ));

                    // Over the shoulder (see COCKPIT_LOOK_BEHIND) - from
                    // where the view actually points right now, so the head
                    // moves exactly as the view turns. +X is the side a
                    // positive yaw turns toward.
                    let facing = self.cockpit_current_rotation * Vector3::z();
                    let turned_yaw = facing.x.atan2(facing.z);
                    let behind = (turned_yaw.abs() / std::f32::consts::PI).clamp(0.0, 1.0).powf(COCKPIT_LOOK_BEHIND.curve);
                    let shoulder_offset = Vector3::new(
                        COCKPIT_LOOK_BEHIND.side * turned_yaw.signum(),
                        COCKPIT_LOOK_BEHIND.up,
                        COCKPIT_LOOK_BEHIND.forward,
                    ) * behind;
                    let head_offset = target.rotation * (shoulder_offset + head.offset);

                    let target_pos = target.position + base_pos + head_offset;

                    // The G tilt sits under the pilot's own look - the neck
                    // leans, and the head turns on top of that.
                    let head_rotation = target.rotation * head.tilt;
                    let look_dir = head_rotation * self.cockpit_current_rotation * Vector3::new(0.0, 0.0, 100.0);
                    let look_at = target_pos + look_dir;
                    (target_pos, look_at, head_rotation * *Vector3::y_axis())
                },
                CameraState::Front => {
                    let (position, fov) = Self::named(&target, FRONT_CAMERA, DEFAULT_FRONT);
                    lens.znear = 0.1;
                    lens.fovy = fov;
                    let target_pos = target.position + (target.rotation * position);
                    let look_at = target.position;
                    (target_pos, look_at, target.rotation * *Vector3::y_axis())
                },
                CameraState::Free => {
                    lens.znear = 0.1;

                    // Mouse wheel adjusts target FOV
                    let scroll = mouse.scroll;
                    if scroll != 0.0 {
                        self.free_target_fov = (self.free_target_fov - scroll * 5.0).clamp(20.0, 120.0);
                    }
                    self.free_current_fov = lerp(self.free_current_fov, self.free_target_fov, delta_time * 8.0);
                    lens.fovy = self.free_current_fov;

                    let sens = input::mouse_sensitivity();
                    self.free_yaw = self.free_yaw - mouse.rel_x * sens.0;
                    self.free_pitch = (self.free_pitch + mouse.rel_y * sens.1).clamp(-89.0, 89.0);

                    // Right stick turns the view like the mouse, at a
                    // steady rate - stick up matches mouse up.
                    let stick_step = FREE_LOOK_STICK_RATE_DEG_S * delta_time;
                    self.free_yaw -= look.x * stick_step;
                    self.free_pitch = (self.free_pitch - look.y * stick_step).clamp(-89.0, 89.0);

                    let rotation_y = UnitQuaternion::from_axis_angle(&Vector3::y_axis(), self.free_yaw.to_radians());
                    let rotation_x = UnitQuaternion::from_axis_angle(&Vector3::x_axis(), self.free_pitch.to_radians());
                    let target_rotation = rotation_y * rotation_x;

                    self.free_current_rotation = UnitQuaternion::new_normalize(lerp_quaternion(
                        self.free_current_rotation.into_inner(),
                        target_rotation.into_inner(),
                        delta_time * 12.0,
                    ));

                    // The orbit (yaw/pitch above) is measured in some frame:
                    // - Video setting "follows plane" on: the plane's own frame,
                    //   GTA-vehicle style - yaw/pitch 0 sits right behind the
                    //   plane, left/right swings round the plane's own wing
                    //   plane, and the whole view turns/rolls with the plane.
                    // - off: the world's - the view stays level with the
                    //   horizon whatever the plane does.
                    let frame = if self.wreck_sea_level.is_none() && GAME_SETTINGS.lock().unwrap().free_camera_follows_plane {
                        target.rotation
                    } else {
                        UnitQuaternion::identity()
                    };
                    let orbit = frame * self.free_current_rotation;

                    let distance_target = if self.wreck_sea_level.is_some() { WRECK_DISTANCE } else { FREE_DISTANCE };
                    self.free_distance = lerp(self.free_distance, distance_target, (delta_time * FREE_DISTANCE_RATE).clamp(0.0, 1.0));

                    // Round the aircraft's free camera point.
                    let mut look_at = target.position + target.rotation * Self::named(&target, FREE_CAMERA, DEFAULT_FREE).0;
                    let mut target_pos = look_at + orbit * Vector3::new(0.0, 0.0, -self.free_distance);
                    if let Some(sea_level) = self.wreck_sea_level {
                        look_at.y = look_at.y.max(sea_level);
                        target_pos = look_at + orbit * Vector3::new(0.0, 0.0, -self.free_distance);
                        target_pos.y = target_pos.y.max(sea_level + WRECK_MIN_HEIGHT);
                    }
                    (target_pos, look_at, orbit * Vector3::y())
                },
            };

            // Fixed modes' right-stick orbit - applied further down, after
            // the F7 editor has read this state's base offset, so its
            // readout doesn't spin with the stick.
            let orbits = matches!(self.state, CameraState::ThirdPerson | CameraState::Front);

            // Plane-relative offset of whatever the current state chose - for
            // the F7 Camera Editor's "base" readout.
            self.debug_base_offset = target.rotation.inverse() * (target_position - target.position);

            // Debug mode overlay: move camera offset with arrow keys / W / S
            let final_position = if self.debug_mode_active {
                let speed = 2.0 * delta_time;
                let mut changed = false;

                if input::is_action_pressed("throttle_up") {
                    self.debug_offset.z += speed;
                    changed = true;
                }
                if input::is_action_pressed("throttle_down") {
                    self.debug_offset.z -= speed;
                    changed = true;
                }
                if input::is_action_pressed("up_wheel") {
                    self.debug_offset.x += speed;
                    changed = true;
                }
                if input::is_action_pressed("down_wheel") {
                    self.debug_offset.x -= speed;
                    changed = true;
                }
                if input::is_action_pressed("pitch_up") {
                    self.debug_offset.y += speed;
                    changed = true;
                }
                if input::is_action_pressed("pitch_down") {
                    self.debug_offset.y -= speed;
                    changed = true;
                }

                // `debug_offset` is a DELTA from the current state's own base
                // offset - so nudging it starts from where the camera already
                // is and switching camera states keeps the tweak. The value
                // to paste into a camera definition is base + delta.
                let resulting = self.debug_base_offset + self.debug_offset;
                if changed {
                    println!("Camera offset: Vector3::new({:.3}, {:.3}, {:.3})",
                        resulting.x, resulting.y, resulting.z);
                }

                target.position + (target.rotation * resulting)
            } else {
                target_position
            };

            let (final_position, target_look_at, target_up) = if orbits {
                self.orbit_look(&look, &target, final_position, target_look_at, target_up, delta_time)
            } else {
                self.look_yaw = 0.0;
                self.look_pitch = 0.0;
                self.look_moving = 0.0;
                (final_position, target_look_at, target_up)
            };

            // On its pose this frame. Coming back from an animation's camera
            // cuts here instead of easing across.
            set_pose(node, final_position, target_look_at, target_up, lens);

            self.target = Some(target);
        }

        if input::is_action_just_pressed("change_camera") && self.wreck_sea_level.is_none() && !self.look_locked {
            self.next_camera();
        }
    }
}

/// Puts the camera's node at `position`, looking at `look_at` with `up`
/// as its up, with `lens` - the node's camera follows (see
/// Scene::sync_cameras).
fn set_pose(node: &mut Node, position: Vector3<f32>, look_at: Vector3<f32>, up: Vector3<f32>, lens: CameraLens) {
    let direction = look_at - position;
    let rotation = match (direction.try_normalize(1e-6), up.try_normalize(1e-6)) {
        (Some(direction), Some(up)) if direction.cross(&up).norm() > 1e-4 => UnitQuaternion::face_towards(&direction, &up),
        _ => facing(direction).unwrap_or_else(UnitQuaternion::identity),
    };
    let scale = node.get_property::<Transform3D>().map(|transform| transform.scale).unwrap_or_else(|| Vector3::new(1.0, 1.0, 1.0));
    node.set_property(Transform3D { position, rotation, scale });
    node.set_property(lens);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tactical view orbiting the origin from straight behind it along
    /// -Z (so looking toward +Z, where +X is to the left).
    fn camera_looking_along_plus_z() -> Camera {
        let mut camera = Camera::new();
        camera.tactical = Some(TacticalOrbit { pivot: Some(Vector3::zeros()), follow: true, yaw: -90.0, pitch: 45.0, distance: 1000.0 });
        camera
    }

    #[test]
    fn dragging_the_map_moves_it_with_the_pointer() {
        let mut camera = camera_looking_along_plus_z();
        // Drag right: the map moves right, so the view moves left - +X.
        camera.pan_tactical((100.0, 0.0), 720.0);
        let pivot = camera.tactical.as_ref().unwrap().pivot.unwrap();
        assert!(pivot.x > 0.0 && pivot.z.abs() < 1e-3, "{pivot:?}");
        // Drag down: the map comes toward the camera, so the view moves away - +Z.
        camera.pan_tactical((0.0, 100.0), 720.0);
        let pivot = camera.tactical.as_ref().unwrap().pivot.unwrap();
        assert!(pivot.z > 0.0, "{pivot:?}");
        assert!(!camera.tactical.as_ref().unwrap().follow, "dragging stops following");
    }
}
