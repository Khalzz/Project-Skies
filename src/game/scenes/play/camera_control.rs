//! The play scene's cameras: what the player's camera follows, and which
//! camera is on screen.
//!
//! The cameras themselves are nodes - the player's view (`PLAYER_CAMERA`,
//! moved every frame by its own `Camera` behavior, see play::camera::camera)
//! and the animations' (`ANIMATION_CAMERA`, put where their camera keys say);
//! the map's own camera nodes (see play::map_nodes) can be looked through
//! from F10's editor. `CameraControl` is what decides, each frame:
//! - what the player's camera follows - F1 the player (again: its next view),
//!   F2 the next aircraft, F4 the next missile, the editor's selection - back
//!   to the player when that's gone; and hands the camera its target
//!   (`update`);
//! - in F10's editor, keeping the editor camera with what was picked;
//! - which camera is on screen (`update_view`);
//! - where the animation camera is (`set_animation_pose`).
//!
//! It knows nothing else about the scene: the play scene tells it what there
//! is to follow (`Followable`) and what wants the screen (`ViewRequest`).
//! Everything else reaches the player's camera through `player_camera` /
//! `player_camera_mut`.

use std::collections::HashMap;

use nalgebra::{UnitQuaternion, Vector3};
use pankarta_software::prelude::*;

use crate::game::ext::game_object::{Camera as GameObjectCamera, Cameras};
use crate::game::ext::physics_behavior::PhysicsNodeExt;
use super::animations::CameraPose;
use super::camera::camera::{Camera, CameraState};
use super::camera::head_motion::HeadInput;
use super::plane::messages::AircraftState;
use super::plane::plane::Plane;
use super::weapon::missiles::Missiles;

/// The player's view - a camera node driven by play::camera::camera::Camera.
pub const PLAYER_CAMERA: &str = "camera";
/// The camera animations' camera keys move (see play::animations).
pub const ANIMATION_CAMERA: &str = "animation_camera";
/// How far F10's editor camera stands off what it focuses on (m) - an
/// aircraft, a missile, anything else.
pub const EDITOR_FOCUS_AIRCRAFT: f32 = 60.0;
pub const EDITOR_FOCUS_NODE: f32 = 25.0;
pub const EDITOR_FOCUS_MISSILE: f32 = 15.0;

/// What there is for the camera to follow this frame - from the play scene.
pub struct Followable<'a> {
    /// The aircraft the player flies.
    pub player_id: &'a str,
    /// Every aircraft on the map, in its order.
    pub aircraft: Vec<&'a str>,
    /// The missiles in flight - followed along their flight, not turned like
    /// their models.
    pub missiles: &'a Missiles,
    /// The aircraft picked in the flight editor's hierarchy, if any - picking
    /// a new one points the camera at it.
    pub editor_selection: Option<&'a str>,
    /// F10's editor is open, and a text field in it has the keys.
    pub editor_active: bool,
    pub editor_typing: bool,
}

/// What wants the screen this frame - see `CameraControl::update_view`.
pub struct ViewRequest {
    /// M's map is up.
    pub tactical_map: bool,
    /// F10's editor is up, the camera it's looking through ("View
    /// through"), if any, and whether it shows the animation's camera
    /// ("Camera in view").
    pub editor: bool,
    pub preview_camera: Option<String>,
    pub timeline_preview: bool,
    /// A running animation's camera keys drive the animation camera.
    pub animating: bool,
}

/// How a key press changes the player camera's view.
enum CameraSwitch {
    To(CameraState),
    Next,
}

pub struct CameraControl {
    /// The node the player's camera follows - empty until the first frame
    /// (then the player's).
    focus: String,
    /// The aircraft selected in the flight editor last frame.
    editor_selection: Option<String>,
    /// F10's editor camera keeping up with what F1-F4 picked: its node id and
    /// where it was last frame - see update_editor_follow.
    editor_follow: Option<(String, Vector3<f32>)>,
    /// Where each wreck in the sea went in, on the surface, and which way it
    /// was heading - what the camera stays on (not the sinking wreck).
    went_in_at: HashMap<String, (Vector3<f32>, UnitQuaternion<f32>)>,
    /// The camera last put on screen - see update_view.
    on_screen: String,
}

impl Default for CameraControl {
    fn default() -> Self {
        Self::new()
    }
}

