use nalgebra::{Matrix4, Vector3, UnitQuaternion};
use rapier3d::prelude::{ColliderHandle, RigidBodyHandle};
use serde::Deserialize;
use wgpu::Buffer;

use crate::game_nodes::game_object::{GameObject, Transform};
use super::model::Model;

pub struct PhysicsData {
    pub rigidbody_handle: RigidBodyHandle,
    pub collider_handle: Option<ColliderHandle>
}

// When moving player we move instance; when moving world object we move renderizable_transform
pub struct InstanceData {
    pub physics_data: Option<PhysicsData>,
    pub renderizable_transform: Transform,
    pub instance: GameObject,
    pub model_ref: String,
}

#[derive(Clone)]
pub struct Instance {
    pub position: Vector3<f32>,
    pub rotation: UnitQuaternion<f32>,
    pub scale: Vector3<f32>,
}

impl Instance {
    pub fn to_raw(&self) -> InstanceRaw {
        let translation = Matrix4::new_translation(&self.position);
        let rotation = self.rotation.to_homogeneous();
        let scale = Matrix4::new_nonuniform_scaling(&self.scale);
        let model: Matrix4<f32> = translation * rotation * scale;

        InstanceRaw {
            model: model.into(),
            normal: (*self.rotation.to_rotation_matrix().matrix()).into(),
        }
    }
}

pub struct ModelDataInstance {
    pub model: Model,
    pub instance_buffer: Buffer,
    pub instance_count: u32,
}

#[derive(Debug, Deserialize)]
pub struct LevelData {
    pub id: String,
    pub description: String,
    pub children: Vec<GameObject>,
}

// Quaternions are not very usable in wgpu, so we save the raw instance here directly
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct InstanceRaw {
    pub(crate) model: [[f32; 4]; 4],
    pub(crate) normal: [[f32; 3]; 3],
}

impl InstanceRaw {
    // Create a vertex buffer layout for InstanceRaw
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        use std::mem;
        wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<InstanceRaw>() as wgpu::BufferAddress, // Instance raw means shader will change per instance
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                // Define each vec4 in a mat4
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 5,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: mem::size_of::<[f32; 4]>() as wgpu::BufferAddress,
                    shader_location: 6,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: mem::size_of::<[f32; 8]>() as wgpu::BufferAddress,
                    shader_location: 7,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: mem::size_of::<[f32; 12]>() as wgpu::BufferAddress,
                    shader_location: 8,
                    format: wgpu::VertexFormat::Float32x4,
                },
                // Rotation for illumination normals
                wgpu::VertexAttribute {
                    offset: mem::size_of::<[f32; 16]>() as wgpu::BufferAddress,
                    shader_location: 9,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: mem::size_of::<[f32; 19]>() as wgpu::BufferAddress,
                    shader_location: 10,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: mem::size_of::<[f32; 22]>() as wgpu::BufferAddress,
                    shader_location: 11,
                    format: wgpu::VertexFormat::Float32x3,
                },
            ],
        }
    }
}