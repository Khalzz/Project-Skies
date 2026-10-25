use super::transform::Transform3D;

/// How a child node's position is given - see `Parent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Space {
    /// An offset from the parent's origin, in its frame: the child moves
    /// (and, unless told otherwise, turns) with it.
    #[default]
    Local,
    /// A place in the world, disconnected from the parent: the child stays
    /// where it is whatever the parent does. It's still the parent's child
    /// in the hierarchy, and goes when the parent does.
    Global,
}

/// A node's place under another node - put on for you by `Node::with_child`
/// (the parent's id, the space and whether it turns with the parent come
/// from there), so it's rarely built by hand. Nodes are still kept flat by
/// id (see `SceneNodes`); this is what makes one a child.
///
/// The node's own `Transform3D` is always where it is in the world - every
/// system (renderer, particles, cameras) reads that. For a `Local` child,
/// `local` is the offset from the parent, and `SceneNodes::resolve_hierarchy`
/// writes the world transform from it every frame; for a `Global` child,
/// `local` is unused and its `Transform3D` is the one to move.
///
/// A child with a `Physics` body isn't moved by its parent - the physics
/// does that.
#[derive(Debug, Clone)]
pub struct Parent {
    pub id: String,
    pub space: Space,
    pub local: Transform3D,
    /// A `Local` child turns with the parent - off: it only follows its
    /// position, keeping its own rotation (a chase camera that shouldn't
    /// roll with the plane).
    pub inherit_rotation: bool,
}