impl CameraControl {
    pub fn new() -> Self {
        Self { focus: String::new(), editor_selection: None, editor_follow: None, went_in_at: HashMap::new(), on_screen: PLAYER_CAMERA.to_owned() }
    }

    /// The scene's two camera nodes: the player's view (its Camera behavior
    /// places it every frame, fed its target by `update`) and the one the
    /// animations' camera keys move.
    pub fn spawn(scene: &mut Scene, app: &mut App) {
        scene.spawn_node(app,
            Node::new(PLAYER_CAMERA)
                .with_property(Transform3D::default())
                .with_property(CameraLens::new(45.0))
                .with_behavior(Camera::new())
        ).expect("play should only spawn 'camera' once");
        scene.spawn_node(app,
            Node::new(ANIMATION_CAMERA)
                .with_property(Transform3D::default())
                .with_property(CameraLens::new(60.0))
        ).expect("play should only spawn the animation camera once");
    }

    /// The player's camera behavior.
    pub fn player_camera(scene: &Scene) -> Option<&Camera> {
        scene.content.nodes.get(PLAYER_CAMERA).and_then(|node| node.get_behavior::<Camera>())
    }

    pub fn player_camera_mut(scene: &mut Scene) -> Option<&mut Camera> {
        scene.content.nodes.get_mut(PLAYER_CAMERA).and_then(|node| node.get_behavior_mut::<Camera>())
    }

    /// The node the player's camera follows (empty before the first frame).
    pub fn focus(&self) -> &str {
        &self.focus
    }

    /// Follows `id` from now on (the tactical map's "follow" button).
    pub fn set_focus(&mut self, id: &str) {
        self.focus = id.to_owned();
    }

    /// The camera on screen.
    pub fn on_screen(&self) -> &str {
        &self.on_screen
    }

    /// The camera follows `id` from its cockpit.
    pub fn in_cockpit_of(&self, scene: &Scene, id: &str) -> bool {
        self.focus == id && Self::player_camera(scene).is_some_and(Camera::is_cockpit)
    }

    /// The player's camera is on screen, in a cockpit - the view the HUD
    /// belongs to.
    pub fn cockpit_on_screen(&self, scene: &Scene) -> bool {
        self.on_screen == PLAYER_CAMERA && Self::player_camera(scene).is_some_and(Camera::is_cockpit)
    }

    /// F10's editor: flies the editor camera to `id`, `distance` off (a
    /// pick from the hierarchy) - and stops keeping up with what F1-F4 had
    /// picked.
    pub fn editor_focus_on(&mut self, scene: &mut Scene, id: &str, distance: f32) {
        self.editor_follow = None;
        if let Some(world) = scene.world_transform(id) {
            if let Some(camera) = Self::player_camera_mut(scene) {
                camera.editor_focus(world.position, distance);
            }
        }
    }

    /// This frame's camera work before the behaviors run: what it follows
    /// (keys, the editor's selection), the view the keys switch it to, the
    /// editor camera kept with what was picked, and the followed thing's
    /// pose handed to the player's camera - so its own update never reads a
    /// target a frame stale.
    pub fn update(&mut self, scene: &mut Scene, app: &App, world: &Followable) {
        let switch = self.update_focus(scene, app, world);
        if let Some(camera) = Self::player_camera_mut(scene) {
            match switch {
                Some(CameraSwitch::To(state)) => camera.set_state(state),
                Some(CameraSwitch::Next) => camera.next_camera(),
                None => {}
            }
        }
        self.update_editor_follow(scene, app, world);
        self.follow(scene, app, world.missiles);
    }

