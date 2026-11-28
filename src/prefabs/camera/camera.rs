use nalgebra::{UnitQuaternion, Vector2, Vector3};
use pankarta_software::prelude::{App, InputSource, Key, SceneAccess};
use pankarta_software::scene_manager::{
    behavior::Behavior,
    node::{
        properties::{CameraLens, Transform3D},
        Node,
    },
    scene::Scene,
};

use crate::game_settings::GAME_SETTINGS;
use crate::head_tracking;
use crate::prefabs::aircraft::aircraft::Aircraft;
use crate::prefabs::camera::camera_view::{CameraKind, CameraView};
use crate::prefabs::camera::head_motion::{HeadInput, HeadMotion};

// What one count of mouse movement turns the orbit, in radians
const RADIANS_PER_COUNT: f32 = 0.007;
const MAX_PITCH: f32 = 80.0 * std::f32::consts::PI / 180.0;
const ZOOM_STEP: f32 = 0.03; // share of the field of view one wheel notch narrows or widens
const FOV_RANGE: (f32, f32) = (10.0, 100.0); // degrees
const ZOOM_SMOOTHING: f32 = 0.12; // seconds the view takes to settle on a new zoom
const MOUSE_SMOOTHING: f32 = 0.015; // seconds the mouse's uneven counts are smoothed over

// In the cockpit: how far the head turns (degrees), and how it eases back while head tracking takes over
const HEAD_TURN: (f32, f32) = (170.0, 70.0);
const LOOK_CENTER_RATE: f32 = 6.0;
const VIEW_PITCH_RATE: f32 = 4.0;
const MAX_HEAD_ROLL: f32 = 45.0;
// Looking behind, the pilot twists and looks over a shoulder: at fully behind the eye moves this far (meters)
// toward the side looked at, up and forward, building with how far round (curve: 1 evenly, higher late)
const LOOK_BEHIND: (f32, f32, f32, f32) = (0.15, 0.05, 0.08, 2.0);

pub struct Camera {
    node_to_follow: Option<String>, // The node it follows, either a player or other
    kind: CameraKind,
    index: usize,
    yaw: f32,              // third person orbit in the world, radians
    pitch: f32,
    placed: bool,          // the third person orbit starts behind the target, then stays put in the world
    fov: Option<(f32, f32)>, // (now, where the wheel is taking it); None until the view sets it
    mouse: Vector2<f32>,
    head: HeadMotion,
    cockpit_look: Vector2<f32>, // the head's turn (yaw left +, pitch up +), degrees
    view_pitch: f32,            // the "cockpit view angle" setting, eased toward
}

impl Camera {
    pub fn new(player_id: &str) -> Self {
        Self {
            node_to_follow: Some(player_id.to_string()),
            kind: CameraKind::ThirdPerson,
            index: 0,
            yaw: 0.0,
            pitch: 0.0,
            placed: false,
            fov: None,
            mouse: Vector2::zeros(),
            head: HeadMotion::new(),
            cockpit_look: Vector2::zeros(),
            view_pitch: 0.0,
        }
    }

    // Whether it's the pilot's eye
    pub fn in_cockpit(&self) -> bool {
        self.kind == CameraKind::FirstPerson
    }

    // The node it's looking at
    pub fn following(&self) -> Option<&str> {
        self.node_to_follow.as_deref()
    }

    pub fn prefab(self, scene: &Scene) -> Node {
        let (transform, fov) = self
            .node_to_follow
            .as_deref()
            .and_then(|id| scene.get_node(id))
            .and_then(|target| self.pose(target))
            .unwrap_or((Transform3D::default(), 60.0));

        Node::new("Camera")
            .with_property(transform)
            .with_behavior(self)
            .with_property(CameraLens::new(fov))
    }

