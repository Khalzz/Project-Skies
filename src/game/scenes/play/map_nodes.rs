//! The map's own nodes (`map.ron`'s `nodes` - cameras, mostly), kept in the
//! scene the way the map has them - see `MapNodes::sync`. The aircraft are
//! flight_manager's.

use nalgebra::Vector3;

use crate::app::App;
use crate::engine::scene_manager::node::Node;
use crate::engine::scene_manager::properties::{CameraLens, LookAt, Parent, Space, Transform3D};
use crate::engine::scene_manager::scene::Scene;

use super::map::{MapSpec, NodeSpec};

#[derive(Default)]
pub struct MapNodes {
    // The map's nodes as last put in the scene - what a new map is compared
    // against, so only what changed is touched.
    applied: Vec<NodeSpec>,
}

impl MapNodes {
    /// Brings the scene in line with `map`'s nodes - cheap when
    /// nothing changed, so call it every frame:
    /// - a node gone from the map is removed (its children with it);
    /// - one missing from the scene is spawned - a new one, or one that
    ///   went with its parent (an aircraft respawned) - parents before
    ///   children;
    /// - one whose spec changed gets its new place, parent, lens and aim
    ///   (an animation playing moves it again from there);
    pub fn sync(&mut self, scene: &mut Scene, app: &mut App, map: &MapSpec) {
        for before in &self.applied {
            if map.node(&before.id).is_none() {
                scene.despawn_node(app, &before.id, None);
            }
        }

        // Missing ones, parents first: each pass spawns those whose parent
        // is there now. A parent that never turns up leaves its children
        // at the top.
        let mut missing: Vec<&NodeSpec> = map.nodes.iter().filter(|spec| scene.content.nodes.get(&spec.id).is_none()).collect();
        while !missing.is_empty() {
            let (ready, waiting): (Vec<&NodeSpec>, Vec<&NodeSpec>) = missing.into_iter().partition(|spec| {
                spec.parent.as_ref().is_none_or(|parent| scene.content.nodes.get(parent).is_some())
            });
            let stuck = ready.is_empty();
            let batch = if stuck { waiting.clone() } else { ready };
            for spec in batch {
                if stuck {
                    eprintln!("map node '{}': its parent '{}' isn't in the scene - put at the top", spec.id, spec.parent.as_deref().unwrap_or(""));
                }
                if let Err(error) = scene.spawn_node(app, build_node(spec, scene)) {
                    eprintln!("map node '{}': {error}", spec.id);
                }
            }
            missing = if stuck { Vec::new() } else { waiting };
        }

        for spec in &map.nodes {
            let changed = self.applied.iter().find(|before| before.id == spec.id).is_none_or(|before| before != spec);
            if changed {
                apply_spec(scene, spec);
            }
        }
        self.applied = map.nodes.clone();

    }
}

/// `spec`'s transform as given - an offset (Local) or a place in the world.
fn given_transform(spec: &NodeSpec) -> Transform3D {
    Transform3D { position: spec.position(), rotation: spec.rotation(), scale: Vector3::new(1.0, 1.0, 1.0) }
}

/// `spec`'s parent, if it has one that's in the scene.
fn parent_property(spec: &NodeSpec, scene: &Scene) -> Option<Parent> {
    let id = spec.parent.as_ref().filter(|parent| scene.content.nodes.get(parent).is_some())?;
    let local = match spec.space {
        Space::Local => given_transform(spec),
        Space::Global => Transform3D::default(),
    };
    Some(Parent { id: id.clone(), space: spec.space, local, inherit_rotation: spec.follow_rotation })
}

fn build_node(spec: &NodeSpec, scene: &Scene) -> Node {
    let mut node = Node::new(&spec.id).add_property(given_transform(spec));
    if let Some(parent) = parent_property(spec, scene) {
        node.set_property(parent);
    }
    if let Some(camera) = spec.camera {
        node.set_property(CameraLens { fovy: camera.fov, znear: camera.near, viewport: None });
    }
    if let Some(target) = &spec.look_at {
        node.set_property(LookAt { target: target.clone() });
    }
    node
}

/// Puts `spec`'s parent, place, lens and aim on its node in the scene.
fn apply_spec(scene: &mut Scene, spec: &NodeSpec) {
    let parent = parent_property(spec, scene);
    let Some(node) = scene.content.nodes.get_mut(&spec.id) else { return };
    match parent {
        Some(parent) => {
            if parent.space == Space::Global {
                node.set_property(given_transform(spec));
            }
            node.set_property(parent);
        }
        None => {
            node.remove_property::<Parent>();
            node.set_property(given_transform(spec));
        }
    }
    match spec.camera {
        Some(camera) => {
            let viewport = node.get_property::<CameraLens>().and_then(|lens| lens.viewport);
            node.set_property(CameraLens { fovy: camera.fov, znear: camera.near, viewport });
        }
        None => {
            if node.remove_property::<CameraLens>().is_some() {
                scene.cameras.remove_camera(&spec.id);
            }
        }
    }
    let Some(node) = scene.content.nodes.get_mut(&spec.id) else { return };
    match &spec.look_at {
        Some(target) => node.set_property(LookAt { target: target.clone() }),
        None => { node.remove_property::<LookAt>(); }
    }
}
