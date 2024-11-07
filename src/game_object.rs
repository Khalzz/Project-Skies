// the entity is the basic object on this "game engine project", it will have the values needed for our "GameObject2Ds"
// the way we "render our objects its based on our object itself" so i will save that "render value" for later

use std::collections::HashMap;

use cgmath::{Deg, Euler, Matrix4, Quaternion, Rad, Vector3};
use serde::{Deserialize, Deserializer};

use crate::{rendering::instance_management::{Instance, InstanceRaw}, transform};

// When i want to do other "element" i have to put this inside, since its the "shorter way" of adding the "basic position and dimensions data"
#[derive(Clone, Copy)]
pub struct GameObject2D {
    pub active: bool,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Deserialize, Clone)]
pub enum Metadata {
    Int(i32),
    Str(String),
    Bool(bool),
    Vector3(Vector3<f32>),
    FloatArray(Vec<f32>),
}

#[derive(Debug, Clone, Copy)]
pub struct Transform {
    pub position: Vector3<f32>,
    pub rotation: Quaternion<f32>,
    pub scale: Vector3<f32>
}

#[derive(Debug, Deserialize, Clone)]
// GameObject
    // this struct contains 2 transform values:
        // 1. render_transform: represents the rendered transform (what the renderer will show in screen)
        // 2. transform: represents the "position" of an object "world relative" so is entirely based on what there is in the json files
pub struct GameObject {
    pub id: String,
    pub model: String,
    pub transform: Transform,
    pub children: Vec<GameObject>,
    pub metadata: HashMap<String, serde_json::Value>
}

#[derive(Debug, Deserialize, Clone, Copy)]
pub struct RawTransform {
    pub position: Vector3<f32>,
    pub rotation: Vector3<f32>,
    pub scale: Vector3<f32>
}

// this transforms the rotation in euler in the  json to the rotation in quaternion for rendering
impl Transform {
    pub fn to_raw(&self) -> InstanceRaw {
        let translation = cgmath::Matrix4::from_translation(self.position.cast::<f32>().unwrap());
        let rotation = cgmath::Matrix4::from(self.rotation.cast::<f32>().unwrap());
        let scale = cgmath::Matrix4::from_nonuniform_scale(self.scale.x as f32, self.scale.y as f32, self.scale.z as f32);
        let model: Matrix4<f32> = translation * Matrix4::from(rotation) * scale;
    
        InstanceRaw {
            model: model.into(),
        }
    }

    // Function to create Transform from RawTransform
    fn from_raw(raw: RawTransform) -> Self {
        let rotation = Euler::new(
            Deg(raw.rotation.x),
            Deg(raw.rotation.y),
            Deg(raw.rotation.z),
        );

        Transform {
            position: raw.position,
            rotation: rotation.into(),
            scale: raw.scale,
        }
    }
}

// Custom deserialization for Transform
impl<'de> Deserialize<'de> for Transform {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw_transform = RawTransform::deserialize(deserializer)?;
        Ok(Transform::from_raw(raw_transform))
    }
}