    fn view<'a>(&self, target: &'a Node) -> Option<(&'a Transform3D, &'a CameraView)> {
        let transform = target.get_property::<Transform3D>()?;
        let views = target.get_behavior::<Aircraft>()?.cameras.of(self.kind);
        Some((transform, views.get(self.index).or(views.first())?))
    }

    fn pose(&self, target: &Node) -> Option<(Transform3D, f32)> {
        let (transform, view) = self.view(target)?;
        let look_at = transform.position + transform.rotation * view.look_at;

        if self.kind == CameraKind::FirstPerson {
            return Some((self.cockpit(transform, view), view.fov));
        }

        let (position, up) = match self.kind {
            // Round the target in the world, by the mouse alone: the target's turns never move it
            CameraKind::ThirdPerson => {
                let orbit = UnitQuaternion::from_axis_angle(&Vector3::y_axis(), self.yaw)
                    * UnitQuaternion::from_axis_angle(&Vector3::x_axis(), self.pitch);
                (look_at + orbit * (view.offset - view.look_at), Vector3::y())
            }
            _ => (
                transform.position + transform.rotation * view.offset,
                transform.rotation * Vector3::y(),
            ),
        };
        let rotation = UnitQuaternion::face_towards(&(position - look_at), &up);

        Some((
            Transform3D {
                position,
                rotation,
                ..Default::default()
            },
            view.fov,
        ))
    }
}

impl Camera {
    /*
        The pilot's eye: the seat's resting eye, moved by the G the head feels, the pilot's look (mouse or head
        tracker) and the twist of looking over a shoulder
    */
    fn cockpit(&self, transform: &Transform3D, view: &CameraView) -> Transform3D {
        let head = self.head.pose(GAME_SETTINGS.lock().unwrap().head_g_reaction);

        // The tracker's own axes are the pilot's: right, up, toward the screen
        let tracked = head_tracking::tracked_pose();
        let tracked_offset = Vector3::new(tracked.x, tracked.y, -tracked.z.max(0.0)) * 0.01;
        let tracked_roll = tracked
            .roll
            .clamp(-MAX_HEAD_ROLL - head.roll_deg, MAX_HEAD_ROLL - head.roll_deg);
        let tracked_turn = UnitQuaternion::from_axis_angle(&Vector3::y_axis(), (-tracked.yaw).to_radians())
            * UnitQuaternion::from_axis_angle(&Vector3::z_axis(), (-tracked_roll).to_radians())
            * UnitQuaternion::from_axis_angle(&Vector3::x_axis(), tracked.pitch.to_radians());

        let ahead = UnitQuaternion::face_towards(&(view.offset - view.look_at), &Vector3::y());
        let look = UnitQuaternion::from_axis_angle(&Vector3::y_axis(), self.cockpit_look.x.to_radians())
            * UnitQuaternion::from_axis_angle(
                &Vector3::x_axis(),
                (self.cockpit_look.y + self.view_pitch).to_radians(),
            );

        // Over the shoulder, from where the view points now: 0 facing forward, 1 facing behind
        let facing = look * -Vector3::z();
        let turned = (-facing.x).atan2(-facing.z);
        let (side, up, forward, curve) = LOOK_BEHIND;
        let behind = (turned.abs() / std::f32::consts::PI).powf(curve);
        let shoulder = Vector3::new(-side * turned.signum(), up, -forward) * behind;

        let eye = view.offset + shoulder + head.offset + tracked_offset;
        Transform3D {
            position: transform.position + transform.rotation * eye,
            rotation: transform.rotation * head.tilt * tracked_turn * ahead * look,
            ..Default::default()
        }
    }
}

impl Behavior for Camera {
    fn on_spawn(&mut self, _node: &mut Node, _scene: &mut SceneAccess, app: &mut App) {
        app.window.set_cursor_locked(true);
    }

