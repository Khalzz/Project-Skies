use std::any::{Any, TypeId};
use std::collections::HashMap;

use crate::app::App;
use crate::engine::physics::physics_behavior::{PhysicsBehavior, PhysicsInput, PhysicsPayload};
use crate::engine::physics::physics_handler::RenderMessage;
use crate::engine::rendering::camera::handler::SceneCameras;

use super::behavior::Behavior;

/// A single entity in a scene: an id, an open set of typed properties
/// (position, model, whatever a future property type needs), and an open set
/// of attached `Behavior`s (custom per-element logic/state, e.g. a `Player`
/// struct with its own `lives` field).
///
/// Properties are erased via `Box<dyn Any>` keyed by `TypeId` rather than a
/// closed enum, specifically so a new property type never requires editing
/// this file - define a plain struct anywhere and `add_property`/
/// `get_property_mut` work with it immediately.
pub struct Node {
    pub id: String,
    properties: HashMap<TypeId, Box<dyn Any>>,
    behaviors: Vec<Box<dyn Behavior>>,
    // This node's physics-thread half (see `PhysicsBehavior`) - only held
    // here until the scene's physics starts, then moved onto that thread for
    // good (see `take_physics_behaviors`).
    physics_behaviors: Vec<Box<dyn PhysicsBehavior>>,
    // Main → physics messages written this frame, sent once per frame.
    physics_input: PhysicsInput,
    // Latest state each physics behavior published, keyed by its concrete type.
    physics_states: HashMap<TypeId, PhysicsPayload>,
}

impl Node {
    pub fn new(id: impl Into<String>) -> Self {
        Node {
            id: id.into(),
            properties: HashMap::new(),
            behaviors: Vec::new(),
            physics_behaviors: Vec::new(),
            physics_input: PhysicsInput::default(),
            physics_states: HashMap::new(),
        }
    }

