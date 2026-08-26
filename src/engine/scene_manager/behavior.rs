use std::any::Any;

use crate::app::App;

use super::node::Node;

/// Custom per-element logic and state (a `Player` with its own `lives`, an
/// enemy's AI, ...), attached to a `Node` via `Node::add_behavior`.
///
/// Distinct from a property: a property is plain, generic data any system
/// might read (`Transform3D`, `Model`) - a behavior is one element's own
/// logic, with exclusive ownership of its own fields, driven by these three
/// hooks instead of external systems reaching in. A behavior can still reach
/// into its node's properties (`node.get_property_mut::<Transform3D>()`) from
/// any of these when it needs to.
///
/// Every hook also gets `&mut App` - needed for anything that drives a
/// subsystem outside the node itself (`app.camera`, `app.audio`, ...), which
/// turned out to be most behaviors worth writing, not an edge case. The real
/// cost of this: `Node`/`SceneNodes` can no longer be unit-tested without a
/// live `App`, which needs a real window/GPU context to construct - so
/// there's no cheap, isolated test path for behavior dispatch anymore (see
/// `scene_nodes.rs`'s own comment on what this replaced).
///
/// All three are no-ops by default - implement only the ones a given
/// behavior actually needs.
pub trait Behavior: Any {
    /// Runs once, right after this node is actually spawned into a
    /// `SceneNodes` - not the behavior's own constructor, since properties it
    /// depends on may only be visible once the node is fully built (mirrors
    /// Godot's `_ready` vs `_init` split).
    fn on_spawn(&mut self, node: &mut Node, app: &mut App) {
        let _ = (node, app);
    }

    /// Runs every rendered frame - variable `dt`.
    fn update(&mut self, node: &mut Node, app: &mut App, dt: f32) {
        let _ = (node, app, dt);
    }

    /// Reacts to physics results once per frame - an approximation of a true
    /// fixed 120Hz tick, not the real physics-thread rate. `Node`'s `Box<dyn
    /// Any>` properties aren't `Send`, so running this on the actual physics
    /// thread (see `engine::physics::physics_handler::Physics::physics_thread`)
    /// would require making the whole property system `Send`-safe - not worth
    /// it for logic that reacts to results rather than computing forces or
    /// collisions itself (that belongs in a `PhysicsTick` impl instead, which
    /// already runs on the real physics thread and never touches `Node`).
    fn fixed_update(&mut self, node: &mut Node, app: &mut App, dt: f32) {
        let _ = (node, app, dt);
    }
}
