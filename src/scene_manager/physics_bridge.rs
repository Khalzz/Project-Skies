use std::collections::HashMap;

use nalgebra::Vector3;

use crate::game_nodes::game_object::{Collider, ColliderShape, Physics};
use crate::physics::physics_resources::PhysicsObjectDef;
use crate::resources;

use super::node::properties::Model;
use super::scene::Scene;

/// Bridges a spawned `Node`'s `Transform3D`+`Physics` properties into a
/// `PhysicsObjectDef` - what its body is made from (see that type) - on
/// `SceneContent::physics_bodies`, until the scene's physics starts or, once
/// it has, the node's spawn hands it on (see `Scene::spawn_one`). Mirrors
/// `render_bridge::register_static_model`'s role for rendering - same
/// "extract once at spawn time" shape, just building a `PhysicsObjectDef`
/// instead of a `GameObject`.
///
/// Also where `ColliderShape::TrimeshFromModel` gets resolved into a real
/// `ColliderShape::Trimesh` (see that variant's own doc comment) - reading
/// the model file and scaling its geometry by this node's own
/// `Transform3D::scale` both happen here, at spawn time, so the body is
/// built from geometry already in hand.
///
/// `trimeshes` is the mesh geometry the scene's loading step already read
/// (by model path) - a path missing from it is read now, on the spot.
///
/// Unlike `register_static_model`, this never needs to run again for the
/// same id later - a body's starting position/mass/colliders are only read
/// once, when it's made; from then on the body is where it is.
pub fn register_physics_body(scene: &mut Scene, id: &str, trimeshes: &TrimeshCache) -> Result<(), String> {
    let node = scene.content.nodes.get(id).ok_or_else(|| format!("no node named '{id}' to register"))?;
    let transform = *node.transform_3d().ok_or_else(|| format!("node '{id}' has no Transform3D property"))?;
    let mut physics = node.get_property::<Physics>().ok_or_else(|| format!("node '{id}' has no Physics property"))?.clone();

    // Geometry read from a model file is scaled the way the model is drawn:
    // node scale, times the node's Model's own scale when it has one.
    let geometry_scale = node.get_property::<Model>()
        .map(|model| model.render_scale(transform.scale))
        .unwrap_or(transform.scale);
    physics.colliders = physics.colliders.into_iter()
        .map(|collider| resolve_collider(collider, geometry_scale, id, trimeshes))
        .collect();

    scene.content.physics_bodies.push(PhysicsObjectDef { id: id.to_owned(), position: transform.position, rotation: transform.rotation, physics });

    Ok(())
}

/// A model's mesh as a collider sees it - its points and triangles.
pub type Trimesh = (Vec<Vector3<f32>>, Vec<[u32; 3]>);
/// Meshes already read for colliders, by model path (see
/// `ColliderShape::TrimeshFromModel`) - or why one couldn't be.
pub type TrimeshCache = HashMap<String, Result<Trimesh, String>>;

/// Every model path `physics` reads a mesh collider from - for the scene's
/// loading step to read ahead.
pub(crate) fn trimesh_paths(physics: &Physics) -> impl Iterator<Item = &str> {
    physics.colliders.iter().filter_map(|collider| match &collider.shape {
        ColliderShape::TrimeshFromModel { model_path } => Some(model_path.as_str()),
        _ => None,
    })
}

/// Reads the mesh at `model_path` (inside the assets folder) for a collider.
pub(crate) fn load_trimesh(model_path: &str) -> Result<Trimesh, String> {
    resources::load_trimesh_geometry(model_path).map_err(|error| error.to_string())
}

fn resolve_collider(mut collider: Collider, scale: Vector3<f32>, id: &str, trimeshes: &TrimeshCache) -> Collider {
    let ColliderShape::TrimeshFromModel { model_path } = &collider.shape else { return collider };

    let loaded = match trimeshes.get(model_path) {
        Some(loaded) => loaded.clone(),
        None => load_trimesh(model_path),
    };
    match loaded {
        Ok((raw_vertices, indices)) => {
            let vertices = raw_vertices.iter()
                .map(|v| Vector3::new(v.x * scale.x, v.y * scale.y, v.z * scale.z))
                .collect();
            collider.shape = ColliderShape::Trimesh { vertices, indices };
            collider
        },
        Err(error) => {
            eprintln!("node '{id}': trimesh collider from '{model_path}' couldn't be loaded: {error}");
            // Falls back to a real (if wrong) ColliderShape rather than
            // leaving the unresolved TrimeshFromModel in place - the physics
            // thread has no file access to retry this itself, so an empty
            // Trimesh (no collision at all) is the honest failure mode here,
            // not a silent do-over later.
            collider.shape = ColliderShape::Trimesh { vertices: vec![], indices: vec![] };
            collider
        }
    }
}
