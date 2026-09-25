use nalgebra::{Point3, UnitQuaternion, Vector3};

use crate::app::App;
use crate::engine::game_nodes::game_object::Cameras;
use crate::engine::input::input;
use crate::engine::rendering::camera::handler::SceneCameras;
use crate::engine::scene_manager::behavior::Behavior;
use crate::engine::scene_manager::node::Node;
use crate::engine::utils::lerps::{lerp, lerp_point3, lerp_quaternion, smoothstep};
use crate::game::game_settings::GAME_SETTINGS;

pub const NORMAL_CAMERA_FOV: f32 = 45.0;

// Right-stick camera look (see `LookInput`). Matches the "look_*" actions'
// own deadzone in settings/input.ron - the stick reads 0 below it, and
// `LookInput::read` rescales what's past it back to 0..1 so the camera
// starts moving from zero instead of snapping to the deadzone's angle.
const LOOK_STICK_DEADZONE: f32 = 0.15;
/// Fixed modes (Normal/Cinematic/Frontal): full sideways stick orbits this
/// far around the plane - 180 = all the way round to its nose, looking back.
const LOOK_MAX_YAW_DEG: f32 = 180.0;
/// Fixed modes: full up/down stick orbits to straight below/above the plane
/// (stops just short of 90 so the view never lines up with its own up axis).
const LOOK_MAX_PITCH_DEG: f32 = 89.0;
/// How fast the orbit follows the stick (per second) - quick, but eases
/// instead of snapping, and eases back to the base view on release.
const LOOK_FOLLOW_RATE: f32 = 10.0;
/// Fixed modes: how much farther from the plane the camera sits whenever
/// the view is swung off its base (0.35 = 35% farther) - one fixed distance,
/// not scaled by how far it's swung, so it holds steady while looking around.
const LOOK_PULLBACK: f32 = 0.35;
/// How fast the camera moves out to / back in from that distance (per second).
const LOOK_PULLBACK_RATE: f32 = 8.0;
/// Fixed modes' look dead zone, as a fraction of stick travel: inside it the
/// camera neither turns nor pulls back - it holds its base view. Past it, the
/// orbit ramps up from zero at the dead zone's edge to full at full stick
/// (per axis), and the pull-back kicks in.
const LOOK_DEADZONE: f32 = 0.25;
/// Once the stick is back inside the dead zone, the camera moves back in
/// when the eased view is within this many degrees of its base.
const LOOK_CENTRED_DEG: f32 = 0.5;
/// Free mode: turn rate at full stick, deg/s - slower than a mouse flick.
const FREE_LOOK_STICK_RATE_DEG_S: f32 = 130.0;

/// The right stick, as look input - both axes -1..1, deadzone removed.
/// `x` positive = look right, `y` positive = look up.
struct LookInput {
    x: f32,
    y: f32,
}

impl LookInput {
    fn read() -> Self {
        Self {
            x: Self::rescale(input::get_axis("look_left", "look_right")),
            y: Self::rescale(input::get_axis("look_down", "look_up")),
        }
    }

