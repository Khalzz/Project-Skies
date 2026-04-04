use std::collections::HashMap;

use nalgebra::Vector2;
use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
pub struct UiStructure {
  pub id: String,
  pub children: HashMap<String, UiComponent>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct UiComponent {
    pub transform: Transform2D,
    pub child: UiNode,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Size2D {
  pub width: f32,
  pub height: f32,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Transform2D {
  pub position: Vector2<f32>,
  pub size: Option<Size2D>,
  pub anchor: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub enum UiNode {
    Label(Label),
    VerticalContainer(VerticalContainer),
}

#[derive(Deserialize, Debug, Clone)]
pub struct VerticalContainer {
    pub margin: Option<f32>,
    pub separation: Option<f32>,
    pub background_color: Option<[f32; 4]>,
    pub border_color: Option<[f32; 4]>,
    pub children: Option<HashMap<String, UiComponent>>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Label {
    pub text: String,
    pub font_size: f32,
    pub color: [f32; 4],
    pub alignment: Option<String>,
    pub background_color: Option<[f32; 4]>,
    pub border_color: Option<[f32; 4]>,
}