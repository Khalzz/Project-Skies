use std::collections::HashMap;

use crate::app::App;
use crate::engine::physics::physics_behavior::{PhysicsBehavior, PhysicsInput, PhysicsPayload};
use crate::engine::physics::physics_handler::RenderMessage;
use crate::engine::rendering::camera::handler::SceneCameras;

use nalgebra::{UnitQuaternion, Vector3};

use crate::engine::game_nodes::game_object::Physics;

use super::behavior::Behavior;
use super::node::Node;
use super::properties::{LookAt, Parent, Space, Transform3D};

/// How deep a chain of parents is followed before it's taken for a loop.
const MAX_DEPTH: usize = 64;

/// Every spawned `Node` in a scene, kept flat by id - no node owns another.
/// The hierarchy is a `Parent` property on each child (see `Node::with_child`)
/// instead of literal nesting: a flat map lets any node be reached by id and
/// two nodes be mutated independently, which nested ownership would fight.
/// `resolve_hierarchy` is what makes children follow their parents.
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

    pub fn iter(&self) -> impl Iterator<Item = &Node> {
        self.nodes.values()
    }

    /// The ids of `id`'s direct children (nodes whose `Parent` is it).
    pub fn children_of(&self, id: &str) -> Vec<String> {
        self.nodes.values()
            .filter(|node| node.get_property::<Parent>().is_some_and(|parent| parent.id == id))
            .map(|node| node.id.clone())
            .collect()
    }

    /// `id`'s children, their children and so on - deepest last.
    pub fn descendants_of(&self, id: &str) -> Vec<String> {
        let mut found = Vec::new();
        let mut next = vec![id.to_owned()];
        while let Some(current) = next.pop() {
            for child in self.children_of(&current) {
                if found.len() < self.nodes.len() && !found.contains(&child) {
                    next.push(child.clone());
                    found.push(child);
                }
            }
        }
        found
    }

    /// Where `id` is in the world - its `Transform3D` (identity for a node
    /// without one, like a pure logic node).
    pub fn world_transform(&self, id: &str) -> Option<Transform3D> {
        let node = self.nodes.get(id)?;
        Some(node.get_property::<Transform3D>().copied().unwrap_or_default())
    }

    /// How many parents up `id` goes (0 for a node with none). A chain that
    /// loops back on itself stops at MAX_DEPTH.
    fn depth(&self, id: &str) -> usize {
        let mut depth = 0;
        let mut current = id;
        while let Some(parent) = self.nodes.get(current).and_then(|node| node.get_property::<Parent>()) {
            depth += 1;
            if depth >= MAX_DEPTH {
                break;
            }
            current = &parent.id;
        }
        depth
    }

    /// Places every `Local` child where its parent now is plus its offset,
    /// then turns every `LookAt` node to face its target - parents before
    /// their children, so a whole chain follows in the same frame. Called
    /// every frame by `App::run`, after everything that moves nodes
    /// (physics, behaviors, animation) and before the renderer and the
    /// cameras read them.
    pub fn resolve_hierarchy(&mut self) {
        let mut order: Vec<(usize, String)> = self.nodes.values()
            .filter(|node| {
                let follows = node.get_property::<Parent>().is_some_and(|parent| parent.space == Space::Local)
                    && node.get_property::<Physics>().is_none();
                follows || node.get_property::<LookAt>().is_some()
            })
            .map(|node| (self.depth(&node.id), node.id.clone()))
            .collect();
        if order.is_empty() {
            return;
        }
        order.sort();

        for (_, id) in order {
            let Some(node) = self.nodes.get(&id) else { continue };
            let parent = node.get_property::<Parent>().filter(|parent| parent.space == Space::Local && node.get_property::<Physics>().is_none()).cloned();
            let look_at = node.get_property::<LookAt>().map(|look_at| look_at.target.clone());

            let mut world = node.get_property::<Transform3D>().copied().unwrap_or_default();
            if let Some(parent) = parent {
                // A parent that's gone leaves the child where it last was.
                if let Some(parent_world) = self.world_transform(&parent.id) {
                    world = parent_world.compose(&parent.local, parent.inherit_rotation);
                }
            }
            if let Some(target) = look_at.and_then(|target| self.world_transform(&target)) {
                if let Some(rotation) = facing(target.position - world.position) {
                    world.rotation = rotation;
                }
            }
            if let Some(node) = self.nodes.get_mut(&id) {
                node.set_property(world);
            }
        }
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Node> {
        self.nodes.values_mut()
    }

    /// Moves every node's physics half out, keyed by node id - called once,
    /// when the scene's physics thread starts (see `Scene::start_physics`).
    pub(crate) fn take_physics_behaviors(&mut self) -> Vec<(String, Vec<Box<dyn PhysicsBehavior>>)> {
        self.nodes.values_mut()
            .map(|node| (node.id.clone(), node.take_physics_behaviors()))
            .filter(|(_, behaviors)| !behaviors.is_empty())
            .collect()
    }

    /// Collects this frame's main → physics messages from every node, keyed
    /// by node id - emptied on the node side, so nothing is sent twice.
    pub(crate) fn take_physics_inputs(&mut self) -> HashMap<String, PhysicsInput> {
        self.nodes.values_mut()
            .filter_map(|node| node.take_physics_input().map(|input| (node.id.clone(), input)))
            .collect()
    }

    /// Hands each node the states its physics half published.
    pub(crate) fn apply_physics_states(&mut self, states: HashMap<String, Vec<PhysicsPayload>>) {
        for (id, node_states) in states {
            if let Some(node) = self.nodes.get_mut(&id) {
                node.set_physics_states(node_states);
            }
        }
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
    /// `physics_data` is this frame's physics-thread results, keyed by node
    /// id (same map `FrameContext::physics_data` carries to
    /// `SceneBehaviour::update`) - each node's own behaviors get just their
    /// own entry, so "react to this node's physics results" is a generic
    /// hook any `Behavior` can implement instead of a scene hand-picking one
    /// node by name (see `Plane::fixed_update`'s own doc comment for the
    /// case this replaced).
    pub fn fixed_update(&mut self, cameras: &mut SceneCameras, app: &mut App, dt: f32, physics_data: &HashMap<String, RenderMessage>) {
        for node in self.nodes.values_mut() {
            let physics_message = physics_data.get(&node.id);
            node.run_fixed_update(cameras, app, dt, physics_message);
        }
    }
}

/// The rotation that points +Z along `direction` with +Y as near up as it
/// gets - `None` for no direction at all.
pub fn facing(direction: Vector3<f32>) -> Option<UnitQuaternion<f32>> {
    let direction = direction.try_normalize(1e-6)?;
    // Straight up or down: any heading will do, Z is as good as any.
    let up = if direction.y.abs() > 0.999 { Vector3::z() } else { Vector3::y() };
    Some(UnitQuaternion::face_towards(&direction, &up))
}