    fn rescale(value: f32) -> f32 {
        let magnitude = ((value.abs() - LOOK_STICK_DEADZONE) / (1.0 - LOOK_STICK_DEADZONE)).clamp(0.0, 1.0);
        magnitude * value.signum()
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
    pub cockpit_current_head_shift: f32,
    pub free_yaw: f32,
    pub free_pitch: f32,
    pub free_current_rotation: UnitQuaternion<f32>,
    pub free_target_fov: f32,
    pub free_current_fov: f32,
    // Fixed modes' right-stick orbit around the plane, degrees, eased toward
    // the stick's absolute position - see `orbit_look`.
    look_yaw: f32,
    look_pitch: f32,
    // 0 = base distance, 1 = pulled back by LOOK_PULLBACK - see `orbit_look`.
    look_pullback: f32,
    cinematic_return_blend: Option<CinematicReturnBlend>,
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
            cockpit_current_head_shift: 0.0,
            free_yaw: 0.0,
            free_pitch: 0.0,
            free_current_rotation: UnitQuaternion::identity(),
            free_target_fov: 60.0,
            free_current_fov: 60.0,
            look_yaw: 0.0,
            look_pitch: 0.0,
            look_pullback: 0.0,
            cinematic_return_blend: None,
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

    /// Fixed modes' Ace Combat-style look: the stick's position IS the
    /// angle - centred is the mode's own base view, full sideways swings the
    /// camera round to the plane's nose (looking back), half is a side view,
    /// full up drops the camera straight below the plane to look up at it,
    /// full down lifts it straight above to look down. Orbits the whole view (position,
    /// look-at and up) around the plane in its own frame, so it follows the
    /// plane through any attitude. Only called for Normal/Cinematic/Frontal.
    fn orbit_look(&mut self, look: &LookInput, target: &CameraTarget, position: Vector3<f32>, look_at: Vector3<f32>, up: Vector3<f32>, delta_time: f32) -> (Vector3<f32>, Vector3<f32>, Vector3<f32>) {
        // Negative yaw swings the view right - same sign convention as the
        // mouse look in Cockpit/Free (mouse right decreases yaw).
        let look_x = Self::past_look_deadzone(look.x);
        let look_y = Self::past_look_deadzone(look.y);

        let t = (delta_time * LOOK_FOLLOW_RATE).clamp(0.0, 1.0);
        self.look_yaw = lerp(self.look_yaw, -look_x * LOOK_MAX_YAW_DEG, t);
        // Negative pitch swings the camera below the plane - so stick up
        // (look.y > 0) looks up at it from underneath.
        self.look_pitch = lerp(self.look_pitch, -look_y * LOOK_MAX_PITCH_DEG, t);

        let local = UnitQuaternion::from_axis_angle(&Vector3::y_axis(), self.look_yaw.to_radians())
            * UnitQuaternion::from_axis_angle(&Vector3::x_axis(), self.look_pitch.to_radians());
        let orbit = target.rotation * local * target.rotation.inverse();

        // Pull back to one fixed distance as soon as the stick leaves the
        // dead zone, and hold it there however far it's swung; only move back
        // in once the stick is back inside it AND the view has eased home.
        let stick_active = look_x != 0.0 || look_y != 0.0;
        let off_centre = stick_active || self.look_yaw.abs() > LOOK_CENTRED_DEG || self.look_pitch.abs() > LOOK_CENTRED_DEG;
        let pullback_target = if off_centre { 1.0 } else { 0.0 };
        self.look_pullback = lerp(self.look_pullback, pullback_target, (delta_time * LOOK_PULLBACK_RATE).clamp(0.0, 1.0));
        let distance_scale = 1.0 + LOOK_PULLBACK * self.look_pullback;

        (
            target.position + orbit * (position - target.position) * distance_scale,
            target.position + orbit * (look_at - target.position),
            orbit * up,
        )
    }

    /// One stick axis with `LOOK_DEADZONE` removed: 0 inside it, then
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
    fn update(&mut self, _node: &mut Node, cameras: &mut SceneCameras, _app: &mut App, delta_time: f32) {
        let look = LookInput::read();

        if let Some(target) = self.target.take() {
            // Calculate target camera position and look-at point
            let (target_position, target_look_at, target_up) = match self.state {
                CameraState::Normal => {
                    let active = cameras.active_mut();
                    active.projection.znear = 0.1;
                    active.projection.fovy = NORMAL_CAMERA_FOV;
                    let target_pos = target.position + (target.rotation * Vector3::new(0.0, 8.0, -50.0));
                    let look_at = target.position + (target.rotation * Vector3::new(0.0, 0.0, 100.0));
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
                    let scroll = input::mouse_scroll_y();
                    if scroll != 0.0 {
                        self.cockpit_target_fov = (self.cockpit_target_fov - scroll * 5.0).clamp(20.0, 120.0);
                    }
                    // Lerp current FOV toward target
                    self.cockpit_current_fov = lerp(self.cockpit_current_fov, self.cockpit_target_fov, delta_time * 8.0);
                    cameras.active_mut().projection.fovy = self.cockpit_current_fov;

                    let max_yaw: f32 = 170.0;
                    let max_pitch: f32 = 70.0;
                    let sens = input::mouse_sensitivity();

                    // Update yaw/pitch from relative mouse, clamp immediately so no over-accumulation
                    self.cockpit_yaw = (self.cockpit_yaw - input::mouse_rel_x() as f32 * sens.0).clamp(-max_yaw, max_yaw);
                    self.cockpit_pitch = (self.cockpit_pitch + input::mouse_rel_y() as f32 * sens.1).clamp(-max_pitch, max_pitch);

                    // Right stick: same absolute mapping as the fixed
                    // modes' orbit, added on top of the mouse look and kept
                    // inside the same head-turn limits. Up is negative pitch
                    // here (mouse down = positive = look down).
                    let yaw = (self.cockpit_yaw - look.x * max_yaw).clamp(-max_yaw, max_yaw);
                    let pitch = (self.cockpit_pitch - look.y * max_pitch).clamp(-max_pitch, max_pitch);

                    // Target head shift (cubic easing near the limit)
                    let t = (yaw / max_yaw).clamp(-1.0, 1.0);
                    let target_head_shift = 0.03 * t * t * t.signum();
                    self.cockpit_current_head_shift = lerp(self.cockpit_current_head_shift, target_head_shift, delta_time * 8.0);
                    let head_offset = target.rotation * Vector3::new(self.cockpit_current_head_shift, 0.0, 0.0);

                    let target_pos = target.position + base_pos + head_offset;

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

                    let look_dir = target.rotation * self.cockpit_current_rotation * Vector3::new(0.0, 0.0, 100.0);
                    let look_at = target_pos + look_dir;
                    (target_pos, look_at, target.rotation * *Vector3::y_axis())
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
                    let scroll = input::mouse_scroll_y();
                    if scroll != 0.0 {
                        self.free_target_fov = (self.free_target_fov - scroll * 5.0).clamp(20.0, 120.0);
                    }
                    self.free_current_fov = lerp(self.free_current_fov, self.free_target_fov, delta_time * 8.0);
                    cameras.active_mut().projection.fovy = self.free_current_fov;

                    let sens = input::mouse_sensitivity();
                    self.free_yaw = self.free_yaw - input::mouse_rel_x() as f32 * sens.0;
                    self.free_pitch = (self.free_pitch + input::mouse_rel_y() as f32 * sens.1).clamp(-89.0, 89.0);

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
                    let frame = if GAME_SETTINGS.lock().unwrap().free_camera_follows_plane {
                        target.rotation
                    } else {
                        UnitQuaternion::identity()
                    };
                    let orbit = frame * self.free_current_rotation;

                    let target_pos = target.position + orbit * Vector3::new(0.0, 0.0, -45.0);
                    let look_at = target.position;
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
                self.look_pullback = 0.0;
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

        if input::is_action_just_pressed("change_camera") {
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
