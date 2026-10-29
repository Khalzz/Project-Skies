use std::any::{Any, TypeId};
use crate::utils::type_map::TypeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::app::App;

use crate::scene_manager::behavior::Behavior;
use crate::scene_manager::scene_access::SceneAccess;
use super::properties::{Space, Transform2D, Transform3D};

/// `get_property` straight on a node that may not be there - what
/// `get_node` hands back - so one `if let` covers both:
///
///     if let Some(transform) = scene.get_node("enemy").get_property::<Transform3D>() { ... }
///
/// None if there's no such node, or it has no such property.
pub trait NodeLookup<'a> {
    fn get_property<T: 'static>(self) -> Option<&'a T>;
}

impl<'a> NodeLookup<'a> for Option<&'a Node> {
    fn get_property<T: 'static>(self) -> Option<&'a T> {
        self?.get_property::<T>()
    }
}

/// `NodeLookup`, to change: `scene.get_node_mut("enemy").get_property_mut::<Shape2D>()`.
pub trait NodeLookupMut<'a> {
    fn get_property_mut<T: 'static>(self) -> Option<&'a mut T>;
}

impl<'a> NodeLookupMut<'a> for Option<&'a mut Node> {
    fn get_property_mut<T: 'static>(self) -> Option<&'a mut T> {
        self?.get_property_mut::<T>()
    }
}

/// Hands out `Node::property_order`'s stamps - one count for every node.
static NEXT_PROPERTY_ORDER: AtomicU64 = AtomicU64::new(0);

/// A single entity in a scene: an id, an open set of typed properties
/// (position, model, whatever a future property type needs), and an open set
/// of attached `Behavior`s (custom per-element logic/state, e.g. a `Player`
/// struct with its own `lives` field).
///
/// Properties are erased via `Box<dyn Any>` keyed by `TypeId` rather than a
/// closed enum, specifically so a new property type never requires editing
/// this file - define a plain struct anywhere and `with_property`/
/// `get_property_mut` work with it immediately.
pub struct Node {
    pub id: String,
    properties: TypeMap<Box<dyn Any>>,
    // When each property was first put on - see `property_order`.
    property_order: TypeMap<u64>,
    behaviors: Vec<Box<dyn Behavior>>,
    // Nodes given with `with_child`, waiting to be spawned right after this
    // one (see `Scene::spawn_node`) - empty once it's in the scene.
    children: Vec<Node>,
    // How this node is placed under the parent it's given to with
    // `with_child` - see `global`/`follow_position_only`.
    space: Space,
    inherit_rotation: bool,
}

impl Node {
    pub fn new(id: impl Into<String>) -> Self {
        Node {
            id: id.into(),
            properties: TypeMap::default(),
            property_order: TypeMap::default(),
            behaviors: Vec::new(),
            children: Vec::new(),
            space: Space::Local,
            inherit_rotation: true,
        }
    }

    /// Adds `child` under this node, builder-style - spawned right after it,
    /// with a `Parent` property put on for you (this node's id, `child`'s
    /// space and whether it turns with this node). By default `child`'s
    /// `Transform3D` is `local`: an offset from this node's origin, moving
    /// and turning with it. Mark it `.global()` for a place in the world
    /// that stays put whatever this node does. Children can have children.
    /// Its id is put under this node's when it spawns: "left_wing" under
    /// "plane" becomes "plane/left_wing" - `scene.child(&id, "left_wing")`
    /// finds it.
    pub fn with_child(mut self, child: Node) -> Self {
        self.children.push(child);
        self
    }

    /// Given to a parent with `with_child`: its `Transform3D` is an offset
    /// from the parent's origin, and it moves with the parent - the default.
    pub fn local(mut self) -> Self {
        self.space = Space::Local;
        self
    }

    /// Given to a parent with `with_child`: its `Transform3D` is a place in
    /// the world, disconnected from the parent - it stays there whatever the
    /// parent does.
    pub fn global(mut self) -> Self {
        self.space = Space::Global;
        self
    }

    /// Given to a parent with `with_child` as `local`: follows the parent's
    /// position only, keeping its own rotation - its offset is along the
    /// world's axes, not turned with the parent.
    pub fn follow_position_only(mut self) -> Self {
        self.inherit_rotation = false;
        self
    }

    /// The children given with `with_child`, and how each wants placing -
    /// taken once, when the node spawns.
    pub(crate) fn take_children(&mut self) -> Vec<Node> {
        std::mem::take(&mut self.children)
    }

    pub(crate) fn placement(&self) -> (Space, bool) {
        (self.space, self.inherit_rotation)
    }