    /// Which node the camera follows, and the view to switch it to - F1:
    /// the player's aircraft, in the cockpit - pressed again there, its next
    /// view (see play::camera::camera::CameraState); F2: the next of the
    /// other aircraft, round and round, in the free view; F3/F4: the next
    /// ground unit / launched missile (there are no ground units yet); or an
    /// aircraft picked in the flight editor's hierarchy. Back to the player
    /// when the one it followed is gone.
    fn update_focus(&mut self, scene: &Scene, app: &App, world: &Followable) -> Option<CameraSwitch> {
        let mut switch = None;
        let selection = world.editor_selection.map(str::to_owned);
        if selection != self.editor_selection {
            if let Some(name) = &selection {
                self.focus = name.clone();
            }
            self.editor_selection = selection;
        }

        if app.input.is_action_just_pressed("camera_player") {
            if self.focus == world.player_id {
                switch = Some(CameraSwitch::Next);
            } else {
                self.focus = world.player_id.to_owned();
                switch = Some(CameraSwitch::To(CameraState::Cockpit));
            }
        }
        if app.input.is_action_just_pressed("camera_next_aircraft") {
            // Every aircraft but the player's, in the map's order - the one
            // after whichever it's on now.
            let others: Vec<&str> = world.aircraft.iter().copied()
                .filter(|name| *name != world.player_id && crate::game::ext::drawn::exists(scene, name))
                .collect();
            let current = others.iter().position(|name| *name == self.focus);
            match others.get(current.map(|index| index + 1).unwrap_or(0) % others.len().max(1)) {
                Some(name) => {
                    self.focus = (*name).to_owned();
                    switch = Some(CameraSwitch::To(CameraState::Free));
                }
                None => println!("Camera: no other aircraft to follow"),
            }
        }
        if app.input.is_action_just_pressed("camera_next_ground_unit") {
            println!("Camera: no ground units to follow yet");
        }
        if app.input.is_action_just_pressed("camera_next_missile") {
            let missiles: Vec<&str> = world.missiles.node_ids().collect();
            let current = missiles.iter().position(|id| *id == self.focus);
            match missiles.get(current.map(|index| index + 1).unwrap_or(0) % missiles.len().max(1)) {
                Some(id) => self.focus = (*id).to_owned(),
                None => println!("Camera: no missiles in the air"),
            }
        }

        if !crate::game::ext::drawn::exists(scene, &self.focus) {
            self.focus = world.player_id.to_owned();
        }
        switch
    }

    /// F10's editor: F1-F4 fly the editor camera to what they picked (it
    /// keeps looking the way it was), and from then on it keeps up with it
    /// as it moves - flying it about with the mouse moves round it. Until
    /// something else is picked (or focused from the hierarchy), or what it
    /// follows is gone.
    fn update_editor_follow(&mut self, scene: &mut Scene, app: &App, world: &Followable) {
        if !world.editor_active {
            self.editor_follow = None;
            return;
        }
        let picked = ["camera_player", "camera_next_aircraft", "camera_next_ground_unit", "camera_next_missile"]
            .iter()
            .any(|action| app.input.is_action_just_pressed(action)) && !world.editor_typing;
        let focus = self.focus.clone();
        let at = scene.world_transform(&focus).map(|transform| transform.position);
        let Some(camera) = Self::player_camera_mut(scene) else { return };
        if picked {
            if let Some(at) = at {
                let distance = if world.aircraft.contains(&focus.as_str()) { EDITOR_FOCUS_AIRCRAFT } else { EDITOR_FOCUS_MISSILE };
                camera.editor_focus(at, distance);
                self.editor_follow = Some((focus, at));
            }
            return;
        }
        match (&self.editor_follow, at) {
            (Some((id, last)), Some(at)) if *id == focus => {
                camera.editor_shift(at - last);
                self.editor_follow = Some((id.clone(), at));
            }
            // Gone (a missile that blew up) or something else picked - it
            // stays where it is.
            (Some(_), _) => self.editor_follow = None,
            (None, _) => {}
        }
    }

    /// Hands the player's camera where the followed node is: a missile along
    /// its flight, a wreck in the sea where it went in (on the surface - not
    /// following it down as it sinks), anything else as it's drawn - with
    /// its named view points and, for an aircraft, the G its pilot's head
    /// feels.
    fn follow(&mut self, scene: &mut Scene, app: &App, missiles: &Missiles) {
        let focus = self.focus.clone();
        let Some(followed) = scene.get_node(&focus).and_then(|node| node.transform_3d()).copied() else { return };
        let (position, rotation) = missiles.camera_pose(&focus).unwrap_or((followed.position, followed.rotation));
        let in_water = scene.content.nodes.get(&focus).and_then(|node| node.get_behavior::<Plane>()).is_some_and(Plane::in_water);
        let (position, rotation) = if in_water {
            let sea_level = app.particles.sea_level().unwrap_or(0.0);
            *self.went_in_at.entry(focus.clone()).or_insert_with(|| {
                let forward = rotation * Vector3::z();
                let heading = Vector3::new(forward.x, 0.0, forward.z).try_normalize(1e-4).unwrap_or_else(Vector3::z);
                (Vector3::new(position.x, sea_level, position.z), UnitQuaternion::face_towards(&heading, &Vector3::y()))
            })
        } else {
            self.went_in_at.remove(&focus);
            (position, rotation)
        };
        let named_cameras = scene.get_node(&focus).and_then(|node| node.get_property::<Cameras>()).cloned();
        let head_input = scene.content.nodes.get(&focus)
            .and_then(|node| node.physics_state::<AircraftState>())
            .map(|state| HeadInput {
                lateral: state.flight_data.lateral_g,
                forward: state.flight_data.longitudinal_g,
                vertical_g: state.flight_data.body_g.y,
            });
        if let Some(camera) = Self::player_camera_mut(scene) {
            camera.set_target(position, rotation, named_cameras);
            if let Some(head_input) = head_input {
                camera.set_head_input(head_input);
            }
        }
    }

