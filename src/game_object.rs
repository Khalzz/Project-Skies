// the entity is the basic object on this "game engine project", it will have the values needed for our "GameObject2Ds"
// the way we "render our objects its based on our object itself" so i will save that "render value" for later

use nalgebra::{Vector3, Matrix3, Matrix4, UnitQuaternion};
use serde::{Deserialize, Deserializer};

use crate::rendering::instance_management::InstanceRaw;

// GameObject2D remains the same
#[derive(Clone, Copy)]
pub struct GameObject2D {
    pub active: bool,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

// Transform struct
#[derive(Debug, Clone, Copy)]
pub struct Transform {
    pub position: Vector3<f32>,
    pub rotation: UnitQuaternion<f32>, // Use UnitQuaternion for normalized rotations
    pub scale: Vector3<f32>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Lighting {
    pub intensity: f32,
    pub color: Vector3<f32>,
}

#[derive(Debug, Deserialize, Clone)]
pub enum ColliderType {
    Cuboid { half_extents: (f32, f32, f32) },
    Ball { radius: f32 },
    Cylinder { half_height: f32, radius: f32 },
    HeightField { heights: Vec<Vec<f32>>, scale_x: f32, scale_y: f32 },
    HalfSpace { normal: Vector3<f32> },
}

#[derive(Debug, Deserialize, Clone)]
pub struct RigidBodyData {
    pub is_static: bool,
    pub mass: f32,
    pub initial_velocity: Vector3<f32>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Physics {
    pub rigidbody: RigidBodyData,
    pub collider: Option<ColliderType>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Cameras {
    pub cockpit_camera: Vector3<f32>,
    pub cinematic_camera: Vector3<f32>,
    pub frontal_camera: Vector3<f32>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct MetaData {
    pub physics: Option<Physics>,
    pub cameras: Option<Cameras>,
    pub lighting: Option<Lighting>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Scene {
    pub id: String,
    pub description: String,
    pub children: Vec<GameObject>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct GameObject {
    pub id: String,
    pub model: String,
    pub transform: Transform,
    pub children: Vec<GameObject>,
    pub metadata: MetaData,
}

#[derive(Debug, Deserialize, Clone, Copy)]
pub struct RawTransform {
    pub position: Vector3<f32>,
    pub rotation: Vector3<f32>,
    pub scale: Vector3<f32>,
}

// Implementing `to_raw` to convert Transform into InstanceRaw
impl Transform {
    pub fn to_raw(&self) -> InstanceRaw {
        let translation = Matrix4::new_translation(&self.position);
        let rotation = self.rotation.to_homogeneous();
        let scale = Matrix4::new_nonuniform_scaling(&self.scale);
        let model = translation * rotation * scale;

        InstanceRaw {
            model: model.into(),
            normal: Matrix3::from(self.rotation).into(),
        }
    }

    // Function to create Transform from RawTransform, converting Euler angles to quaternion
    fn from_raw(raw: RawTransform) -> Self {
        let rotation = UnitQuaternion::from_euler_angles(
            raw.rotation.x.to_radians(),
            raw.rotation.y.to_radians(),
            raw.rotation.z.to_radians(),
        );

        Transform {
            position: raw.position,
            rotation,
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