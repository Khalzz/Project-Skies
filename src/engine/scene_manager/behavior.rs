use std::any::Any;

use crate::app::App;
use crate::engine::rendering::camera::handler::SceneCameras;

use super::node::Node;

/// Splits off `as_any`/`as_any_mut` into their own supertrait purely so they
/// can be given a blanket impl - a trait's own default method body doesn't
/// get an implicit `Self: Sized` the way a plain generic fn/impl does (that's
/// deliberate, it's what keeps the trait object-safe), so `self` can't
/// coerce to `&dyn Any` directly inside `Behavior`'s own default methods.
/// Hoisting them here, where `impl<T: Any> AsAny for T` is an ordinary
/// generic impl (T: Sized by default), sidesteps that - every `Behavior`
/// implementor gets a working `as_any`/`as_any_mut` for free, no boilerplate
/// per type, same trick `downcast-rs`-style crates use.
pub trait AsAny {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T: Any> AsAny for T {
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}

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
/// Every hook also gets `&mut SceneCameras` and `&mut App` - `cameras` for
/// anything that drives the owning scene's own cameras (e.g. a free-fly
/// `Camera` behavior), `app` for anything else outside the node itself
/// (`app.audio`, ...). `cameras` is threaded in separately from a `&mut
/// Scene` rather than handing the whole scene over - a node's behaviors run
/// from inside `scene.content.nodes`, already borrowed off `scene`, so only
/// sibling fields of `content` (like `cameras`) can be reached at the same
/// time, not `scene` itself. The real cost of all this: `Node`/`SceneNodes`
/// can no longer be unit-tested without a live `App`, which needs a real
/// window/GPU context to construct - so there's no cheap, isolated test path
/// for behavior dispatch anymore (see `scene_nodes.rs`'s own comment on what
/// this replaced).
///
/// All three are no-ops by default - implement only the ones a given
/// behavior actually needs.
pub trait Behavior: Any + AsAny {
    /// Runs once, right after this node is actually spawned into a
    /// `SceneNodes` - not the behavior's own constructor, since properties it
    /// depends on may only be visible once the node is fully built (mirrors
    /// Godot's `_ready` vs `_init` split).
    fn on_spawn(&mut self, node: &mut Node, cameras: &mut SceneCameras, app: &mut App) {
        let _ = (node, cameras, app);
    }

    /// Runs every rendered frame - variable `dt`.
    fn update(&mut self, node: &mut Node, cameras: &mut SceneCameras, app: &mut App, dt: f32) {
        let _ = (node, cameras, app, dt);
    }

    /// Reacts to physics results once per frame - an approximation of a true
    /// fixed 120Hz tick, not the real physics-thread rate. `Node`'s `Box<dyn
    /// Any>` properties aren't `Send`, so running this on the actual physics
    /// thread (see `engine::physics::physics_handler::Physics::physics_thread`)
    /// would require making the whole property system `Send`-safe - not worth
    /// it for logic that reacts to results rather than computing forces or
    /// collisions itself (that belongs in a `PhysicsTick` impl instead, which
    /// already runs on the real physics thread and never touches `Node`).
    fn fixed_update(&mut self, node: &mut Node, cameras: &mut SceneCameras, app: &mut App, dt: f32) {
        let _ = (node, cameras, app, dt);
    }
}