    /// The animation camera where a running animation's keys put it.
    pub fn set_animation_pose(scene: &mut Scene, pose: &CameraPose) {
        if let Some(node) = scene.content.nodes.get_mut(ANIMATION_CAMERA) {
            node.set_property(Transform3D { position: pose.position, rotation: pose.rotation, scale: Vector3::new(1.0, 1.0, 1.0) });
            node.set_property(CameraLens::new(pose.fovy));
        }
    }

    /// Puts the right camera on screen:
    /// - in M's map, the map's view (the player's camera, in its map mode);
    /// - in F10's editor, the editor camera - or a camera it's looking
    ///   through ("View through", or the animation's camera with "Camera in
    ///   view");
    /// - otherwise the animation camera while a running animation's camera
    ///   keys drive it;
    /// - the player's view the rest of the time, cut to as soon as a
    ///   cinematic camera lets go of the screen.
    pub fn update_view(&mut self, scene: &mut Scene, request: ViewRequest) {
        let wanted = if request.tactical_map {
            PLAYER_CAMERA.to_owned()
        } else if request.editor {
            request.preview_camera
                .or_else(|| (request.timeline_preview && request.animating).then(|| ANIMATION_CAMERA.to_owned()))
                .unwrap_or_else(|| PLAYER_CAMERA.to_owned())
        } else if request.animating {
            ANIMATION_CAMERA.to_owned()
        } else {
            PLAYER_CAMERA.to_owned()
        };
        let wanted = if scene.cameras.get(&wanted).is_some() { wanted } else { PLAYER_CAMERA.to_owned() };

        // Every camera drawn the whole screen's size (the player's sizes
        // itself).
        for node in scene.content.nodes.iter_mut() {
            if node.id == PLAYER_CAMERA {
                continue;
            }
            if let Some(lens) = node.get_property_mut::<CameraLens>() {
                lens.viewport = None;
            }
        }

        // Already on it - unless the camera on screen went away (a camera
        // node despawned with its parent and spawned again).
        if wanted == self.on_screen && scene.cameras.has_active_camera() {
            return;
        }
        scene.select_camera(&wanted);
        self.on_screen = wanted;
    }
}

/// The rotation looking along `forward` with `up` as near up as it gets -
/// a camera's pose as a node rotation.
pub fn facing_with_up(forward: Vector3<f32>, up: Vector3<f32>) -> UnitQuaternion<f32> {
    match (forward.try_normalize(1e-6), up.try_normalize(1e-6)) {
        (Some(forward), Some(up)) if forward.cross(&up).norm() > 1e-4 => UnitQuaternion::face_towards(&forward, &up),
        (Some(forward), _) => pankarta_software::scene_manager::scene_nodes::facing(forward).unwrap_or_else(UnitQuaternion::identity),
        _ => UnitQuaternion::identity(),
    }
}

/// The fellow aviator's view points - what the player's camera can be put
/// on while it follows it.
pub fn fellow_aviator_cameras() -> Cameras {
    let mut cameras: Cameras = HashMap::new();
    cameras.insert("cockpit".to_owned(), GameObjectCamera { position: Vector3::new(0.0, 2.3, 14.3), fov: 60.0, head_limits: None });
    cameras.insert("front".to_owned(), GameObjectCamera { position: Vector3::new(0.0, 6.0, 26.0), fov: 60.0, head_limits: None });
    cameras
}
