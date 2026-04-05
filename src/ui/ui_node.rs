use std::collections::{HashMap, HashSet};

use glyphon::{cosmic_text::Align, Color, FontSystem, TextArea};
use nalgebra::{base, vector};
use rapier3d::parry::utils::hashmap;
use crate::{app::{App, Size}, rendering::{ui::{Ui, UiRendering}, vertex::VertexUi}, utils::lerps::{lerp, lerp_u32}};
use super::{label::{self, Label}, ui_transform::{Rect, UiTransform}, vertical_container::{self, VerticalContainerData}};

use crate::ui::ui_structure;

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
}

impl UiNode {
    pub const NUM_INDICES: u32 = 6;

    pub fn new(transform: UiTransform, visibility: Visibility, content_data: UiNodeParameters, app: &mut App) -> Self {
        let content = match content_data {
            UiNodeParameters::Text { text, color, align, font_size } => UiNodeContent::Text(Label::new(&mut app.ui.text.font_system, text, transform.clone(), color, align, font_size)),
            UiNodeParameters::VerticalContainerData { separation, children, margin } => UiNodeContent::VerticalContainer(VerticalContainerData::new(margin, separation, children)),
        };
        
        Self {
            transform,
            visibility,
            content,
        }
    }

    /// # Node content preparation
    /// This function will generate the render data based on the content of the "UiNode", in this way we can define stuff like handling a node as a container,
    /// as a simple object, or more.
    /// 
    /// Params:
    /// size: The size of the screen
    /// vertices: The vector of "VertexUi" that contains the data that will be setted for the each renderizable element

    pub fn node_content_preparation(&mut self, size: &Size, ui: &mut UiRendering, font_system: &mut FontSystem, delta_time: f32) -> (Vec<TextArea>, u16, u32) {
        let mut text_areas: Vec<TextArea> = Vec::new();
        let vertices_to_add = 0;
        let indices_to_add = 0;

        let transform = &mut self.transform;
        let visibility = &self.visibility;
        let content = &mut self.content;
    
        match content {
            UiNodeContent::Text(label) => {
                let vertices_slice = Self::compute_vertices(transform, visibility, size);
                let indice_slice = Self::compute_indices(ui.num_vertices);
                label.text_area(&transform.rect);
                label.buffer.set_size(font_system, Some((transform.rect.right - transform.rect.left) as f32), Some((transform.rect.bottom - transform.rect.top) as f32));
        
                let (text_area, added_vertices, added_indices) = label.ui_node_data_creation(size, &mut ui.vertices, &vertices_slice, &mut ui.indices, &indice_slice, &transform.rect);
                text_areas.push(text_area);
                ui.num_vertices += added_vertices;
                ui.num_indices += added_indices;
            },
            UiNodeContent::VerticalContainer(vertical_container) => {
                let mut base_position = transform.y + vertical_container.margin;
                let mut end_bottom = transform.rect.bottom;
                let auto_width = transform.width == 0.0;
                let auto_height = transform.height == 0.0;
    
                if auto_width {
                    let mut max_child_width: f32 = 0.0;
                    match &vertical_container.children {
                        ChildrenType::IndexedChildren(vec) => {
                            for child in vec.iter() {
                                max_child_width = max_child_width.max(child.transform.width);
                            }
                        },
                        ChildrenType::MappedChildren(hash_map) => {
                            for (_id, child) in hash_map.iter() {
                                max_child_width = max_child_width.max(child.transform.width);
                            }
                        },
                    }
                    let total_width = max_child_width + vertical_container.margin * 2.0;
                    transform.width = total_width;
                    transform.rect.right = transform.rect.left + total_width;
                }

                let vertices_slice = Self::compute_vertices(transform, visibility, size);
                let indice_slice = Self::compute_indices(ui.num_vertices);
                let (container_vertices, container_indices) = vertical_container.ui_node_data_creation(size, &mut ui.vertices, &vertices_slice, &mut ui.indices, &indice_slice);
                ui.num_vertices += container_vertices;
                ui.num_indices += container_indices;

                match &mut vertical_container.children {
                    ChildrenType::IndexedChildren(vec) => {
                        for child in vec {
                            let (child_text_areas, child_vertices, child_indices) = Self::handle_children(transform, &mut end_bottom, child, vertical_container.margin, vertical_container.separation, &mut base_position, size, ui, delta_time, font_system);
                            text_areas.extend(child_text_areas);
                            ui.num_vertices += child_vertices;
                            ui.num_indices += child_indices;
                        }
                    },
                    ChildrenType::MappedChildren(hash_map) => {
                        for (_id, child) in hash_map {
                            let (child_text_areas, child_vertices, child_indices) = Self::handle_children(transform, &mut end_bottom, child, vertical_container.margin, vertical_container.separation, &mut base_position, size, ui, delta_time, font_system);
                            text_areas.extend(child_text_areas);
                            ui.num_vertices += child_vertices;
                            ui.num_indices += child_indices;
                        }
                    },
                }
                
                transform.rect.bottom = if transform.smooth_change {
                    lerp(transform.rect.bottom, end_bottom + vertical_container.margin, delta_time * 20.0)
                } else {
                    end_bottom + vertical_container.margin
                };
                if auto_height {
                    transform.height = transform.rect.bottom - transform.rect.top;
                }

            },
        }
    
        (text_areas, vertices_to_add, indices_to_add)
    }

