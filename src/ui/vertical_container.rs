use glyphon::{FontSystem, TextArea};

use crate::{app::Size, rendering::vertex::VertexUi};

use super::{ui_node::{UiNode, UiNodeParameters}, ui_transform::Rect};

/// # Vertical Container
/// This struct will be designed for "rendering listed data" like for example, subtitles in a certain order (in this case on a vertical one)
/// while respecting elements  like margin or separation between all of them.

pub struct VecticalContainerData {
    pub separation: f32,
    pub children: Vec<UiNode>
}

impl VecticalContainerData {
    pub fn new(separation: f32, children: Vec<UiNode>) -> Self {
        Self {
            separation,
            children,
        }
    }

    // this function will do the positioning of all the elements in the screen based on their separation
    pub fn ui_node_data_creation(&mut self, size: &Size, vertices: &mut Vec<VertexUi>, vertices_slice: &[VertexUi; 4], indices: &mut Vec<u16>, indices_slice: &[u16; 6], num_vertices: &mut u16, parent_rect: &mut Rect) -> (Vec<TextArea>, u16, u32) {
        let mut base_position = parent_rect.top as f32 + self.separation;
        let mut height = 0.0;

        

        let mut text_areas: Vec<TextArea> = Vec::new();
        let mut vertices_to_add = 0;
        let mut indices_to_add = 0;

        for child in &mut self.children {
            // set the sizxe of the buffer

            // child.transform.rect.left = parent_rect.left;
            // child.transform.rect.right = parent_rect.right;
            child.transform.width = (parent_rect.right - parent_rect.left) as f32;
            // child.transform.rect.left = parent_rect.left;
            // child.transform.rect.right = parent_rect.right;
            child.transform.y = base_position;
            // child.transform.x = 1000.0;
            child.transform.apply_transformation();
            base_position += child.transform.height + self.separation;


            match &mut child.content {
                super::ui_node::UiNodeContent::Text(label) => {
                    // label.buffer.set_size(font_system, child.transform.width, child.transform.height);

                    let (text_area, added_vertices, added_indices) = label.ui_node_data_creation(size, vertices, &vertices_slice, indices, &indices_slice, num_vertices, &child.transform.rect);

                    text_areas.push(text_area);
                    vertices_to_add += added_vertices;
                    indices_to_add += added_indices;
                },
                super::ui_node::UiNodeContent::VerticalContainer(vectical_container_data) => {
                    
                },
            }
        }

        parent_rect.bottom = base_position as u32;

        (text_areas, vertices_to_add, indices_to_add)
    }
}