    fn update(&mut self, node: &mut Node, scene: &mut SceneAccess, app: &mut App) {
        let pressed = [
            (Key::F1, CameraKind::FirstPerson),
            (Key::F2, CameraKind::ThirdPerson),
            (Key::F3, CameraKind::Cinematic),
        ]
        .into_iter()
        .find(|(key, _)| app.input.is_just_pressed(InputSource::key(*key)))
        .map(|(_, kind)| kind);

        if let Some(kind) = pressed {
            if kind == self.kind {
                self.index += 1;
            } else {
                self.kind = kind;
                self.index = 0;
            }
            self.placed = false;
            self.yaw = 0.0;
            self.pitch = 0.0;
            self.cockpit_look = Vector2::zeros();
            self.fov = None;
        }

        // Escape frees the mouse, a click takes it back
        if app.input.is_just_pressed(InputSource::key(Key::Escape)) {
            app.window.set_cursor_locked(false);
        } else if app.input.mouse_left_button_just_pressed() {
            app.window.set_cursor_locked(true);
        }

        // Mouse right swings the view right, mouse down looks down
        if app.window.is_cursor_locked() {
            let ease = 1.0 - (-app.time.delta_time / MOUSE_SMOOTHING).exp();
            self.mouse += (app.input.mouse_look() - self.mouse) * ease;
            let turn = self.mouse * RADIANS_PER_COUNT;
            if self.kind == CameraKind::FirstPerson && !head_tracking::trackir_receiving() {
                let degrees = turn.map(f32::to_degrees);
                self.cockpit_look.x = (self.cockpit_look.x - degrees.x).clamp(-HEAD_TURN.0, HEAD_TURN.0);
                self.cockpit_look.y = (self.cockpit_look.y - degrees.y).clamp(-HEAD_TURN.1, HEAD_TURN.1);
            } else {
                self.yaw -= turn.x;
                self.pitch = (self.pitch - turn.y).clamp(-MAX_PITCH, MAX_PITCH);
            }
        }

        // While a head tracker is looking, a leftover mouse look eases back to center
        let dt = app.time.delta_time;
        if head_tracking::trackir_receiving() {
            self.cockpit_look *= (-LOOK_CENTER_RATE * dt).exp();
        }
        let view_pitch = GAME_SETTINGS.lock().unwrap().cockpit_view_pitch_deg;
        self.view_pitch += (view_pitch - self.view_pitch) * (1.0 - (-VIEW_PITCH_RATE * dt).exp());

        let Some(target) = self.node_to_follow.as_deref().and_then(|id| scene.get_node(id)) else {
            return;
        };

        let count = target
            .get_behavior::<Aircraft>()
            .map_or(0, |aircraft| aircraft.cameras.of(self.kind).len());

        // The head feels what the aircraft does, whichever view is on, so the cockpit picks it up mid-throw
        if let Some(sensors) = target.get_behavior::<Aircraft>().map(|aircraft| aircraft.sensors) {
            let input = HeadInput {
                lateral: sensors.lateral_g,
                forward: sensors.longitudinal_g,
                vertical_g: sensors.load_factor,
            };
            self.head.update(input, dt);
        }
        if self.index >= count {
            self.index = 0;
        }

        // Entering third person, the orbit starts behind where the target is heading
        if self.kind == CameraKind::ThirdPerson && !self.placed {
            if let Some(forward) = target
                .get_property::<Transform3D>()
                .map(|transform| transform.rotation * -Vector3::z())
            {
                self.yaw = (-forward.x).atan2(-forward.z);
                self.pitch = 0.0;
                self.placed = true;
            }
        }

        let Some((pose, view_fov)) = self.pose(target) else {
            return;
        };

        let (now, wanted) = self.fov.get_or_insert((view_fov, view_fov));
        let scroll = app.input.mouse_scroll_y();
        *wanted = (*wanted * (1.0 - scroll * ZOOM_STEP)).clamp(FOV_RANGE.0, FOV_RANGE.1);
        *now += (*wanted - *now) * (1.0 - (-app.time.delta_time / ZOOM_SMOOTHING).exp());
        let fov = *now;

        if let Some(transform) = node.get_property_mut::<Transform3D>() {
            transform.position = pose.position;
            transform.rotation = pose.rotation;
        }
        if let Some(lens) = node.get_property_mut::<CameraLens>() {
            lens.fovy = fov;
        }
    }
}
