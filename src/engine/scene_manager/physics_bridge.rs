use nalgebra::Vector3;

use crate::engine::game_nodes::game_object::{ColliderType, Physics};
use crate::engine::physics::physics_resources::PhysicsObjectDef;
use crate::resources;

use super::properties::Transform3D;
use super::scene::Scene;

/// Bridges a spawned `Node`'s `Transform3D`+`Physics` properties into
/// `SceneContent::physics_bodies` - the plain, `Send`-safe list
/// `SceneBehaviour::fixed_update` hands to the physics thread (see
/// `PhysicsObjectDef`'s own doc comment for why that's a copy, not a live
/// reference back to the node - `Node`'s `Box<dyn Any>` properties can't
/// cross the thread boundary). Mirrors `render_bridge::register_static_model`'s
/// role for rendering - same "extract once at spawn time" shape, just
/// building a `PhysicsObjectDef` instead of a `GameObject`.
///
/// Also where `ColliderType::TrimeshFromModel` gets resolved into a real
/// `ColliderType::Trimesh` (see that variant's own doc comment) - reading
/// the model file and scaling its geometry by this node's own
/// `Transform3D::scale` both need to happen exactly here, at spawn time,
/// same as `register_static_model` resolves a `Model`'s `model_ref` into
/// real instance data instead of the physics thread (which only ever sees
/// already-resolved `PhysicsObjectDef`s, no file access of its own) doing it
/// later.
///
/// Unlike `register_static_model`, this never needs to run again for the
/// same id later - a physics body's starting position/mass/colliders are
/// only ever read once, when the physics thread actually starts (see
/// `App::run`'s `run_fixed_update` call), not kept in sync afterward the way
/// a model's shared instance buffer is.
pub fn register_physics_body(scene: &mut Scene, id: &str) -> Result<(), String> {
    let node = scene.content.nodes.get(id).ok_or_else(|| format!("no node named '{id}' to register"))?;
    let transform = *node.get_property::<Transform3D>().ok_or_else(|| format!("node '{id}' has no Transform3D property"))?;
    let mut physics = node.get_property::<Physics>().ok_or_else(|| format!("node '{id}' has no Physics property"))?.clone();

    physics.colliders = physics.colliders.into_iter()
        .map(|collider| resolve_collider(collider, transform.scale, id))
        .collect();

    scene.content.physics_bodies.push(PhysicsObjectDef { id: id.to_owned(), position: transform.position, physics });

    Ok(())
}

fn resolve_collider(collider: ColliderType, scale: Vector3<f32>, id: &str) -> ColliderType {
    let ColliderType::TrimeshFromModel { model_path } = &collider else { return collider };

    match resources::load_trimesh_geometry(model_path) {
        Ok((raw_vertices, indices)) => {
            let vertices = raw_vertices.iter()
                .map(|v| Vector3::new(v.x * scale.x, v.y * scale.y, v.z * scale.z))
                .collect();
            ColliderType::Trimesh { vertices, indices }
        },
        Err(error) => {
            eprintln!("node '{id}': trimesh collider from '{model_path}' couldn't be loaded: {error}");
            // Falls back to a real (if wrong) ColliderType rather than
            // leaving the unresolved TrimeshFromModel in place - the physics
            // thread has no file access to retry this itself, so an empty
            // Trimesh (no collision at all) is the honest failure mode here,
            // not a silent do-over later.
            ColliderType::Trimesh { vertices: vec![], indices: vec![] }
        }
    }
}
