use nalgebra::{Vector3};
use pankarta_software::game_nodes::game_object::{Collider, ColliderShape, Physics, RigidBodyData};
use pankarta_software::prelude::*;
use pankarta_software::rendering::ui::ui::Ui;
use pankarta_software::ui::layer::Layer;
use pankarta_software::ui::ui_transform::SizeValue;
use pankarta_software::ui::widgets::text;
use pankarta_software::ui::TextStyle;

use crate::player::Player;

pub struct WalkingScene;

const GROUND_TEXTURE: &str = "textures/grid/Dark/texture_01.png";
const CUBE_TEXTURES: [&str; 4] = [
    "textures/grid/Orange/texture_01.png",
    "textures/grid/Green/texture_01.png",
    "textures/grid/Purple/texture_01.png",
    "textures/grid/Red/texture_01.png",
]; 

const PLAYER_HALF_HEIGHT: f32 = 0.9;

fn static_box(size: Vector3<f32>) -> Physics {
    Physics {
        rigidbody: RigidBodyData::fixed(),
        colliders: vec![Collider::new(ColliderShape::Cuboid { half_extents: (size.x / 2.0, size.y / 2.0, size.z / 2.0) })],
    }
}

impl WalkingScene {
    pub fn new(scene: &mut Scene, app: &mut App) -> Self {
        app.renderer.set_environment(Environment::ProceduralSky { sun: true });

        let ground = Node::new("ground")
            .with_property(Transform3D::default())
            .with_property(Model::new(ModelSource::Plane { subdivisions: 0 }).with_scale(Vector3::new(200.0, 1.0, 200.0)))
            .with_property(Material::standard().texture(GROUND_TEXTURE).tiled(4.0, TextureSpace::World))
            .with_property(Physics {
                rigidbody: RigidBodyData { is_static: true, mass: 0.0, ..Default::default() },
                colliders:  vec![Collider::new(ColliderShape::HalfSpace { normal: Vector3::y() })],
            });


        let plane = Node::new("plane")
            .with_property(Transform3D {
                position: Vector3::new(0.0, 2.0, 0.0),
                scale: Vector3::new(14.0, 14.0, 14.0),
                ..Default::default()
            })
            .with_property(Physics {
                rigidbody: RigidBodyData { is_static: true, mass: 0.0, ..Default::default() },
                colliders:  vec![Collider::new(ColliderShape::TrimeshFromModel { model_path: "models/f16.glb".to_owned() } )],
            })
            .with_property(Model::new(ModelSource::File("models/f16.glb".to_owned())));

        let player = Node::new("player").with_behavior(Player::new(Vector3::new(0.0, PLAYER_HALF_HEIGHT + 0.5, -10.0)));

        let sun = Node::new("sun")
            .with_property(Transform3D::facing(Vector3::new(-0.3, -1.0, -0.2)))
            .with_property(Light::directional([1.0, 1.0, 1.0]));

        let mut nodes = vec![ground, player, plane, sun];

        for index in 0..12 {
            let angle = index as f32 / 12.0 * std::f32::consts::TAU;
            let height = 0.5 + (index % 4) as f32 * 0.75;
            let size = Vector3::new(2.0, height, 2.0);
            nodes.push(Node::new(format!("cube_{index}"))
                .with_property(Transform3D { position: Vector3::new(angle.sin() * 12.0, height / 2.0, angle.cos() * 12.0), ..Default::default() })
                .with_property(Model::new(ModelSource::Cube).with_scale(size))
                .with_property(Material::standard().texture(CUBE_TEXTURES[index % 4]).tiled(1.0, TextureSpace::Object))
                .with_property(static_box(size)));
        }
        for node in nodes {
            if let Err(error) = scene.spawn_node(app, node) {
                eprintln!("{error}");
            }
        }

        // The frame rate, top left.
        let framerate = text(app, "- fps", &TextStyle::default())
            .set_padding((10.0, 10.0))
            .set_size(SizeValue::Pixels(200.0), SizeValue::Fit);
        Layer::new(app).set_child("framerate", framerate).build(app);

        app.window.set_cursor_locked(true);
        Self
    }
}

impl SceneBehaviour for WalkingScene {
    fn update(&mut self, _scene: &mut Scene, app: &mut App, _ctx: &mut FrameContext) {
        if let Some(label) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "framerate").and_then(|node| node.as_label_mut()) {
            let fps = app.time.fps().round();
            label.set_text(&mut app.ui.text.font_system, &format!("{fps} fps"), true);
        }
    }
}