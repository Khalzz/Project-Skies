use std::collections::HashMap;

use crate::app::App;
use crate::engine::rendering::camera::handler::SceneCameras;

use super::behavior::Behavior;
use super::node::Node;

/// Every spawned `Node` in a scene, flat - no node owns another node.
/// Deliberately not a tree: a flat map lets two different nodes be reached
/// and mutated independently without fighting the borrow checker the way
/// nested ownership would, and nothing here needs real parent-child transform
/// composition today. If an object ever needs to visually follow another,
/// that's a `Parent` property plus a small system resolving it, not literal
/// nesting.
///
/// `spawn`/`update`/`fixed_update` all take `&mut App` now, to hand down to
/// `Behavior`'s hooks (see that trait's own doc comment on why) - which means
/// this type can no longer be unit-tested on its own the way it originally
/// was: `App` needs a live window/GPU context to construct, not something a
/// unit test can reasonably spin up. The behavior-dispatch tests that used to
/// live in this file (duplicate-id guard, on_spawn/update/fixed_update call
/// counts) were removed for that reason, not because the behavior they
/// covered stopped mattering.
#[derive(Default)]
pub struct SceneNodes {
    nodes: HashMap<String, Node>,
}

impl SceneNodes {
    pub fn new() -> Self {
        SceneNodes { nodes: HashMap::new() }
    }

    /// Inserts `node`, refusing to silently overwrite an existing id - a
    /// duplicate id is almost always an authoring bug (every lookup here
    /// treats id as the unique address for that node), so this is loud about
    /// it instead of letting one node quietly clobber another.
    pub fn spawn(&mut self, mut node: Node, cameras: &mut SceneCameras, app: &mut App) -> Result<(), String> {
        if self.nodes.contains_key(&node.id) {
            let message = format!("scene already has a node named '{}' - refusing to overwrite it", node.id);
            eprintln!("{message}");
            return Err(message);
        }

        node.run_on_spawn(cameras, app);
        self.nodes.insert(node.id.clone(), node);
        Ok(())
    }

    /// Convenience for the common case: build a fresh `Node` and attach one
    /// behavior to it in a single call - `scene.spawn_with_behavior("player",
    /// Player::new(start_position), app)`.
    pub fn spawn_with_behavior<B: Behavior + 'static>(&mut self, id: impl Into<String>, behavior: B, cameras: &mut SceneCameras, app: &mut App) -> Result<(), String> {
        self.spawn(Node::new(id).add_behavior(behavior), cameras, app)
    }

    pub fn get(&self, id: &str) -> Option<&Node> {
        self.nodes.get(id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut Node> {
        self.nodes.get_mut(id)
    }

    pub fn remove(&mut self, id: &str) -> Option<Node> {
        self.nodes.remove(id)
    }

    /// Drops every spawned node - a scene-reset step (see `App::run`), so a
    /// node/behavior spawned by one scene (e.g. `sandbox`'s `Camera`) doesn't
    /// keep getting `update`/`fixed_update` calls after switching to a scene
    /// that never spawned it. No `on_despawn` hook exists on `Behavior` yet,
    /// so anything a behavior did outside its own node on spawn (`Camera`
    /// registering itself with `cameras`, hiding the cursor) isn't
    /// automatically undone here - only the node/behavior itself goes away.
    pub fn clear(&mut self) {
        self.nodes.clear();
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Node> {
        self.nodes.values_mut()
    }

    /// Runs every node's attached behaviors' `update` - called once per
    /// rendered frame from `App::run`.
    pub fn update(&mut self, cameras: &mut SceneCameras, app: &mut App, dt: f32) {
        for node in self.nodes.values_mut() {
            node.run_update(cameras, app, dt);
        }
    }

    /// Runs every node's attached behaviors' `fixed_update` - see
    /// `Behavior::fixed_update`'s own doc comment for why this is an
    /// approximation of a true fixed tick, not the real physics-thread rate.
    pub fn fixed_update(&mut self, cameras: &mut SceneCameras, app: &mut App, dt: f32) {
        for node in self.nodes.values_mut() {
            node.run_fixed_update(cameras, app, dt);
        }
    }
}
