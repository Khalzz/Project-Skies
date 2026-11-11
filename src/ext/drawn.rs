//! Where a node is drawn this frame, and the view points it carries - what
//! the game used to read off its engine's render instances
//! (`renderizable_instances`), which the engine keeps to itself. Read from
//! the node: its world transform (physics has already put it there this
//! frame), and its `Cameras` property.

use pankarta_software::prelude::*;

use super::game_object::Cameras;

/// A drawn node, shaped like the game's old render instance.
pub struct Drawn {
    pub instance: DrawnInstance,
}

pub struct DrawnInstance {
    pub transform: Transform3D,
    pub metadata: DrawnMetadata,
}

pub struct DrawnMetadata {
    /// Its named view points (a plane's cockpit, chase...) - see
    /// game_object::Cameras.
    pub cameras: Option<Cameras>,
}

/// Node `id` as it's drawn - None if there's no such node, or it has no
/// model or mesh to be drawn with.
pub fn get(scene: &Scene, id: &str) -> Option<Drawn> {
    let node = scene.get_node(id)?;
    if node.get_property::<Model>().is_none() && node.get_property::<Mesh>().is_none() {
        return None;
    }
    let transform = scene.world_transform(id)?;
    Some(Drawn { instance: DrawnInstance { transform, metadata: DrawnMetadata { cameras: node.get_property::<Cameras>().cloned() } } })
}

/// Whether node `id` is drawn (see `get`).
pub fn exists(scene: &Scene, id: &str) -> bool {
    scene.get_node(id).is_some_and(|node| node.get_property::<Model>().is_some() || node.get_property::<Mesh>().is_some())
}
