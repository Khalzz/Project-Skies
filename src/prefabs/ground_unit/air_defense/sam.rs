use nalgebra::Vector3;
use pankarta_software::prelude::*;

use super::super::utils;

#[derive(Clone, Debug)]
pub struct Sam {
    pub integrated_radar: bool,
    pub detection_range: f32,
}

impl Sam {
    pub fn new(integrated_radar: bool, detection_range: f32) -> Self {
        Sam {
            integrated_radar,
            detection_range,
        }
    }

    pub fn prefab(self, node: Node) -> Node {
        let node = node
            .with_property(Physics {
                rigidbody: RigidBodyData {
                    center_of_mass: Vector3::new(0.0, 1.0, 0.0),
                    ..RigidBodyData::dynamic(8_000.0)
                },
                colliders: vec![
                    Collider::new(ColliderShape::Cuboid {
                        half_extents: (1.2, 0.5, 3.6),
                    })
                    .offset(Vector3::new(0.0, 0.5, 0.0))
                    .friction(0.8),
                    Collider::new(ColliderShape::Cuboid {
                        half_extents: (1.2, 1.6, 4.05),
                    })
                    .offset(Vector3::new(0.0, 2.4, 0.1))
                    .friction(0.8),
                ],
            })
            .with_property(model());

        let node = if self.integrated_radar {
            node.with_child(utils::sensor(self.detection_range))
        } else {
            node
        };
        node.with_behavior(self)
    }
}

fn model() -> Model {
    let dark = Material::standard().color(UiColor::Rgb(30, 30, 30));
    let glass = Material::standard().color(UiColor::Rgb(40, 60, 80));
    let mut model = Model::new()
        .material(Material::standard().color(UiColor::Rgb(90, 105, 70))) // default for every part
        .part(
            "chassis",
            Mesh::cube()
                .transform(Transform3D {
                    position: Vector3::new(0.0, 1.0, 0.0),
                    scale: Vector3::new(2.2, 0.4, 8.0),
                    ..Default::default()
                })
                .material(dark.clone()),
        )
        .part(
            "cab",
            Mesh::cube().transform(Transform3D {
                position: Vector3::new(0.0, 2.2, 2.9),
                scale: Vector3::new(2.4, 2.0, 2.0),
                ..Default::default()
            }),
        )
        .part(
            "windshield",
            Mesh::cube()
                .transform(Transform3D {
                    position: Vector3::new(0.0, 2.6, 3.91),
                    scale: Vector3::new(2.0, 0.8, 0.05),
                    ..Default::default()
                })
                .material(glass),
        )
        .part(
            "bumper",
            Mesh::cube()
                .transform(Transform3D {
                    position: Vector3::new(0.0, 0.9, 4.05),
                    scale: Vector3::new(2.4, 0.4, 0.2),
                    ..Default::default()
                })
                .material(dark.clone()),
        )
        .part(
            "cargo",
            Mesh::cube().transform(Transform3D {
                position: Vector3::new(0.0, 2.6, -1.1),
                scale: Vector3::new(2.4, 2.8, 5.6),
                ..Default::default()
            }),
        );
    // Six wheels: one axle under the cab, two under the cargo.
    for (index, z) in [3.0, -1.6, -3.0].into_iter().enumerate() {
        for (side, x) in [("left", 1.15), ("right", -1.15)] {
            model = model.part(
                format!("wheel_{side}_{index}"),
                utils::wheel(Vector3::new(x, 0.5, z)).material(dark.clone()),
            );
        }
    }
    model
}

impl Behavior for Sam {
    fn update(&mut self, _node: &mut Node, _scene: &mut SceneAccess, _app: &mut App) {}
}
