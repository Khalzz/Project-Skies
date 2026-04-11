use std::collections::HashMap;

use crate::app::Size;
use crate::rendering::vertex::VertexUi;
use crate::ui::ui_node::UiNode;

pub struct Container {
    pub margin: f32,
    pub gap: f32,
    pub children: HashMap<String, UiNode>,
}

impl Container {
    pub fn new(margin: f32, gap: f32, children: HashMap<String, UiNode>) -> Self {
        Self { margin, gap, children }
    }

    pub fn ui_node_data_creation(&self, _size: &Size, vertices: &mut Vec<VertexUi>, vertices_slice: &[VertexUi; 4], indices: &mut Vec<u16>, indices_slice: &[u16; 6]) -> (u16, u32) {
        vertices.extend_from_slice(vertices_slice);
        indices.extend_from_slice(indices_slice);

        (vertices_slice.len() as u16, UiNode::NUM_INDICES)
    }
}
