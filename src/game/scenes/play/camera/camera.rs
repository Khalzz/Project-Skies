use nalgebra::{Point3, UnitQuaternion, Vector3};

use crate::app::App;
use crate::engine::game_nodes::game_object::Cameras;
use crate::engine::input::input;
use crate::engine::rendering::camera::handler::SceneCameras;
use crate::engine::scene_manager::behavior::Behavior;
use crate::engine::scene_manager::node::Node;
use crate::engine::utils::lerps::{lerp, lerp_point3, lerp_quaternion, smoothstep};
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
/// Fixed modes (Normal/Cinematic/Frontal): full sideways stick orbits this
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
/// for the debug overlay (F8).
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

pub enum CameraState {
    Normal,
    Cockpit,
    Cinematic,
    Frontal,
    Free,
}

struct CinematicReturnBlend {
    elapsed: f32,
    duration: f32,
    from_position: Point3<f32>,
    from_look_at: Point3<f32>,
    from_fov: f32,
}

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
    cinematic_return_blend: Option<CinematicReturnBlend>,
    // Free mode's current orbit distance, eased toward FREE_DISTANCE (or
    // WRECK_DISTANCE after a crash).
    free_distance: f32,
    // The water's height, once the plane has crashed - see `enter_wreck_view`.
    wreck_sea_level: Option<f32>,
}

impl Camera {
    pub fn new() -> Self {
        Self {
            state: CameraState::Normal,
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
            cinematic_return_blend: None,
            free_distance: FREE_DISTANCE,
            wreck_sea_level: None,
        }
    }

    pub fn state_name(&self) -> &'static str {
        match self.state {
            CameraState::Normal => "Normal",
            CameraState::Cockpit => "Cockpit",
            CameraState::Cinematic => "Cinematic",
            CameraState::Frontal => "Frontal",
            CameraState::Free => "Free",
        }
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

