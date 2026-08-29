use nalgebra::{Point3, UnitQuaternion, Vector3};

use crate::app::App;
use crate::engine::game_nodes::game_object::Lighting;
use crate::engine::input::input;
use crate::engine::rendering::camera::handler::LookAtTarget;
use crate::engine::rendering::enviroment::environment::Environment;
use crate::engine::scene_manager::node::Node;
use crate::engine::scene_manager::properties::{Model, Transform3D};
use crate::engine::rendering::camera::camera::yaw_pitch_rotation;
use crate::engine::scene_manager::scene::{FrameContext, Scene, SceneBehaviour};
#[allow(unused_imports)]
use crate::game::camera::camera::{Camera, CameraConfig, Fov, Speed};
use crate::resources;
use crate::transform::Transform;

use super::rebind_modal;
use super::ui;

// Resting pose the camera drifts around (see GameLogic::drift_camera) - the
// same values the old, pre-node-system main menu camera used.
const BASE_POSITION: Vector3<f32> = Vector3::new(2.45, 24.58, -6.39);
const BASE_YAW_DEG: f32 = 93.0;
const BASE_PITCH_DEG: f32 = -10.5;

pub struct GameLogic {
    // Seconds of drift_camera runtime so far - drives the sin/cos wobble
    // below, not real elapsed scene time (same idea as its own delta_time
    // accumulation).
    elapsed: f32,
}

impl GameLogic {
    // this is called once, when the scene becomes active. Synchronous - no
    // level to background-load any more (see main.rs's own comment on why
    // this scene moved off create_loaded_scene).
    pub fn new(scene: &mut Scene, app: &mut App, environment: Environment) -> Self {
        resources::apply_environment(scene, app, environment);

        app.window_manager.context.mouse().set_relative_mouse_mode(false);

        ui::build(app);

        Self::spawn_world(scene, app);

        let camera = scene.create_camera("main_menu", Transform::new(BASE_POSITION, yaw_pitch_rotation(BASE_YAW_DEG, BASE_PITCH_DEG), Vector3::new(1.0, 1.0, 1.0)), 70.0);
        // camera.look_at = Some(LookAtTarget::Node("plane".to_owned()));
        scene.select_camera("main_menu");

        Self { elapsed: 0.0 }
    }

    fn spawn_world(scene: &mut Scene, app: &mut App) {
        scene.spawn_node(app,
            Node::new("sun")
                .add_property(Transform3D {
                    position: Vector3::new(1000.0, 1_000_000.0, 1000.0),
                    rotation: UnitQuaternion::identity(),
                    scale: Vector3::new(1.0, 1.0, 1.0),
                })
                .add_property(Model { model_ref: "F16".to_owned() })
        ).expect("main_menu should only spawn 'sun' once");
        if let Some(sun) = scene.content.renderizable_instances.get_mut("sun") {
            sun.instance.metadata.lighting = Some(Lighting { intensity: 1.0, color: Vector3::new(0.7, 0.7, 0.8) });
        }

        scene.spawn_node(app,
            Node::new("plane")
                .add_property(Transform3D {
                    position: Vector3::new(-5.0, 20.0, 25.0),
                    rotation: UnitQuaternion::from_euler_angles(0.0f32.to_radians(), 180.0f32.to_radians(), (-45.0f32).to_radians()),
                    scale: Vector3::new(14.0, 14.0, 14.0),
                })
                .add_property(Model { model_ref: "F16".to_owned() })
        ).expect("main_menu should only spawn 'plane' once");

        scene.spawn_node(app,
            Node::new("world")
                .add_property(Transform3D {
                    position: Vector3::new(0.0, 0.0, 0.0),
                    rotation: UnitQuaternion::identity(),
                    scale: Vector3::new(100_000.0, 1.0, 100_000.0),
                })
                .add_property(Model { model_ref: "Water".to_owned() })
        ).expect("main_menu should only spawn 'world' once");
    }

    pub fn update(&mut self, scene: &mut Scene, app: &mut App, delta_time: f32) {
        self.drift_camera(scene, delta_time);
        rebind_modal::update(app);

        app.ui.has_changed = true;
    }

    fn drift_camera(&mut self, scene: &mut Scene, delta_time: f32) {
        self.elapsed += delta_time;
        let t = self.elapsed;

        let offset = Vector3::new(
            (t * 0.6).sin() * 0.4,
            (t * 0.9).sin() * 0.25,
            (t * 0.45).cos() * 0.35,
        );
        let yaw_drift = (t * 0.35).sin() * 1.5;
        let pitch_drift = (t * 0.5).sin() * 0.8;

        if let Some(camera) = scene.cameras.get_mut("main_menu") {
            camera.camera.set_position(Point3::from(BASE_POSITION + offset));
            camera.camera.set_yaw_pitch((BASE_YAW_DEG + yaw_drift).to_radians(), (BASE_PITCH_DEG + pitch_drift).to_radians());
        }
    }
}

impl SceneBehaviour for GameLogic {
    fn update(&mut self, scene: &mut Scene, app: &mut App, ctx: &mut FrameContext) {
        let _ = ctx;
        self.update(scene, app, app.time.delta_time);
    }
}
