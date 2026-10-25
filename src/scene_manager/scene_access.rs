//! What a node's behaviors reach the rest of their scene through - every
//! other node, the cameras, and spawning and despawning:
//!
//!     fn update(&mut self, node: &mut Node, scene: &mut SceneAccess, app: &mut App) {
//!         let target = scene.get_node("player").and_then(|player| player.transform_2d()).map(|t| t.position);
//!         if let Some(door) = scene.get_node_mut("door") { /* ... */ }
//!         scene.spawn(Node::new("bullet"));
//!     }
//!
//! While a node's behaviors run, that node is taken out of the scene - it's
//! handed to them as `node` instead - so `get_node` doesn't find it (they
//! already have it), and everything else can be changed freely, other nodes
//! included, without ever touching it.
//!
//! Changes to nodes and cameras happen right away. Spawning and despawning
//! wait until every behavior has run (the scene's nodes can't come and go
//! while they're being walked) - then they happen on their own, the same
//! frame.

use crate::rendering::camera::handler::SceneCameras;

use super::node::Node;
use super::scene_nodes::SceneNodes;

/// See the module doc.
pub struct SceneAccess<'a> {
    nodes: &'a mut SceneNodes,
    cameras: &'a mut SceneCameras,
    changes: &'a mut SceneChanges,
}

/// Spawns and despawns behaviors asked for (see `SceneAccess::spawn`),
/// waiting to be done once they've all run.
#[derive(Default)]
pub(crate) struct SceneChanges {
    pub(crate) spawns: Vec<Node>,
    pub(crate) despawns: Vec<String>,
}

impl<'a> SceneAccess<'a> {
    pub(crate) fn new(nodes: &'a mut SceneNodes, cameras: &'a mut SceneCameras, changes: &'a mut SceneChanges) -> Self {
        Self { nodes, cameras, changes }
    }

    /// The node `id` - None if there's none, or it's the one whose behaviors
    /// are running (that one is their `node`).
    pub fn get_node(&self, id: &str) -> Option<&Node> {
        self.nodes.get(id)
    }

    /// The node `id`, to change - right away (see `get_node`).
    pub fn get_node_mut(&mut self, id: &str) -> Option<&mut Node> {
        self.nodes.get_mut(id)
    }

    /// Every other node.
    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.iter()
    }

    pub fn cameras(&self) -> &SceneCameras {
        self.cameras
    }

    pub fn cameras_mut(&mut self) -> &mut SceneCameras {
        self.cameras
    }

    /// Adds `node` to the scene once every behavior has run this frame (see
    /// the module doc) - with its children, its model, its physics body, like
    /// `Scene::spawn_node`.
    pub fn spawn(&mut self, node: Node) {
        self.changes.spawns.push(node);
    }

    /// Removes the node `id` (and its children) once every behavior has run
    /// this frame, like `Scene::despawn_node`. It can be this node's own id.
    pub fn despawn(&mut self, id: impl Into<String>) {
        self.changes.despawns.push(id.into());
    }
}
