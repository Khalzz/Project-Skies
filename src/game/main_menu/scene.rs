use nalgebra::{UnitQuaternion, Vector3};

use crate::app::App;
use crate::engine::game_nodes::game_object::Lighting;
use crate::engine::rendering::enviroment::environment::Environment;
use crate::engine::scene_manager::node::Node;
use crate::engine::scene_manager::properties::{Model, Transform3D};
use crate::engine::rendering::camera::camera::yaw_pitch_rotation;
use crate::engine::scene_manager::scene::{FrameContext, Scene};
use crate::game::camera::camera::{Camera, CameraConfig, Fov, Speed};
use crate::resources;
use crate::transform::Transform;

use super::rebind_modal;
use super::ui;

pub struct GameLogic;

impl GameLogic {
    // this is called once, when the scene becomes active. Synchronous - no
    // level to background-load any more (see main.rs's own comment on why
    // this scene moved off create_loaded_scene).
    pub fn new(app: &mut App, environment: Environment) -> Self {
        resources::apply_environment(app, environment);

        app.window_manager.context.mouse().set_relative_mouse_mode(false);

        ui::build(app);

        Self::spawn_world(app);

        app.spawn_node(
            Node::new("main_menu_camera")
                .add_behavior(Camera::new(CameraConfig {
                    camera_name: "main_menu".to_owned(),
                    transform: Transform::new(Vector3::new(2.45, 24.58, -6.39), yaw_pitch_rotation(93.0, -10.5), Vector3::new(1.0, 1.0, 1.0)),
                    speed: Speed { base_speed: 0.0, sprint_multiplier: 0.0 }, // unused - free: false
                    fov: Fov { max: 70.0, min: 70.0, speed: 0.0 }, // .max is the actual starting fovy; min/speed unused - free: false
                    look_sensitivity_scale: 0.0, // unused - free: false
                    max_pitch_deg: 0.0, // unused - free: false
                    free: false,
                }))
        ).expect("main_menu should only spawn 'main_menu_camera' once");

        Self
    }

    fn spawn_world(app: &mut App) {
        app.spawn_node(
            Node::new("sun")
                .add_property(Transform3D {
                    position: Vector3::new(1000.0, 1_000_000.0, 1000.0),
                    rotation: UnitQuaternion::identity(),
                    scale: Vector3::new(1.0, 1.0, 1.0),
                })
                .add_property(Model { model_ref: "F16".to_owned() })
        ).expect("main_menu should only spawn 'sun' once");
        if let Some(sun) = app.renderizable_instances.get_mut("sun") {
            sun.instance.metadata.lighting = Some(Lighting { intensity: 1.0, color: Vector3::new(0.7, 0.7, 0.8) });
        }

        app.spawn_node(
            Node::new("plane")
                .add_property(Transform3D {
                    position: Vector3::new(-5.0, 20.0, 25.0),
                    rotation: UnitQuaternion::from_euler_angles(0.0f32.to_radians(), 180.0f32.to_radians(), (-45.0f32).to_radians()),
                    scale: Vector3::new(14.0, 14.0, 14.0),
                })
                .add_property(Model { model_ref: "F16".to_owned() })
        ).expect("main_menu should only spawn 'plane' once");

        app.spawn_node(
            Node::new("world")
                .add_property(Transform3D {
                    position: Vector3::new(0.0, 0.0, 0.0),
                    rotation: UnitQuaternion::identity(),
                    scale: Vector3::new(100_000.0, 1.0, 100_000.0),
                })
                .add_property(Model { model_ref: "Water".to_owned() })
        ).expect("main_menu should only spawn 'world' once");
    }

    pub fn update(&mut self, app: &mut App, delta_time: f32) {
        let _ = delta_time;
        rebind_modal::update(app);

        app.ui.has_changed = true;
    }
}

impl Scene for GameLogic {
    fn update(&mut self, app: &mut App, ctx: &mut FrameContext) {
        let _ = ctx;
        self.update(app, app.time.delta_time);
    }
}


