
use cgmath::{Matrix3, Matrix4};
use rapier3d::prelude::RigidBodyHandle;
use serde::Deserialize;
use wgpu::Buffer;

use crate::game_object::{GameObject, Transform};

use super::model::Model;

pub struct PhysicsData {
    pub rigidbody_handle: RigidBodyHandle
}

// when moving player we move instance, when moving world object we move renderizable_transform
pub struct InstanceData {
    pub physics_data: Option<PhysicsData>,
    pub renderizable_transform: Transform,
    pub instance: GameObject,
    pub model_ref: String,
}

#[derive(Clone)]
pub struct Instance {
    pub position: cgmath::Vector3<f32>,
    pub rotation: cgmath::Quaternion<f32>,
    pub scale: cgmath::Vector3<f32>,
}

impl Instance {
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

// quaternions are not very usable in wgpu so instead of doing math in the shader we are gonna save the raw instance here directly
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct InstanceRaw {
    pub(crate) model: [[f32; 4]; 4],
    pub(crate) normal: [[f32; 3]; 3]
}

impl InstanceRaw {
    // we create a vertexBuffer
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        use std::mem;
        wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<InstanceRaw>() as wgpu::BufferAddress, // the vertexbuffer is of type instance raw it means that our shader will only change to use the next instance
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                // A mat 4 is basically a vec4 of vec4's so we have to define every vec4 of it
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
                // these are for the rotation of ilumination normals
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
// Instancing
