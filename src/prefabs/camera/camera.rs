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

use crate::prefabs::aircraft::aircraft::Aircraft;
use crate::prefabs::camera::camera_view::{CameraKind, CameraView};

// What one count of mouse movement turns the orbit, in radians
const RADIANS_PER_COUNT: f32 = 0.007;
const MAX_PITCH: f32 = 80.0 * std::f32::consts::PI / 180.0;
const ZOOM_STEP: f32 = 0.03; // share of the field of view one wheel notch narrows or widens
const FOV_RANGE: (f32, f32) = (10.0, 100.0); // degrees
const ZOOM_SMOOTHING: f32 = 0.12; // seconds the view takes to settle on a new zoom
const MOUSE_SMOOTHING: f32 = 0.015; // seconds the mouse's uneven counts are smoothed over

pub struct Camera {
    node_to_follow: Option<String>, // The node it follows, either a player or other
    kind: CameraKind,
    index: usize,
    yaw: f32,              // third person orbit in the world, radians
    pitch: f32,
    placed: bool,          // the third person orbit starts behind the target, then stays put in the world
    fov: Option<(f32, f32)>, // (now, where the wheel is taking it); None until the view sets it
    mouse: Vector2<f32>,
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
        }
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
            self.yaw -= turn.x;
            self.pitch = (self.pitch - turn.y).clamp(-MAX_PITCH, MAX_PITCH);
        }

        let Some(target) = self.node_to_follow.as_deref().and_then(|id| scene.get_node(id)) else {
            return;
        };

        let count = target
            .get_behavior::<Aircraft>()
            .map_or(0, |aircraft| aircraft.cameras.of(self.kind).len());
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
