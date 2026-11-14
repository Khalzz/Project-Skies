//! The game's "physics half" of a node - `PhysicsBehavior` - run on the
//! engine's in-process physics world.
//!
//! The game used to run these on its own physics thread, talking to the node
//! only through typed messages. The engine steps physics on the main thread
//! instead (`Behavior::fixed_update`, right before every step), so this
//! module keeps the same API on top of it:
//!
//! - `PhysicsHost` is an ordinary engine `Behavior` holding the node's
//!   physics behaviors - added for you by `add_physics_behavior`. Every
//!   physics step it runs their `fixed_update` with a `PhysicsCtx` built from
//!   `app.physics`; every frame it collects what they `publish`, and draws
//!   their `debug_draw` into the physics debug view.
//! - The messages live on the node as a `PhysicsLink` property (a node's
//!   behaviors are taken off it while they run, its properties aren't), so
//!   any behavior or scene code can write inputs and read states:
//!   - main → physics: `set_physics_input` (latest value wins) and
//!     `push_physics_event` (seen by exactly one step);
//!   - physics → main: `physics_state::<T>()`.
//!
//! All of it through `PhysicsNodeExt`, implemented for the engine's `Node`.

use std::any::{Any, TypeId};
use std::collections::HashMap;

use nalgebra::Vector3;
use pankarta_software::prelude::*;
use pankarta_software::rendering::lines::Lines;
use rapier3d::prelude::{Collider, ColliderSet, NarrowPhase, QueryPipeline, RigidBody, RigidBodyHandle, RigidBodySet};

/// Anything crossing between a node's two halves - a type-erased box only
/// the side that receives it downcasts back into its real type.
pub type PhysicsPayload = Box<dyn Any>;

/// The physics half of a node - attached with
/// `PhysicsNodeExt::add_physics_behavior`, run every physics step right
/// before the world advances.
///
/// It never sees its `Node` - only its body and the world. The two halves
/// only talk through typed messages (see the module doc).
pub trait PhysicsBehavior: 'static {
    /// One fixed physics step - `dt` is the engine's timestep
    /// (`app.physics.timestep()`).
    fn fixed_update(&mut self, ctx: &mut PhysicsCtx, dt: f32);

    /// Snapshot for the main half, taken once per rendered frame (not once
    /// per step). Stored on the node keyed by its concrete type, so two
    /// physics behaviors publishing different types don't clobber each other.
    fn publish(&self) -> Option<PhysicsPayload> {
        None
    }

    /// Debug lines for the physics debug view, in world space. Called once
    /// per rendered frame and only while the view is on - so it should just
    /// draw whatever the last `fixed_update` left behind, not compute
    /// anything. `body` is this node's own rigidbody.
    fn debug_draw(&self, body: &RigidBody, draw: &mut DebugDraw) {
        let _ = (body, draw);
    }
}

/// Draws a frame's debug lines from `PhysicsBehavior::debug_draw` into the
/// engine's physics debug view (`app.physics.debug_lines()`), in world space.
pub struct DebugDraw<'a> {
    lines: &'a mut Lines,
}

impl<'a> DebugDraw<'a> {
    /// Lines drawn after this belong to `owner` (a node id) - kept for the
    /// old API; the engine draws every line where it's given.
    pub fn set_owner(&mut self, _owner: Option<String>) {}

    /// A line from `from` to `to`.
    pub fn line(&mut self, from: Vector3<f32>, to: Vector3<f32>, color: [f32; 3]) {
        self.lines.line(from, to, color);
    }

    /// A line starting at `origin`, pointing along `vector` - e.g. a force
    /// drawn from where it's applied.
    pub fn ray(&mut self, origin: Vector3<f32>, vector: Vector3<f32>, color: [f32; 3]) {
        self.lines.ray(origin, vector, color);
    }

    /// A small 3-axis cross marking a point, `size` meters from end to end.
    pub fn cross(&mut self, center: Vector3<f32>, size: f32, color: [f32; 3]) {
        self.lines.cross(center, size, color);
    }

    /// A closed outline through `corners`, in order.
    pub fn outline(&mut self, corners: &[Vector3<f32>], color: [f32; 3]) {
        self.lines.outline(corners, color);
    }

