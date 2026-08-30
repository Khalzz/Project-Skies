use nalgebra::{Quaternion, Vector3};

use crate::app::App;
use crate::engine::rendering::enviroment::environment::Environment;
use crate::engine::scene_manager::node::Node;
use crate::engine::scene_manager::properties::{Model, Transform3D};
use crate::engine::scene_manager::scene::{FrameContext, Scene, SceneBehaviour};
use crate::game::camera::camera::{Camera, CameraConfig, Fov, Speed};
use crate::resources;
use crate::transform::Transform;

pub struct GameLogic;

impl GameLogic {
    pub fn new(scene: &mut Scene, app: &mut App, environment: Environment) -> Self {
        resources::apply_environment(scene, app, environment);

        scene.cameras.create_camera("sandbox_camera", Transform { position: Vector3::new(0.0, 0.0, 0.0), rotation: Quaternion::identity(), scale: Vector3::new(1.0, 1.0, 1.0) }, 80.0);

        scene.spawn_node(app,
            Node::new("f16")
                .add_property(Transform3D { position: Vector3::new(0.0, 0.0, 0.0), ..Default::default() })
                .add_property(Model { model_ref: "F16".to_owned() })
        ).expect("sandbox should only spawn 'f16' once");

        Self
    }
}

impl SceneBehaviour for GameLogic {
    fn update(&mut self, scene: &mut Scene, app: &mut App, ctx: &mut FrameContext) {
        let _ = (scene, app, ctx);
    }
}
