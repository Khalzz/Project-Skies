use nalgebra::{Matrix4, Vector3, UnitQuaternion};
use serde::Deserialize;
use wgpu::Buffer;

use crate::{game_nodes::game_object::{GameObject, Transform}, rendering::models::model::Model};

// When moving player we move instance; when moving world object we move renderizable_transform
pub struct InstanceData {
    pub renderizable_transform: Transform,
    pub instance: GameObject,
    pub model_ref: String,
    /// The shader it's drawn with (its node's `Material`) - see
    /// engine::rendering::shader_library.
    pub material: String,
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
    /// Which of this frame's instances (in the buffer, in order) each
    /// material draws - they're written sorted by material, so each is one
    /// run (see App::run). Empty until the first frame's written.
    pub material_ranges: Vec<(String, std::ops::Range<u32>)>,
    /// Some of its meshes drawn with another material (by mesh name, the
    /// material instance - see Material::meshes), set every frame from the
    /// node that has them.
    pub mesh_materials: std::collections::HashMap<String, String>,
    /// Which entry this is - a new one for every one made (see `new_stamp`),
    /// so one put in place of another under the same name (a node spawned
    /// with a model already drawn) reads as a change (see
    /// App::place_materials).
    pub stamp: u64,
}

impl ModelDataInstance {
    /// A stamp no entry has had before - see `stamp`.
    pub fn new_stamp() -> u64 {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }
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