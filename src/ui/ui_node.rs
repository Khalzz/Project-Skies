use std::collections::{HashMap, HashSet};

use glyphon::{cosmic_text::Align, Color, FontSystem, TextArea};
use nalgebra::{base, vector};
use rapier3d::parry::utils::hashmap;
use crate::{app::{App, Size}, rendering::vertex::VertexUi};
use super::{label::{self, Label}, ui_transform::{Rect, UiTransform}, vertical_container::{self, VerticalContainerData}};

pub enum Alignment {
    Start,
    Center,
    Custom,

    VerticalAlignment(f32)
    
}

// We do this so the container node type can save vectors and hashmap values
pub enum ChildrenType {
    IndexedChildren(Vec<UiNode>),
    MappedChildren(HashMap<String, UiNode>)

}

pub enum UiNodeContent {
    Text(Label),
    VerticalContainer(VerticalContainerData)
}

/// This is for setting or passing info/data for the content of the UI node
pub enum UiNodeParameters<'a> {
    Text {
        text: &'a str,
        color: Color,
        align: Align,
        font_size: f32
    },
    VerticalContainerData {
        margin: f32, // separation between container and content
        separation: f32, // separation between elements in the content
        children: ChildrenType
    }
}

pub struct UiNodeRenderizableData<'a> {
    text_area: TextArea<'a>,
    num_vertices: u16,
}

/// # Visibility
/// 
/// This struct will dedicate to visibility element of the ui node itself and his parameters are:
/// - **Color**: The color of the inner object (for example if it contains a text, the text will have this color)
/// - **Background Color**: the color of the background will be setted as this one
/// - **border Color**: the color of the border setted on the object
pub struct Visibility {
    pub background_color: [f32; 4],
    pub border_color: [f32; 4],
}

impl Visibility {
    pub fn new(background_color: [f32; 4], border_color: [f32; 4]) -> Self {
        Self { background_color, border_color }
    }
}

/// # UI Node
/// 
/// A ui node is the base of **a UI element that can be rendered on screen** this can be:
/// - A button
/// - A label
/// - A text input
/// - Etc...
/// 
/// The main way it works is by setting the type in "UiNodeContent" we define what it can be used, and then
/// is accesable from it.
/// 
/// The Ui Node has this properties:
/// - **transform**: sets the position, size and rotation of the object
/// - **visibility**: sets the visibility configuration of a object in screen, including the main color, background color, border color, etc...
/// - **content**: sets his content, this can be a label, image or other
pub struct UiNode {
    pub transform: UiTransform,
    pub visibility: Visibility,
    pub content: UiNodeContent,
    pub parent_transform: Option<UiTransform>
}

impl UiNode {
    pub const NUM_INDICES: u32 = 6;

    pub fn new(mut transform: UiTransform, visibility: Visibility, content_data: UiNodeParameters, app: &mut App, parent: Option<UiTransform>) -> Self {
        // This was added but never implemented, check why ❌❌❌
        let base_position = match parent {
            Some(parent_base) => {
                (parent_base.x, parent_base.y);
            },
            None => {
                (0.0, 0.0);
            },
        };

        let content = match content_data {
            UiNodeParameters::Text { text, color, align, font_size } => UiNodeContent::Text(Label::new(&mut app.ui.text.font_system, text, transform.clone(), color, align, font_size)),
            UiNodeParameters::VerticalContainerData { separation, children, margin } => UiNodeContent::VerticalContainer(VerticalContainerData::new(margin, separation, children)),
        };
        
        Self {
            transform,
            visibility,
            content,
            parent_transform: None
        }
    }

    /// # Node content preparation
    /// This function will generate the render data based on the content of the "UiNode", in this way we can define stuff like handling a node as a container,
    /// as a simple object, or more.
    /// 
    /// Params:
    /// size: The size of the screen
    /// vertices: The vector of "VertexUi" that contains the data that will be setted for the each renderizable element

    pub fn node_content_preparation(&mut self, size: &Size, font_system: &mut FontSystem, vertices: &mut Vec<VertexUi>, indices: &mut Vec<u16>, num_vertices: &mut u16, num_indices: &mut u32) -> (Vec<TextArea>, u16, u32) {
        let mut text_areas: Vec<TextArea> = Vec::new();
        let vertices_to_add = 0;
        let indices_to_add = 0;
    
        let vertices_slice = self.vertices(size);
        let indice_slice = self.indices(*num_vertices);
    
        match &mut self.content {
            UiNodeContent::Text(label) => {
                label.text_area(&self.transform.rect);
                // label.realign(font_system);
                label.buffer.set_size( font_system, (self.transform.rect.right - self.transform.rect.left) as f32, (self.transform.rect.bottom - self.transform.rect.top) as f32,);
        
                let (text_area, added_vertices, added_indices) = label.ui_node_data_creation(size, vertices, &vertices_slice, indices, &indice_slice, &self.transform.rect);
                text_areas.push(text_area);
                *num_vertices += added_vertices;
                *num_indices += added_indices;
            },
            UiNodeContent::VerticalContainer(vertical_container) => {
                let mut base_position = self.transform.y + vertical_container.margin;
    
                // Render the base container itself
                let (container_vertices, container_indices) = vertical_container.ui_node_data_creation(size, vertices, &vertices_slice, indices, &indice_slice);
                *num_vertices += container_vertices;
                *num_indices += container_indices;

                match &mut vertical_container.children {
                    ChildrenType::IndexedChildren(vec) => {
                        for child in vec {
                            let (child_text_areas, child_vertices, child_indices) = Self::handle_children(&mut self.transform, child, vertical_container.margin, vertical_container.separation, &mut base_position, size, font_system, vertices, indices, num_vertices, num_indices);
                            text_areas.extend(child_text_areas);
                            *num_vertices += child_vertices;
                            *num_indices += child_indices;
                        }
                    },
                    ChildrenType::MappedChildren(hash_map) => {
                        for (_id, child) in hash_map {
                            let (child_text_areas, child_vertices, child_indices) = Self::handle_children(&mut self.transform, child, vertical_container.margin, vertical_container.separation, &mut base_position, size, font_system, vertices, indices, num_vertices, num_indices);
                            text_areas.extend(child_text_areas);
                            *num_vertices += child_vertices;
                            *num_indices += child_indices;
                        }
                    },
                }
            },
        }
    
        (text_areas, vertices_to_add, indices_to_add)
    }

