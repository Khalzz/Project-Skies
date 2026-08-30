use nalgebra::{Point3, UnitQuaternion, Vector3};

use crate::app::App;
use crate::engine::game_nodes::game_object::Cameras;
use crate::engine::input::input;
use crate::engine::rendering::camera::handler::SceneCameras;
use crate::engine::scene_manager::behavior::Behavior;
use crate::engine::scene_manager::node::Node;
use crate::engine::utils::lerps::{lerp, lerp_point3, lerp_quaternion, smoothstep};

pub const NORMAL_CAMERA_FOV: f32 = 45.0;

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
            debug_offset: Vector3::new(0.0, 0.6, -3.0),
            debug_mode_active: false,
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
            cinematic_return_blend: None,
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
        if let Some(target) = &self.target {
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

                    let yaw = self.cockpit_yaw;
                    let pitch = self.cockpit_pitch;

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

                    let rotation_y = UnitQuaternion::from_axis_angle(&Vector3::y_axis(), self.free_yaw.to_radians());
                    let rotation_x = UnitQuaternion::from_axis_angle(&Vector3::x_axis(), self.free_pitch.to_radians());
                    let target_rotation = rotation_y * rotation_x;

                    self.free_current_rotation = UnitQuaternion::new_normalize(lerp_quaternion(
                        self.free_current_rotation.into_inner(),
                        target_rotation.into_inner(),
                        delta_time * 12.0,
                    ));

                    let target_pos = target.position + (self.free_current_rotation * Vector3::new(0.0, 0.0, -45.0));
                    let look_at = target.position;
                    (target_pos, look_at, *Vector3::y_axis())
                },
            };

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

                if changed {
                    println!("Camera offset: Vector3::new({:.3}, {:.3}, {:.3})",
                        self.debug_offset.x,
                        self.debug_offset.y,
                        self.debug_offset.z);
                }

                // Apply debug offset relative to the plane
                target.position + (target.rotation * self.debug_offset)
            } else {
                target_position
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