    fn handle_children<'a>(transform: &mut UiTransform, end_bottom: &mut f32,  child: &'a mut UiNode, margin: f32, separation: f32, base_position: &mut f32, size: &Size, ui: &mut UiRendering, delta_time: f32, font_system: &mut FontSystem) -> (Vec<TextArea<'a>>, u16, u32) {
        // Reset child's transform based on parent's properties
        child.transform.width = ((transform.rect.right - margin) - (transform.rect.left + margin)) as f32;
        child.transform.x = transform.x + margin; // Align with parent's x
        child.transform.y = *base_position; // Set y position based on parent's layout
        *base_position += child.transform.height + separation; // Update base position for next child
        *end_bottom = (child.transform.y + child.transform.height) + separation;

        // Apply transformations specific to this child
        child.transform.apply_transformation();

        child.node_content_preparation(size, ui, font_system, delta_time)
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

    fn compute_indices(base: u16) -> [u16; 6] {
        [base, 1 + base, 2 + base, base, 2 + base, 3 + base]
    }

    fn compute_vertices(transform: &UiTransform, visibility: &Visibility, screen_size: &Size) -> [VertexUi; 4] {
        let top = 1.0 - (transform.rect.top as f32 / (screen_size.height as f32 / 2.0));
        let left = (transform.rect.left as f32 / (screen_size.width as f32 / 2.0)) - 1.0;
        let bottom = 1.0 - (transform.rect.bottom as f32 / (screen_size.height as f32 / 2.0));
        let right = (transform.rect.right as f32 / (screen_size.width as f32 / 2.0)) - 1.0;

        let rect = [
            transform.rect.top as f32,
            transform.rect.left as f32,
            transform.rect.bottom as f32,
            transform.rect.right as f32,
        ];

        let left_top = vector![left, top, 0.0];
        let left_bottom = vector![left, bottom, 0.0];
        let right_top = vector![right, top, 0.0];
        let right_bottom = vector![right, bottom, 0.0];

        [
            VertexUi { position: left_top.into(), color: visibility.background_color, rect, border_color: visibility.border_color },
            VertexUi { position: left_bottom.into(), color: visibility.background_color, rect, border_color: visibility.border_color },
            VertexUi { position: right_bottom.into(), color: visibility.background_color, rect, border_color: visibility.border_color },
            VertexUi { position: right_top.into(), color: visibility.background_color, rect, border_color: visibility.border_color },
        ]
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

    /// Create a label node from minimal parameters. Transform, visibility, and content
    /// are all derived automatically — no manual UiTransform/Visibility construction needed.
    pub fn label(
        font_system: &mut FontSystem,
        text: &str,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        font_size: f32,
        color: Color,
        align: Align,
        background_color: [f32; 4],
        border_color: [f32; 4],
    ) -> Self {
        let transform = UiTransform::new(x, y, height, width, 0.0, false);
        let visibility = Visibility::new(background_color, border_color);
        let content = UiNodeContent::Text(Label::new(font_system, text, transform.clone(), color, align, font_size));
        Self { transform, visibility, content }
    }

    /// Create a UiNode directly from a ron UiComponent definition.
    /// This is the single entry point for building UI from .ron data.
    pub fn from_component(
        component: &ui_structure::UiComponent,
        font_system: &mut FontSystem,
        screen_width: f32,
        screen_height: f32,
    ) -> Self {
        let width = component.transform.size.as_ref().map(|s| s.width).unwrap_or(0.0);
        let height = component.transform.size.as_ref().map(|s| s.height).unwrap_or(0.0);
        let auto_size = component.transform.size.is_none();

        let mut x = component.transform.position.x;
        let mut y = component.transform.position.y;

        if let Some(anchor) = &component.transform.anchor {
            for part in anchor.split(',').map(|s| s.trim()) {
                match part {
                    "center_x" => x = (screen_width / 2.0) - (width / 2.0) + component.transform.position.x,
                    "center_y" => y = (screen_height / 2.0) - (height / 2.0) + component.transform.position.y,
                    "center" => {
                        x = (screen_width / 2.0) - (width / 2.0) + component.transform.position.x;
                        y = (screen_height / 2.0) - (height / 2.0) + component.transform.position.y;
                    }
                    "bottom" => y = screen_height - height + component.transform.position.y,
                    "right" => x = screen_width - width + component.transform.position.x,
                    _ => {}
                }
            }
        }

        let transform = UiTransform::new(x, y, height, width, 0.0, auto_size);
        let mut visibility = Visibility::new([0.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 0.0]);

        let content = match &component.child {
            ui_structure::UiNode::Label(label_data) => {
                let color = Color::rgba(
                    (label_data.color[0] * 255.0) as u8,
                    (label_data.color[1] * 255.0) as u8,
                    (label_data.color[2] * 255.0) as u8,
                    (label_data.color[3] * 255.0) as u8,
                );
                let align = match label_data.alignment.as_deref() {
                    Some("Center") => Align::Center,
                    Some("Right") => Align::Right,
                    _ => Align::Left,
                };
                let bg = label_data.background_color.unwrap_or([0.0, 0.0, 0.0, 0.0]);
                let border = label_data.border_color.unwrap_or([0.0, 0.0, 0.0, 0.0]);
                visibility = Visibility::new(bg, border);
                UiNodeContent::Text(Label::new(
                    font_system,
                    &label_data.text,
                    transform.clone(),
                    color,
                    align,
                    label_data.font_size,
                ))
            }
            ui_structure::UiNode::VerticalContainer(container_data) => {
                let bg = container_data.background_color.unwrap_or([0.0, 0.0, 0.0, 0.0]);
                let border = container_data.border_color.unwrap_or([0.0, 0.0, 0.0, 0.0]);
                visibility = Visibility::new(bg, border);
                let margin = container_data.margin.unwrap_or(0.0);
                let separation = container_data.separation.unwrap_or(0.0);

                let children = if let Some(ron_children) = &container_data.children {
                    let mut map = HashMap::new();
                    for (child_id, child_component) in ron_children {
                        map.insert(
                            child_id.clone(),
                            UiNode::from_component(child_component, font_system, screen_width, screen_height),
                        );
                    }
                    ChildrenType::MappedChildren(map)
                } else {
                    ChildrenType::IndexedChildren(vec![])
                };

                UiNodeContent::VerticalContainer(VerticalContainerData::new(margin, separation, children))
            }
        };

        Self { transform, visibility, content }
    }
}
