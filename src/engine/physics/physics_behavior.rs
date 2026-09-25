use std::any::{Any, TypeId};
use std::collections::HashMap;

use nalgebra::Vector3;
use rapier3d::prelude::{ColliderSet, QueryPipeline, RigidBody, RigidBodyHandle, RigidBodySet};

use crate::engine::physics::physics::DebugPhysicsMessageType;
use crate::engine::primitive::manual_vertex::ManualVertex;

/// Anything crossing the thread boundary between a node's two halves - a
/// type-erased box the engine routes by node id without ever opening it.
/// Only the node that receives it downcasts it back into its real type
/// (same trick `Node`'s own properties use).
pub type PhysicsPayload = Box<dyn Any + Send>;

/// The physics-thread half of a node - attached with
/// `Node::add_physics_behavior`, moved onto the physics thread once when the
/// scene's physics starts, and from then on run at the real fixed rate
/// (`FIXED_TIMESTEP`), right before every `rapier` step.
///
/// It never sees its `Node` (not `Send`, stays on the main thread). The two
/// halves only talk through typed messages:
/// - main → physics: `Node::set_physics_input` / `Node::push_physics_event`,
///   read here via `PhysicsCtx::input` / `PhysicsCtx::events`.
/// - physics → main: whatever `publish` returns, read on the main thread via
///   `Node::physics_state::<T>()`.
pub trait PhysicsBehavior: Send {
    /// One fixed physics step - `dt` is always `FIXED_TIMESTEP`.
    fn fixed_update(&mut self, ctx: &mut PhysicsCtx, dt: f32);

    /// Snapshot to send back to the main thread, called once per rendered
    /// frame (whenever the main thread requests physics data), not once per
    /// step. Stored on the node keyed by its concrete type, so two physics
    /// behaviors on the same node publishing different types don't clobber
    /// each other.
    fn publish(&self) -> Option<PhysicsPayload> {
        None
    }

    /// Debug lines for the F2 overlay, in world space. Called on the physics
    /// thread once per rendered frame (right alongside `publish`, not once
    /// per step) and only while the overlay is on - so it should just draw
    /// whatever the last `fixed_update` left behind, not compute anything.
    /// `body` is this node's own rigidbody, for turning body-local points
    /// into world space.
    fn debug_draw(&self, body: &RigidBody, draw: &mut DebugDraw) {
        let _ = (body, draw);
    }
}

/// Collects a frame's debug lines from every `PhysicsBehavior::debug_draw`,
/// all in world space - the renderer makes them camera-relative itself.
#[derive(Default)]
pub struct DebugDraw {
    lines: Vec<DebugPhysicsMessageType>,
}

impl DebugDraw {
    /// A line from `from` to `to`.
    pub fn line(&mut self, from: Vector3<f32>, to: Vector3<f32>, color: [f32; 3]) {
        self.lines.push(DebugPhysicsMessageType::RenderizableLines([
            ManualVertex { position: from.into(), color },
            ManualVertex { position: to.into(), color },
        ]));
    }

    /// A line starting at `origin`, pointing along `vector` - e.g. a force
    /// drawn from where it's applied.
    pub fn ray(&mut self, origin: Vector3<f32>, vector: Vector3<f32>, color: [f32; 3]) {
        self.line(origin, origin + vector, color);
    }

    pub(crate) fn into_lines(self) -> Vec<DebugPhysicsMessageType> {
        self.lines
    }
}

/// Main → physics messages for one node, split by how they have to be delivered:
/// - `states`: latest-value (stick position, throttle...) - a newer one of the
///   same type simply replaces the older one.
/// - `events`: one-shot (reset, toggle, fire...) - queued in order and seen by
///   exactly one fixed step, so they're never dropped or repeated no matter
///   how many steps (zero, one, several) run per rendered frame.
#[derive(Default)]
pub struct PhysicsInput {
    states: HashMap<TypeId, PhysicsPayload>,
    events: Vec<PhysicsPayload>,
}

impl PhysicsInput {
    pub(crate) fn set_state<T: Any + Send>(&mut self, state: T) {
        self.states.insert(TypeId::of::<T>(), Box::new(state));
    }

    pub(crate) fn push_event<T: Any + Send>(&mut self, event: T) {
        self.events.push(Box::new(event));
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.states.is_empty() && self.events.is_empty()
    }

    /// Folds a newer batch into this one - states overwrite, events append.
    pub(crate) fn merge(&mut self, newer: PhysicsInput) {
        self.states.extend(newer.states);
        self.events.extend(newer.events);
    }

    pub(crate) fn clear_events(&mut self) {
        self.events.clear();
    }
}

/// Everything a `PhysicsBehavior` gets for one fixed step: its own node's
/// rigidbody, the shared physics world, and whatever the main-thread half
/// sent it.
pub struct PhysicsCtx<'a> {
    pub body: RigidBodyHandle,
    pub bodies: &'a mut RigidBodySet,
    pub colliders: &'a ColliderSet,
    pub query: &'a QueryPipeline,
    input: &'a PhysicsInput,
}

impl<'a> PhysicsCtx<'a> {
    /// This node's own rigidbody.
    pub fn rigidbody(&self) -> &RigidBody {
        &self.bodies[self.body]
    }

    pub fn rigidbody_mut(&mut self) -> &mut RigidBody {
        &mut self.bodies[self.body]
    }

    /// Latest input state of type `T` the main-thread half sent (see
    /// `Node::set_physics_input`), `None` until it has sent one.
    pub fn input<T: Any>(&self) -> Option<&T> {
        self.input.states.get(&TypeId::of::<T>())?.downcast_ref::<T>()
    }

    /// Every queued event of type `T` for this step (see
    /// `Node::push_physics_event`) - gone again after this step.
    pub fn events<T: Any>(&self) -> impl Iterator<Item = &T> {
        self.input.events.iter().filter_map(|event| event.downcast_ref::<T>())
    }
}

/// One node's physics half as it lives on the physics thread.
pub(crate) struct PhysicsNode {
    pub(crate) id: String,
    body: RigidBodyHandle,
    behaviors: Vec<Box<dyn PhysicsBehavior>>,
    input: PhysicsInput,
}

impl PhysicsNode {
    pub(crate) fn new(id: String, body: RigidBodyHandle, behaviors: Vec<Box<dyn PhysicsBehavior>>) -> Self {
        PhysicsNode { id, body, behaviors, input: PhysicsInput::default() }
    }

    pub(crate) fn receive_input(&mut self, input: PhysicsInput) {
        self.input.merge(input);
    }

    /// Runs every behavior's `fixed_update` for one step, then consumes the
    /// queued events so the next step doesn't see them again.
    pub(crate) fn fixed_update(&mut self, bodies: &mut RigidBodySet, colliders: &ColliderSet, query: &QueryPipeline, dt: f32) {
        let mut ctx = PhysicsCtx { body: self.body, bodies, colliders, query, input: &self.input };
        for behavior in self.behaviors.iter_mut() {
            behavior.fixed_update(&mut ctx, dt);
        }
        self.input.clear_events();
    }

    pub(crate) fn publish(&self) -> Vec<PhysicsPayload> {
        self.behaviors.iter().filter_map(|behavior| behavior.publish()).collect()
    }

    pub(crate) fn debug_draw(&self, bodies: &RigidBodySet, draw: &mut DebugDraw) {
        let Some(body) = bodies.get(self.body) else { return };
        for behavior in &self.behaviors {
            behavior.debug_draw(body, draw);
        }
    }
}
