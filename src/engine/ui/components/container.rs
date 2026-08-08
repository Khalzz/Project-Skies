//! The `UiNodeContent::Container` payload - a node that lays out other nodes rather
//! than rendering content of its own. Most of what makes a container behave (size,
//! padding, orientation, background/border/rounding, hover, click, its children)
//! actually lives on the owning `UiNode`/`Style` - this struct only holds what's
//! genuinely container-specific.
//!
//! - `Container::new(gap, children)` - `gap` is the space between stacked
//!   children; `children` is the ordered `(id, UiNode)` list built up via
//!   `UiNode::set_child(...)`.
//! - `ui_node_data_creation(...)` - appends this container's own background/border
//!   quad (already computed by the caller, see `UiNode::compute_vertices`) into the
//!   shared vertex/index buffers. Doesn't touch children - `UiNode::
//!   node_content_preparation`'s Container branch lays those out and recurses into
//!   them itself.

use crate::app::Size;
use crate::engine::rendering::vertex::VertexUi;
use crate::engine::ui::ui_node::UiNode;

pub struct Container {
    pub gap: f32,
    // Vec, not HashMap - children render/lay out in insertion order (HashMap has no
    // iteration-order guarantee at all, which made .set_child(...) render in a
    // different order than it was called in).
    pub children: Vec<(String, UiNode)>,
}

impl Container {
    pub fn new(gap: f32, children: Vec<(String, UiNode)>) -> Self {
        Self { gap, children }
    }

    pub fn ui_node_data_creation(&self, _size: &Size, vertices: &mut Vec<VertexUi>, vertices_slice: &[VertexUi; 4], indices: &mut Vec<u16>, indices_slice: &[u16; 6]) -> (u16, u32) {
        vertices.extend_from_slice(vertices_slice);
        indices.extend_from_slice(indices_slice);

        (vertices_slice.len() as u16, UiNode::NUM_INDICES)
    }
}
