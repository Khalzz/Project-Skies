use glyphon::{cosmic_text::Align, Color, TextArea};
use nalgebra::vector;
use crate::{app::{App, Size}, rendering::vertex::VertexUi};
use super::{label::{self, Label}, ui_transform::{Rect, UiTransform}, vertical_container::{self, VecticalContainerData}};

pub enum Alignment {
    Start,
    Center,
    Custom,

    VerticalAlignment(f32)
    
}

pub enum UiNodeContent {
    Text(Label),
    VerticalContainer(VecticalContainerData)
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
        separation: f32,
        children: Vec<UiNode>
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
            UiNodeParameters::VerticalContainerData { separation, children } => UiNodeContent::VerticalContainer(VecticalContainerData::new(separation, children)),
        };
        
        Self {
            transform,
            visibility,
            content,
            parent_transform: None
        }
    }

    // this function will dedicate itself mainly to set how we will "display" each element on the screen, either the node is a singular object or a list of them
    pub fn node_content_preparation(&mut self, size: &Size, vertices: &mut Vec<VertexUi>, indices: &mut Vec<u16>, num_vertices: &mut u16) -> (Vec<TextArea>, u16, u32) {
        let mut text_areas: Vec<TextArea> = Vec::new();
        let mut vertices_to_add = 0;
        let mut indices_to_add = 0;

        let vartices_slice = self.vertices(size);
        let indice_slice = self.indices(*num_vertices);

        match &mut self.content {
            UiNodeContent::Text(label) => {
                let (text_area, added_vertices, added_indices) = label.ui_node_data_creation(size, vertices, &vartices_slice, indices, &indice_slice, num_vertices, &self.transform.rect);

                text_areas.push(text_area);
                vertices_to_add += added_vertices;
                indices_to_add += added_indices;
            },
            UiNodeContent::VerticalContainer(vertical_container) => {
                let (text_area, added_vertices, added_indices) = vertical_container.ui_node_data_creation(size, vertices, &vartices_slice, indices, &indice_slice, num_vertices, &mut self.transform.rect);

                text_areas.extend(text_area);
                vertices_to_add += added_vertices;
                indices_to_add += added_indices;
            },
        }

        (text_areas, vertices_to_add, indices_to_add)
    }

    // fix this so we can use it for instantiation of multiple objects

    

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
}