    /// Puts `property` on (or replaces it) in place - `with_property` for a
    /// node that's already built.
    pub fn set_property<T: 'static>(&mut self, property: T) {
        // In the 3D world or on the 2D plane, never both - one transform
        // replaces the other.
        if TypeId::of::<T>() == TypeId::of::<Transform3D>() {
            self.remove_property::<Transform2D>();
        } else if TypeId::of::<T>() == TypeId::of::<Transform2D>() {
            self.remove_property::<Transform3D>();
        }
        self.property_order.entry(TypeId::of::<T>()).or_insert_with(|| NEXT_PROPERTY_ORDER.fetch_add(1, Ordering::Relaxed));
        self.properties.insert(TypeId::of::<T>(), Box::new(property));
    }

    /// When the property of type `T` was first put on - a later one is
    /// higher, on whichever node. Replacing it keeps its place. What decides
    /// which of a 2D node's visuals is in front (see `Shape2D`).
    pub fn property_order<T: 'static>(&self) -> Option<u64> {
        self.property_order.get(&TypeId::of::<T>()).copied()
    }

    /// Where it is in the 3D world - its `Transform3D`; None for a 2D node.
    /// Short for `get_property::<Transform3D>()`.
    pub fn transform_3d(&self) -> Option<&Transform3D> {
        self.get_property::<Transform3D>()
    }

    pub fn transform_3d_mut(&mut self) -> Option<&mut Transform3D> {
        self.get_property_mut::<Transform3D>()
    }

    /// Where it is on the 2D plane - its `Transform2D`; None for a 3D node.
    /// Short for `get_property::<Transform2D>()`.
    pub fn transform_2d(&self) -> Option<&Transform2D> {
        self.get_property::<Transform2D>()
    }

    pub fn transform_2d_mut(&mut self) -> Option<&mut Transform2D> {
        self.get_property_mut::<Transform2D>()
    }

    /// Takes the property of type `T` off, if it has one.
    pub fn remove_property<T: 'static>(&mut self) -> Option<T> {
        self.property_order.remove(&TypeId::of::<T>());
        self.properties.remove(&TypeId::of::<T>())?.downcast::<T>().ok().map(|boxed| *boxed)
    }

    /// Attaches a property, builder-style (chains like `ui_node.rs`'s
    /// `.set_size(...).set_position(...)`). A node holds at most one property
    /// of a given type - a second `with_property::<T>` call replaces the first.
    pub fn with_property<T: 'static>(mut self, property: T) -> Self {
        self.set_property(property);
        self
    }

    pub fn get_property_mut<T: 'static>(&mut self) -> Option<&mut T> {
        self.properties.get_mut(&TypeId::of::<T>())?.downcast_mut::<T>()
    }

    pub fn get_property<T: 'static>(&self) -> Option<&T> {
        self.properties.get(&TypeId::of::<T>())?.downcast_ref::<T>()
    }

    /// Attaches a behavior, builder-style - same chaining as `with_property`.
    pub fn with_behavior<B: Behavior + 'static>(mut self, behavior: B) -> Self {
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

    /// Runs `on_spawn` on every attached behavior. Called once by
    /// `SceneNodes::spawn`, right after the node is actually in the scene -
    /// not part of construction, since a behavior's `on_spawn` may want to see
    /// properties a later `with_property` call added after it was attached.
    pub(crate) fn run_on_spawn(&mut self, scene: &mut SceneAccess, app: &mut App) {
        // Behaviors are taken out of self before calling into them - a
        // behavior stored inside self.behaviors can't also receive &mut self,
        // that would alias. Same pattern in run_update/run_fixed_update below.
        let mut behaviors = std::mem::take(&mut self.behaviors);
        for behavior in behaviors.iter_mut() {
            behavior.on_spawn(self, scene, app);
        }
        self.behaviors = behaviors;
    }

    pub(crate) fn run_update(&mut self, scene: &mut SceneAccess, app: &mut App) {
        let mut behaviors = std::mem::take(&mut self.behaviors);
        for behavior in behaviors.iter_mut() {
            behavior.update(self, scene, app);
        }
        self.behaviors = behaviors;
    }

    pub(crate) fn run_fixed_update(&mut self, scene: &mut SceneAccess, app: &mut App) {
        let mut behaviors = std::mem::take(&mut self.behaviors);
        for behavior in behaviors.iter_mut() {
            behavior.fixed_update(self, scene, app);
        }
        self.behaviors = behaviors;
    }

    /// Whether it has any behaviors - one without is skipped when they run.
    pub(crate) fn has_behaviors(&self) -> bool {
        !self.behaviors.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Health(u8);

    #[derive(Debug, PartialEq)]
    struct Name(&'static str);

    #[test]
    fn a_maybe_node_reads_its_property_in_one_step() {
        let mut node = Node::new("player").with_property(Health(3));
        assert_eq!(Some(&node).get_property::<Health>(), Some(&Health(3)));
        assert_eq!(Some(&node).get_property::<Name>(), None);
        assert_eq!(None::<&Node>.get_property::<Health>(), None);
        Some(&mut node).get_property_mut::<Health>().unwrap().0 = 1;
        assert_eq!(node.get_property::<Health>().unwrap().0, 1);
    }

    #[test]
    fn a_replaced_property_keeps_its_place_in_the_order() {
        let mut node = Node::new("test").with_property(Health(3)).with_property(Name("a"));
        let (health, name) = (node.property_order::<Health>().unwrap(), node.property_order::<Name>().unwrap());
        assert!(health < name);
        node.set_property(Health(1));
        assert_eq!(node.property_order::<Health>(), Some(health));
        node.remove_property::<Health>();
        node.set_property(Health(2));
        assert!(node.property_order::<Health>().unwrap() > name, "put back on: last");
    }

    #[test]
    fn with_property_then_get_property_mut_roundtrips() {
        let mut node = Node::new("test").with_property(Health(3));
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
    fn second_with_property_of_same_type_replaces_the_first() {
        let mut node = Node::new("test").with_property(Health(3)).with_property(Health(9));
        assert_eq!(node.get_property::<Health>(), Some(&Health(9)));
    }

    #[test]
    fn a_node_is_3d_or_2d_never_both() {
        let mut node = Node::new("test").with_property(Transform3D::default());
        assert!(node.transform_3d().is_some());
        assert!(node.transform_2d().is_none());

        node.get_property_mut::<Transform3D>().unwrap().position.y = 5.0;
        assert_eq!(node.transform_3d().unwrap().position.y, 5.0);

        // A 2D transform replaces the 3D one - and back.
        node.set_property(Transform2D::default());
        assert!(node.get_property::<Transform3D>().is_none());
        assert!(node.get_property::<Transform2D>().is_some());
        node.set_property(Transform3D::default());
        assert!(node.get_property::<Transform2D>().is_none());
    }
}