    /// `collider`'s shape as a wireframe, where it actually is in the world.
    pub fn collider(&mut self, collider: &Collider, color: [f32; 3]) {
        pankarta_software::physics::world::draw_shape(self.lines, collider.position(), collider.shape(), color);
    }
}

/// Main → physics messages for one node, split by how they have to be
/// delivered:
/// - `states`: latest-value (stick position, throttle...) - a newer one of
///   the same type simply replaces the older one.
/// - `events`: one-shot (reset, toggle, fire...) - queued in order and seen
///   by exactly one fixed step, so they're never dropped or repeated no
///   matter how many steps (zero, one, several) run per rendered frame.
#[derive(Default)]
pub struct PhysicsInput {
    states: HashMap<TypeId, PhysicsPayload>,
    events: Vec<PhysicsPayload>,
}

/// Everything a `PhysicsBehavior` gets for one fixed step: its own node's
/// rigidbody, the physics world, and whatever the main half sent it.
pub struct PhysicsCtx<'a> {
    pub body: RigidBodyHandle,
    pub bodies: &'a mut RigidBodySet,
    pub colliders: &'a mut ColliderSet,
    /// Contacts from the step that just ran. This step's own contacts are
    /// written when the world advances, after `fixed_update`.
    pub narrow_phase: &'a NarrowPhase,
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

    /// Latest input state of type `T` the main half sent (see
    /// `set_physics_input`), `None` until it has sent one.
    pub fn input<T: Any>(&self) -> Option<&T> {
        self.input.states.get(&TypeId::of::<T>())?.downcast_ref::<T>()
    }

    /// Every queued event of type `T` for this step (see
    /// `push_physics_event`) - gone again after this step.
    pub fn events<T: Any>(&self) -> impl Iterator<Item = &T> {
        self.input.events.iter().filter_map(|event| event.downcast_ref::<T>())
    }
}

/// The node property carrying a node's physics messages both ways (see the
/// module doc) - put on by `add_physics_behavior`.
#[derive(Default)]
pub struct PhysicsLink {
    input: PhysicsInput,
    states: HashMap<TypeId, PhysicsPayload>,
}

/// The engine `Behavior` running a node's `PhysicsBehavior`s - see the
/// module doc.
pub struct PhysicsHost {
    behaviors: Vec<Box<dyn PhysicsBehavior>>,
}

impl Behavior for PhysicsHost {
    fn fixed_update(&mut self, node: &mut Node, _scene: &mut SceneAccess, app: &mut App) {
        let dt = app.physics.timestep();
        let Some(body) = app.physics.handle(&node.id) else { return };
        let Some(link) = node.get_property_mut::<PhysicsLink>() else { return };
        let physics = &mut app.physics;
        let mut ctx = PhysicsCtx {
            body,
            bodies: &mut physics.bodies,
            colliders: &mut physics.colliders,
            narrow_phase: &physics.narrow_phase,
            query: &physics.query,
            input: &link.input,
        };
        for behavior in &mut self.behaviors {
            behavior.fixed_update(&mut ctx, dt);
        }
        // Seen by this step - not by the next.
        link.input.events.clear();
    }

    fn update(&mut self, node: &mut Node, _scene: &mut SceneAccess, app: &mut App) {
        if let Some(link) = node.get_property_mut::<PhysicsLink>() {
            for state in self.behaviors.iter().filter_map(|behavior| behavior.publish()) {
                // `&*state`, not `state` - the Box's own TypeId would be
                // `Box<dyn Any>`, not the type inside it.
                link.states.insert(Any::type_id(&*state), state);
            }
        }

        // Only while the physics debug view is on.
        if app.physics.debug_lines().is_none() {
            return;
        }
        let Some(handle) = app.physics.handle(&node.id) else { return };
        let physics = &mut app.physics;
        let Some(body) = physics.bodies.get(handle).cloned() else { return };
        let Some(lines) = physics.debug_lines() else { return };
        let mut draw = DebugDraw { lines };
        for behavior in &self.behaviors {
            behavior.debug_draw(&body, &mut draw);
        }
    }
}

