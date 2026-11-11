use nalgebra::Vector3;

use pankarta_software::prelude::*;


pub struct GameLogic;

impl GameLogic {
    pub fn new(scene: &mut Scene, app: &mut App, environment: Environment) -> Self {
        app.renderer.set_environment(environment);

        let camera = Node::new("sandbox_camera")
            .with_property(Transform3D::default())
            .with_property(CameraLens::new(80.0));
        let f16 = Node::new("f16")
            .with_property(Transform3D { position: Vector3::new(0.0, 0.0, 0.0), ..Default::default() })
            .with_property(Model::file("planes/f16/model/f16.glb").double_sided(DoubleSidedMeshes::Meshes(vec!["Afterburner".to_owned()])));
        for node in [camera, f16] {
            if let Err(error) = scene.spawn_node(app, node) {
                eprintln!("{error}");
            }
        }

        Self
    }
}

impl SceneBehaviour for GameLogic {}