    /// Attaches a property, builder-style (chains like `ui_node.rs`'s
    /// `.set_size(...).set_position(...)`). A node holds at most one property
    /// of a given type - a second `add_property::<T>` call replaces the first.
    pub fn add_property<T: 'static>(mut self, property: T) -> Self {
        self.properties.insert(TypeId::of::<T>(), Box::new(property));
        self
    }

    pub fn get_property_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.properties.get_mut(&TypeId::of::<T>())?.downcast_mut::<T>()
    }

    pub fn get_property<T: 'static>(&self) -> Option<&T> {
        self.properties.get(&TypeId::of::<T>())?.downcast_ref::<T>()
    }

    /// Attaches a behavior, builder-style - same chaining as `add_property`.
    pub fn add_behavior<B: Behavior + 'static>(mut self, behavior: B) -> Self {
        self.behaviors.push(Box::new(behavior));
        self
    }

    /// Finds this node's attached behavior of concrete type `B`, if any -
    /// same idea as `get_property`/`get_property_mut`, just over `behaviors`
    /// instead of `properties` (see `Behavior::as_any`/`as_any_mut`'s own doc
    /// comment for why this exists: a `SceneBehaviour` can reach into a
    /// specific node's behavior this way, for data a `Behavior` itself has no
    /// way to reach on its own).
    pub fn get_behavior<B: Behavior + 'static>(&self) -> Option<&B> {
        self.behaviors.iter().find_map(|behavior| behavior.as_any().downcast_ref::<B>())
    }

    pub fn get_behavior_mut<B: Behavior + 'static>(&mut self) -> Option<&mut B> {
        self.behaviors.iter_mut().find_map(|behavior| behavior.as_any_mut().downcast_mut::<B>())
    }

    /// Attaches this node's physics-thread half, builder-style. Only picked up
    /// if the node is spawned *before* the scene's physics starts (same as a
    /// `Physics` property's rigidbody) - and it needs that rigidbody to act
    /// on, so the node must also carry a `Physics` property.
    pub fn add_physics_behavior<P: PhysicsBehavior + 'static>(mut self, physics_behavior: P) -> Self {
        self.physics_behaviors.push(Box::new(physics_behavior));
        self
    }

    /// Latest `T` this node's physics half published (see
    /// `PhysicsBehavior::publish`) - a copy from the last frame the physics
    /// thread reported back, `None` until it has.
    pub fn physics_state<T: Any>(&self) -> Option<&T> {
        self.physics_states.get(&TypeId::of::<T>())?.downcast_ref::<T>()
    }

    /// Sets a latest-value input for this node's physics half (read there via
    /// `PhysicsCtx::input::<T>()`) - a newer `T` replaces the older one.
    pub fn set_physics_input<T: Any + Send>(&mut self, input: T) {
        self.physics_input.set_state(input);
    }

    /// Queues a one-shot event for this node's physics half (read there via
    /// `PhysicsCtx::events::<T>()`) - seen by exactly one fixed step.
    pub fn push_physics_event<T: Any + Send>(&mut self, event: T) {
        self.physics_input.push_event(event);
    }

    pub(crate) fn take_physics_behaviors(&mut self) -> Vec<Box<dyn PhysicsBehavior>> {
        std::mem::take(&mut self.physics_behaviors)
    }

    pub(crate) fn take_physics_input(&mut self) -> Option<PhysicsInput> {
        if self.physics_input.is_empty() {
            return None;
        }
        Some(std::mem::take(&mut self.physics_input))
    }

    pub(crate) fn set_physics_states(&mut self, states: Vec<PhysicsPayload>) {
        for state in states {
            // `&*state`, not `state` - the Box's own TypeId would be
            // `Box<dyn Any + Send>`, not the type inside it.
            self.physics_states.insert(Any::type_id(&*state), state);
        }
    }

    /// Runs `on_spawn` on every attached behavior. Called once by
    /// `SceneNodes::spawn`, right after the node is actually in the scene -
    /// not part of construction, since a behavior's `on_spawn` may want to see
    /// properties a later `add_property` call added after it was attached.
    pub(crate) fn run_on_spawn(&mut self, cameras: &mut SceneCameras, app: &mut App) {
        // Behaviors are taken out of self before calling into them - a
        // behavior stored inside self.behaviors can't also receive &mut self,
        // that would alias. Same pattern in run_update/run_fixed_update below.
        let mut behaviors = std::mem::take(&mut self.behaviors);
        for behavior in behaviors.iter_mut() {
            behavior.on_spawn(self, cameras, app);
        }
        self.behaviors = behaviors;
    }

    pub(crate) fn run_update(&mut self, cameras: &mut SceneCameras, app: &mut App, dt: f32) {
        let mut behaviors = std::mem::take(&mut self.behaviors);
        for behavior in behaviors.iter_mut() {
            behavior.update(self, cameras, app, dt);
        }
        self.behaviors = behaviors;
    }

    pub(crate) fn run_fixed_update(&mut self, cameras: &mut SceneCameras, app: &mut App, dt: f32, physics_message: Option<&RenderMessage>) {
        let mut behaviors = std::mem::take(&mut self.behaviors);
        for behavior in behaviors.iter_mut() {
            behavior.fixed_update(self, cameras, app, dt, physics_message);
        }
        self.behaviors = behaviors;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Health(u8);

    #[test]
    fn add_property_then_get_property_mut_roundtrips() {
        let mut node = Node::new("test").add_property(Health(3));
        assert_eq!(node.get_property::<Health>(), Some(&Health(3)));

        node.get_property_mut::<Health>().unwrap().0 = 1;
        assert_eq!(node.get_property::<Health>(), Some(&Health(1)));
    }

    #[test]
    fn missing_property_is_none() {
        let mut node = Node::new("test");
        assert!(node.get_property_mut::<Health>().is_none());
    }

    #[test]
    fn second_add_property_of_same_type_replaces_the_first() {
        let mut node = Node::new("test").add_property(Health(3)).add_property(Health(9));
        assert_eq!(node.get_property::<Health>(), Some(&Health(9)));
    }
}
