use serde::Deserialize;
use super::game_object::GameObject;

#[derive(Debug, Deserialize, Clone)]
pub struct Scene {
    pub id: String,
    pub description: String,
    pub children: Vec<GameObject>,
}