/// The physics-half API on the engine's `Node` - see the module doc.
pub trait PhysicsNodeExt {
    /// Attaches a physics half, builder-style. The node needs a `Physics`
    /// property for it to act on.
    fn add_physics_behavior<P: PhysicsBehavior>(self, physics_behavior: P) -> Self;
    /// Latest `T` this node's physics half published - `None` until it has.
    fn physics_state<T: Any>(&self) -> Option<&T>;
    /// Sets a latest-value input for the physics half (read there with
    /// `PhysicsCtx::input::<T>()`) - a newer `T` replaces the older one.
    fn set_physics_input<T: Any>(&mut self, input: T);
    /// Queues a one-shot event for the physics half (read there with
    /// `PhysicsCtx::events::<T>()`) - seen by exactly one fixed step.
    fn push_physics_event<T: Any>(&mut self, event: T);
}

impl PhysicsNodeExt for Node {
    fn add_physics_behavior<P: PhysicsBehavior>(mut self, physics_behavior: P) -> Self {
        if self.get_property::<PhysicsLink>().is_none() {
            self.set_property(PhysicsLink::default());
        }
        match self.get_behavior_mut::<PhysicsHost>() {
            Some(host) => {
                host.behaviors.push(Box::new(physics_behavior));
                self
            }
            None => self.with_behavior(PhysicsHost { behaviors: vec![Box::new(physics_behavior)] }),
        }
    }

    fn physics_state<T: Any>(&self) -> Option<&T> {
        self.get_property::<PhysicsLink>()?.states.get(&TypeId::of::<T>())?.downcast_ref::<T>()
    }

    fn set_physics_input<T: Any>(&mut self, input: T) {
        if let Some(link) = self.get_property_mut::<PhysicsLink>() {
            link.input.states.insert(TypeId::of::<T>(), Box::new(input));
        }
    }

    fn push_physics_event<T: Any>(&mut self, event: T) {
        if let Some(link) = self.get_property_mut::<PhysicsLink>() {
            link.input.events.push(Box::new(event));
        }
    }
}

impl PhysicsInput {
    /// Sets a latest-value input (see `PhysicsNodeExt::set_physics_input`).
    pub fn set_state<T: Any>(&mut self, state: T) {
        self.states.insert(TypeId::of::<T>(), Box::new(state));
    }

    /// Queues a one-shot event (see `PhysicsNodeExt::push_physics_event`).
    pub fn push_event<T: Any>(&mut self, event: T) {
        self.events.push(Box::new(event));
    }

    /// Folds a newer batch into this one - states overwrite, events append.
    fn merge(&mut self, newer: PhysicsInput) {
        self.states.extend(newer.states);
        self.events.extend(newer.events);
    }
}

/// A node's physics half run on a Rapier world of its own, without a scene
/// or an App - for tests that fly an aircraft's physics alone (see
/// prefabs::plane::flight_tests). Steps exactly as `PhysicsHost` does.
pub struct PhysicsNode {
    pub id: String,
    body: RigidBodyHandle,
    behaviors: Vec<Box<dyn PhysicsBehavior>>,
    input: PhysicsInput,
}

impl PhysicsNode {
    pub fn new(id: String, body: RigidBodyHandle, behaviors: Vec<Box<dyn PhysicsBehavior>>) -> Self {
        PhysicsNode { id, body, behaviors, input: PhysicsInput::default() }
    }

    pub fn receive_input(&mut self, input: PhysicsInput) {
        self.input.merge(input);
    }

    /// Every behavior's `fixed_update` for one step, then the events are
    /// used up.
    pub fn fixed_update(&mut self, bodies: &mut RigidBodySet, colliders: &mut ColliderSet, narrow_phase: &NarrowPhase, query: &QueryPipeline, dt: f32) {
        let mut ctx = PhysicsCtx { body: self.body, bodies, colliders, narrow_phase, query, input: &self.input };
        for behavior in self.behaviors.iter_mut() {
            behavior.fixed_update(&mut ctx, dt);
        }
        self.input.events.clear();
    }

    pub fn publish(&self) -> Vec<PhysicsPayload> {
        self.behaviors.iter().filter_map(|behavior| behavior.publish()).collect()
    }
}