    fn handle_children<'a>(transform: &mut UiTransform, child: &'a mut UiNode, margin: f32, separation: f32, base_position: &mut f32, size: &Size, font_system: &mut FontSystem, vertices: &mut Vec<VertexUi>, indices: &mut Vec<u16>, num_vertices: &mut u16, num_indices: &mut u32) -> (Vec<TextArea<'a>>, u16, u32) {
        // Reset child's transform based on parent's properties
        child.transform.width = ((transform.rect.right - margin as u32) - (transform.rect.left + margin as u32)) as f32;
        child.transform.x = transform.x + margin; // Align with parent's x
        child.transform.y = *base_position; // Set y position based on parent's layout
        *base_position += child.transform.height + separation; // Update base position for next child

        transform.rect.bottom = (child.transform.y + child.transform.height) as u32 + separation as u32;

        // Apply transformations specific to this child
        child.transform.apply_transformation();

        child.node_content_preparation(size, font_system, vertices, indices, num_vertices, num_indices)
        
    }

    /// # Ui node render data getter
    /// 
    /// This function will mainly get as a parameter information about the renderizable element, mainly a list of vertex and indices
    /// 
    /// ## Returns:
    /// A amount of values ordered as (text area, num_vertices, num indices)
    pub fn indices(&self, base: u16) -> [u16; 6] {
        [base, 1 + base, 2 + base, base, 2 + base, 3 + base]
    }

    pub fn vertices(&mut self, screen_size: &Size) -> [VertexUi; 4] {
        let top = 1.0 - (self.transform.rect.top as f32 / (screen_size.height as f32 / 2.0));
        let left = (self.transform.rect.left as f32 / (screen_size.width as f32 / 2.0)) - 1.0;
        let bottom = 1.0 - (self.transform.rect.bottom as f32 / (screen_size.height as f32 / 2.0));
        let right = (self.transform.rect.right as f32 / (screen_size.width as f32 / 2.0)) - 1.0;

        let rect = [
            self.transform.rect.top as f32,
            self.transform.rect.left as f32,
            self.transform.rect.bottom as f32,
            self.transform.rect.right as f32,
        ];

        let left_top = vector![left, top, 0.0];
        let left_bottom = vector![left, bottom, 0.0];
        let right_top = vector![right, top, 0.0];
        let right_bottom = vector![right, bottom, 0.0];

        [
            VertexUi { 
                position: left_top.into(), 
                color: self.visibility.background_color, 
                rect,
                border_color: self.visibility.border_color, 
            },
            VertexUi { 
                position: left_bottom.into(), 
                color: self.visibility.background_color, 
                rect, 
                border_color: self.visibility.border_color, 
            },
            VertexUi { position: right_bottom.into(), 
                color: self.visibility.background_color, 
                rect, 
                border_color: self.visibility.border_color, 
            },
            VertexUi { position: right_top.into(), 
                color: self.visibility.background_color, 
                rect, 
                border_color: self.visibility.border_color, 
            },
        ]
    }

    pub fn add_children(&mut self, id: String, ui_node: UiNode) {
        match &mut self.content {
            UiNodeContent::VerticalContainer(vertical_container_data) => {
                match &mut vertical_container_data.children {
                    ChildrenType::IndexedChildren(vec) => {
                        vec.push(ui_node);
                    },
                    ChildrenType::MappedChildren(hash_map) => {
                        hash_map.insert(id, ui_node);
                    },
                }
            },
            _ => {},
        }
    }

    pub fn get_container_hashed(&mut self) -> Result<&mut HashMap<String, UiNode>, String> {
        match &mut self.content {
            UiNodeContent::Text(label) => Err("This UiNode is not a container".to_owned()),
            UiNodeContent::VerticalContainer(vertical_container_data) => {
                match &mut vertical_container_data.children {
                    ChildrenType::IndexedChildren(vec) => Err("This UiNode is not a map".to_owned()),
                    ChildrenType::MappedChildren(hash_map) => Ok(hash_map),
                }
            },
        }
    }
}
