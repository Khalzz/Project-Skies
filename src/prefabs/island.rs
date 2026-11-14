//! The runway island - the land at the map's origin, with the runway on it.
//! The same island wherever it's put up: play (solid, to land on), the
//! sandbox and the follow test (to look at).

use nalgebra::{UnitQuaternion, Vector3};

use pankarta_software::prelude::*;

/// Its model.
pub const MODEL: &str = "models/Runway/Runway.glb";
/// Where it sits, and how much bigger it's drawn than its file.
pub const POSITION: Vector3<f32> = Vector3::new(0.0, 100.0, 0.0);
pub const SCALE: f32 = 60.0;

/// Its triangles, as its file has them (not yet scaled or placed) - for its
/// collider, the missiles' ground and the tactical map's contours. Empty
/// when the file can't be read.
pub fn trimesh() -> (Vec<Vector3<f32>>, Vec<[u32; 3]>) {
    crate::ext::mesh_data::load_trimesh_geometry(MODEL).unwrap_or_else(|error| {
        eprintln!("island: the trimesh from '{MODEL}' couldn't be loaded: {error}");
        (vec![], vec![])
    })
}

/// The island's node, named `name` - solid (a fixed body, its own triangles
/// as its collider) when it's given them (`trimesh`), only drawn otherwise.
pub fn island(name: &str, trimesh: Option<(Vec<Vector3<f32>>, Vec<[u32; 3]>)>) -> Node {
    let scale = Vector3::repeat(SCALE);
    let node = Node::new(name)
        .with_property(Transform3D { position: POSITION, rotation: UnitQuaternion::identity(), scale })
        .with_property(Model::file(MODEL).double_sided(DoubleSidedMeshes::All));
    let Some((vertices, indices)) = trimesh else { return node };
    // The collider is in world units: its vertices scaled as the node is.
    node.with_property(Physics {
        rigidbody: RigidBodyData::fixed(),
        colliders: vec![Collider::new(ColliderShape::Trimesh {
            vertices: vertices.iter().map(|vertex| vertex.component_mul(&scale)).collect(),
            indices,
        })],
    })
}