    pub fn snapshot_cinematic_return(&mut self, cameras: &SceneCameras) {
        let cam = cameras.active();
        self.cinematic_return_blend = Some(CinematicReturnBlend {
            elapsed: 0.0,
            duration: 0.6,
            from_position: cam.camera.position(),
            from_look_at: cam.camera.position() + cam.camera.calc_forward_direction() * 100.0,
            from_fov: cam.projection.fovy,
        });
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

    fn next_camera(&mut self) {
        match self.state {
            CameraState::Normal => {
                self.state = CameraState::Free;
            },
            CameraState::Cockpit => self.state = CameraState::Cinematic,
            CameraState::Cinematic => self.state = CameraState::Frontal,
            CameraState::Frontal => self.state = CameraState::Normal,
            CameraState::Free => self.state = CameraState::Cockpit,
        }
    }
}

impl Behavior for Camera {
    fn update(&mut self, _node: &mut Node, cameras: &mut SceneCameras, app: &mut App, delta_time: f32) {
        let look = LookInput::read();
        // F8 frees the mouse for the debug overlay - it's clicking buttons
        // then, not looking around (see App::debug_mouse_free).
        let mouse = MouseLook::read(app.debug_mouse_free);
        // Runs in every mode, so switching into the cockpit picks the head
        // up wherever the G has it rather than snapping from rest.
        self.head_motion.update(self.head_input, delta_time);

        if let Some(target) = self.target.take() {
            // Calculate target camera position and look-at point
            let (target_position, target_look_at, target_up) = match self.state {
                CameraState::Normal => {
                    let active = cameras.active_mut();
                    active.projection.znear = 0.1;
                    active.projection.fovy = NORMAL_CAMERA_FOV;
                    let target_pos = target.position + (target.rotation * Vector3::new(0.0, 3.85, -23.77));
                    let look_at = target.position + (target.rotation * Vector3::new(0.0, 0.0, 47.8));
                    (target_pos, look_at, target.rotation * *Vector3::y_axis())
                },
                CameraState::Cockpit => {
                    let (base_pos, _default_fov) = if let Some(named_cameras) = &target.cameras {
                        if let Some(cam) = named_cameras.get("cockpit") {
                            (target.rotation * cam.position, cam.fov)
                        } else {
                            (target.rotation * Vector3::new(0.0, 0.2, 1.3), 70.0)
                        }
                    } else {
                        (target.rotation * Vector3::new(0.0, 0.2, 1.3), 70.0)
                    };
                    cameras.active_mut().projection.znear = 0.01;

                    // Mouse wheel adjusts target FOV
                    let scroll = mouse.scroll;
                    if scroll != 0.0 {
                        self.cockpit_target_fov = (self.cockpit_target_fov - scroll * 5.0).clamp(20.0, 120.0);
                    }
                    // Lerp current FOV toward target
                    self.cockpit_current_fov = lerp(self.cockpit_current_fov, self.cockpit_target_fov, delta_time * 8.0);
                    let head = self.head_motion.pose(GAME_SETTINGS.lock().unwrap().head_g_reaction);
                    cameras.active_mut().projection.fovy = self.cockpit_current_fov;

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
                CameraState::Cinematic => {
                    let (target_pos, fov) = if let Some(named_cameras) = &target.cameras {
                        if let Some(cam) = named_cameras.get("cinematic") {
                            (target.position + (target.rotation * cam.position), cam.fov)
                        } else {
                            (target.position + (target.rotation * Vector3::new(-1.0, 3.0, -1.0)), 60.0)
                        }
                    } else {
                        (target.position + (target.rotation * Vector3::new(-1.0, 3.0, -1.0)), 60.0)
                    };
                    cameras.active_mut().projection.fovy = fov;
                    let look_at = target.position + (target.rotation * Vector3::new(30.0, 0.0, 100.0));
                    (target_pos, look_at, target.rotation * *Vector3::y_axis())
                },
                CameraState::Frontal => {
                    let (target_pos, fov) = if let Some(named_cameras) = &target.cameras {
                        if let Some(cam) = named_cameras.get("frontal") {
                            (target.position + (target.rotation * cam.position), cam.fov)
                        } else {
                            (target.position + (target.rotation * Vector3::new(0.0, 2.0, 3.0)), 60.0)
                        }
                    } else {
                        (target.position + (target.rotation * Vector3::new(0.0, 2.0, 3.0)), 60.0)
                    };
                    cameras.active_mut().projection.fovy = fov;
                    let look_at = target.position;
                    (target_pos, look_at, target.rotation * *Vector3::y_axis())
                },
                CameraState::Free => {
                    cameras.active_mut().projection.znear = 0.1;

                    // Mouse wheel adjusts target FOV
                    let scroll = mouse.scroll;
                    if scroll != 0.0 {
                        self.free_target_fov = (self.free_target_fov - scroll * 5.0).clamp(20.0, 120.0);
                    }
                    self.free_current_fov = lerp(self.free_current_fov, self.free_target_fov, delta_time * 8.0);
                    cameras.active_mut().projection.fovy = self.free_current_fov;

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

                    let mut look_at = target.position;
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
            let orbits = matches!(self.state, CameraState::Normal | CameraState::Cinematic | CameraState::Frontal);

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

            // Apply camera position directly (no interpolation to match object
            // movement) - except right after a CameraTrack::Shot ends, where
            // cinematic_return_blend (see its own doc comment) eases from the
            // Shot's last pose into whatever's computed above instead of cutting.
            let target_position: Point3<f32> = final_position.into();
            let target_look_at: Point3<f32> = target_look_at.into();
            let target_fov = cameras.active().projection.fovy;

            let (blended_position, blended_look_at, blended_fov) = match &mut self.cinematic_return_blend {
                Some(blend) => {
                    blend.elapsed += delta_time;
                    let t = smoothstep((blend.elapsed / blend.duration).clamp(0.0, 1.0));
                    let blended = (
                        lerp_point3(blend.from_position, target_position, t),
                        lerp_point3(blend.from_look_at, target_look_at, t),
                        lerp(blend.from_fov, target_fov, t),
                    );
                    if blend.elapsed >= blend.duration {
                        self.cinematic_return_blend = None;
                    }
                    blended
                }
                None => (target_position, target_look_at, target_fov),
            };

            let active = cameras.active_mut();
            active.camera.set_position(blended_position);
            active.camera.look_at(blended_look_at);
            active.camera.up = target_up;
            active.projection.fovy = blended_fov;

            self.target = Some(target);
        }

        if input::is_action_just_pressed("change_camera") && self.wreck_sea_level.is_none() {
            self.next_camera();
        }
        // F5 - offsets whichever named camera is already active (see
        // final_position above).
        if input::is_action_just_pressed("toggle_camera_offset_debug") {
            self.debug_mode_active = !self.debug_mode_active;
            println!("Camera debug mode: {}", if self.debug_mode_active { "ON" } else { "OFF" });
        }
    }
}
