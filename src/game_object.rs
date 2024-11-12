// the entity is the basic object on this "game engine project", it will have the values needed for our "GameObject2Ds"
// the way we "render our objects its based on our object itself" so i will save that "render value" for later

use std::collections::HashMap;

use cgmath::{Deg, Euler, Matrix3, Matrix4, Quaternion, Rad, Vector3};
use rapier3d::prelude::Collider;
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

#[derive(Debug, Clone, Copy)]
pub struct Transform {
    pub position: Vector3<f32>,
    pub rotation: Quaternion<f32>,
    pub scale: Vector3<f32>
}

/* 

(
    id: "world",
    model: "Water/water.gltf",
    transform: (
        position: (x: 0.0, y: 0.0, z: 0.0),
        rotation: (x: 0.0, y: 0.0, z: 0.0),
        scale: (x: 100000.0, y: 1.0, z: 100000.0),
    ),
    children: [],
    metadata: (
        physics: Some(
            rigidbody: (
                is_static: true,
                mass: 0,
                initial_velocity: (0.0, 0.0, 550.0),
            ),
            collider: Some((
                    shape: Cuboid {  10.0, 0.1, 10.0 }
            ))
        )
    ),
)

*/

#[derive(Debug, Deserialize, Clone)]
pub struct Lighting {
    pub intensity: f32,
    pub color: Vector3<f32>
}

#[derive(Debug, Deserialize, Clone)]
pub enum ColliderType {
    Cuboid { half_extents: (f32, f32, f32) },
    Ball { radius: f32 },
    Cylinder { half_height: f32, radius: f32 },
    HeightField { heights: Vec<Vec<f32>>, scale_x: f32, scale_y: f32 },
    HalfSpace { normal:  nalgebra::Vector3<f32> }
}

#[derive(Debug, Deserialize, Clone)]
pub struct RigidBodyData {
    pub is_static: bool,
    pub mass: f32,
    pub initial_velocity: nalgebra::Vector3<f32>
}

#[derive(Debug, Deserialize, Clone)]
pub struct Physics {
    pub rigidbody: RigidBodyData,
    pub collider: Option<ColliderType>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CameraData {
    pub cockpit_camera: Vector3<f32>,
    pub cinematic_camera: Vector3<f32>,
    pub frontal_camera: Vector3<f32>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MetaData {
    pub physics: Option<Physics>,
    pub cameras: Option<CameraData>,
    pub lighting: Option<Lighting>
}

#[derive(Debug, Deserialize, Clone)]
pub struct Scene {
    pub id: String,
    pub description: String,
    pub children: Vec<GameObject>
}

#[derive(Debug, Deserialize, Clone)]
pub struct GameObject {
    pub id: String,
    pub model: String,
    pub transform: Transform,
    pub children: Vec<GameObject>,
    pub metadata: MetaData
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
            normal: Matrix3::from(self.rotation).into()
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

