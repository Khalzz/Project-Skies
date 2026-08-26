use std::any::{Any, TypeId};
use std::collections::HashMap;

use crate::app::App;

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
}

impl Node {
    pub fn new(id: impl Into<String>) -> Self {
        Node {
            id: id.into(),
            properties: HashMap::new(),
            behaviors: Vec::new(),
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

    /// Runs `on_spawn` on every attached behavior. Called once by
    /// `SceneNodes::spawn`, right after the node is actually in the scene -
    /// not part of construction, since a behavior's `on_spawn` may want to see
    /// properties a later `add_property` call added after it was attached.
    pub(crate) fn run_on_spawn(&mut self, app: &mut App) {
        // Behaviors are taken out of self before calling into them - a
        // behavior stored inside self.behaviors can't also receive &mut self,
        // that would alias. Same pattern in run_update/run_fixed_update below.
        let mut behaviors = std::mem::take(&mut self.behaviors);
        for behavior in behaviors.iter_mut() {
            behavior.on_spawn(self, app);
        }
        self.behaviors = behaviors;
    }

    pub(crate) fn run_update(&mut self, app: &mut App, dt: f32) {
        let mut behaviors = std::mem::take(&mut self.behaviors);
        for behavior in behaviors.iter_mut() {
            behavior.update(self, app, dt);
        }
        self.behaviors = behaviors;
    }

    pub(crate) fn run_fixed_update(&mut self, app: &mut App, dt: f32) {
        let mut behaviors = std::mem::take(&mut self.behaviors);
        for behavior in behaviors.iter_mut() {
            behavior.fixed_update(self, app, dt);
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
