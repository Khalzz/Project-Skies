use std::any::Any;

use crate::app::App;
use super::node::Node;
use super::scene_access::SceneAccess;

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
/// enemy's AI, ...), attached to a `Node` via `Node::with_behavior`.
///
/// Distinct from a property: a property is plain, generic data any system
/// might read (`Transform3D`, `Model`) - a behavior is one element's own
/// logic, with exclusive ownership of its own fields, driven by these hooks
/// instead of external systems reaching in.
///
/// Every hook gets its own `node` (its properties: `node.transform_2d_mut()`,
/// `node.get_property_mut::<T>()`...), the rest of the scene through `scene`
/// (other nodes, the cameras, spawning and despawning - see `SceneAccess`),
/// and `app` for anything outside the scene (`app.time.delta_time`,
/// `app.audio`, ...).
///
/// The real cost of `app`: `Node`/`SceneNodes` can't be unit-tested without a
/// live `App`, which needs a real window/GPU context to construct - so
/// there's no cheap, isolated test path for behavior dispatch (see
/// `scene_nodes.rs`'s own comment on what this replaced).
///
/// All are no-ops by default - implement only the ones a given behavior
/// actually needs.
pub trait Behavior: Any + AsAny {
    /// Runs once, right after this node is actually spawned into a
    /// `SceneNodes` - not the behavior's own constructor, since properties it
    /// depends on may only be visible once the node is fully built (mirrors
    /// Godot's `_ready` vs `_init` split).
    ///
    /// A behavior can set its own node up here: a `Model`, `Physics` or
    /// `CameraLens` it puts on (`node.set_property(..)`) is drawn, given a
    /// body or made a camera the same as one the node was spawned with.
    fn on_spawn(&mut self, node: &mut Node, scene: &mut SceneAccess, app: &mut App) {
        let _ = (node, scene, app);
    }

    /// Runs every rendered frame - how long it was is `app.time.delta_time`.
    fn update(&mut self, node: &mut Node, scene: &mut SceneAccess, app: &mut App) {
        let _ = (node, scene, app);
    }

    /// Runs every physics step (`app.physics.timestep()` long - 120 a
    /// second by default), right before the world advances by it - none,
    /// one or several times a frame, however many the time that passed
    /// calls for. Where a body is pushed or its velocity set
    /// (`app.physics.body_mut(&node.id)`), so it moves the same at any frame
    /// rate.
    ///
    /// Input works here: held keys as anywhere, and a key `just_pressed`
    /// since the last step is seen by exactly one step - never missed, never
    /// twice. Mouse movement is a frame's, though: read it in `update`.
    /// The node's `Transform3D` is where it was drawn last frame - its body
    /// is where it really is now.
    fn fixed_update(&mut self, node: &mut Node, scene: &mut SceneAccess, app: &mut App) {
        let _ = (node, scene, app);
    }
}
