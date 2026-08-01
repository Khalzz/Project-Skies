use std::collections::HashMap;

use serde::Deserialize;
use super::ui_transform::{SelfAnchor, ChildAnchor, Direction, Fit};

#[derive(Deserialize, Debug, Clone)]
pub struct UiStructure {
    pub id: String,
    pub children: HashMap<String, UiComponent>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct UiComponent {
    pub transform: Transform2D,
    pub content: UiContent,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Size2D {
    pub width: f32,
    pub height: f32,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Position2D {
    pub x: f32,
    pub y: f32,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Transform2D {
    pub position: Position2D,
    pub size: Option<Size2D>,
    pub self_anchor: Option<SelfAnchor>,
    pub child_anchor: Option<ChildAnchor>,
    pub direction: Option<Direction>,
    pub fit: Option<Fit>,
}

#[derive(Deserialize, Debug, Clone)]
pub enum UiContent {
    Label(LabelData),
    Container(ContainerData),
}

#[derive(Deserialize, Debug, Clone)]
pub struct LabelData {
    pub text: String,
    pub font_size: f32,
    pub color: [f32; 4],
    pub alignment: Option<String>,
    pub background_color: Option<[f32; 4]>,
    pub border_color: Option<[f32; 4]>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ContainerData {
    pub margin: Option<f32>,
    pub gap: Option<f32>,
    pub background_color: Option<[f32; 4]>,
    pub border_color: Option<[f32; 4]>,
    pub children: Option<HashMap<String, UiComponent>>,
}
