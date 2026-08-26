use nalgebra::{Quaternion, Vector3};

use crate::app::App;
use crate::engine::rendering::enviroment::environment::Environment;
use crate::engine::scene_manager::node::Node;
use crate::engine::scene_manager::properties::{Model, Transform3D};
use crate::engine::scene_manager::scene::{FrameContext, Scene};
use crate::game::camera::camera::{Camera, CameraConfig, Fov, Speed};
use crate::resources;
use crate::transform::Transform;

pub struct GameLogic;

impl GameLogic {
    pub fn new(app: &mut App, environment: Environment) -> Self {
        resources::apply_environment(app, environment);

        app.spawn_node(
            Node::new("sandbox_camera")
                .add_behavior(Camera::new(CameraConfig {
                    camera_name: "sandbox_free".to_owned(),
                    transform: Transform::new(Vector3::new(0.0, 0.0, 0.0), Quaternion::identity(), Vector3::new(1.0, 1.0, 1.0)),
                    speed: Speed { base_speed: 10.0, sprint_multiplier: 5.0 },
                    fov: Fov { max: 120.0, min: 10.0, speed: 5.0 },
                    look_sensitivity_scale: 0.5,
                    max_pitch_deg: 89.0,
                    free: true,
                }))
        ).expect("sandbox should only spawn 'sandbox_camera' once");

        app.spawn_node(
            Node::new("f16")
                .add_property(Transform3D { position: Vector3::new(0.0, 0.0, 0.0), ..Default::default() })
                .add_property(Model { model_ref: "F16".to_owned() })
        ).expect("sandbox should only spawn 'f16' once");

        Self
    }
}

impl Scene for GameLogic {
    fn update(&mut self, app: &mut App, ctx: &mut FrameContext) {
        let _ = (app, ctx);
    }
}
