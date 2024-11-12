use std::{collections::HashMap, default, mem, ops::Range};


use cgmath::Rotation;
use gltf::material::AlphaMode;
use wgpu::BindGroup;

use crate::transform::Transform;

use super::textures::Texture;

pub trait Vertex {
    fn desc() -> wgpu::VertexBufferLayout<'static>;
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ModelVertex {
    pub position: [f32; 3],
    pub tex_coords: [f32; 2],
    pub normal: [f32; 3],
}

impl Vertex for ModelVertex {
    fn desc() -> wgpu::VertexBufferLayout<'static> {
        // Vertex buffer layout with normalized coordinates
        wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<ModelVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                // Position attribute
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                // Texture coordinates attribute
                wgpu::VertexAttribute {
                    offset: mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // Normal attribute
                wgpu::VertexAttribute {
                    offset: mem::size_of::<[f32; 5]>() as wgpu::BufferAddress,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x3,
                },
            ],
        }
    }
}

pub struct Material {
    pub name: String,
    pub diffuse_texture: Texture,
    pub bind_group: wgpu::BindGroup,
}

pub struct Mesh {
    pub name: String,
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub num_elements: u32,
    pub material: usize,
    pub transform_buffer: wgpu::Buffer,
    pub transform_bind_group: wgpu::BindGroup,
    pub transform: Transform,
    pub base_transform: Transform,
    pub parent_transform: Option<Transform>,
    pub alpha_mode: AlphaMode
}

pub struct Model {
    pub meshes: HashMap<String, Mesh>,
    pub materials: Vec<Material>
}

impl Mesh {
    pub fn update_transform(&self, queue: &wgpu::Queue) {
        let transform_data: Transform;

        match &self.parent_transform {
            Some(parent_transform) => {
                transform_data = Transform::new(parent_transform.position + parent_transform.rotation.rotate_vector(self.transform.position - parent_transform.position), parent_transform.rotation * self.transform.rotation, parent_transform.scale)
            },
            None => {
                transform_data = Transform::new(self.transform.position, self.transform.rotation, self.transform.scale)
            },
        }

        queue.write_buffer(&self.transform_buffer, 0, bytemuck::cast_slice(&[transform_data.to_matrix_bufferable()]));
    }

    pub fn change_transform(&mut self, queue: &wgpu::Queue, transform: Transform) {
        if transform.position != self.transform.position || transform.rotation != self.transform.rotation || transform.scale != self.transform.scale {
            self.transform = transform;
            self.update_transform(queue);
        }
    }
}

pub trait DrawModel<'a> {
    // Draw a single mesh of the model
    fn draw_mesh(&mut self, mesh: &'a Mesh, material: &'a Material, camera_bind_group: &'a wgpu::BindGroup, light_bind_group: &'a wgpu::BindGroup);
    
    // Draw multiple instances of a single mesh
    fn draw_mesh_instanced(
        &mut self,
        mesh: &'a Mesh,
        material: &'a Material,
        instances: Range<u32>,
        camera_bind_group: &'a wgpu::BindGroup,
        light_bind_group: &'a wgpu::BindGroup
    );

    // Draw all meshes of the model
    fn draw_model(&mut self, model: &'a Model, camera_bind_group: &'a wgpu::BindGroup, light_bind_group: &'a wgpu::BindGroup);

    // Draw multiple instances of the entire model
    fn draw_model_instanced(
        &mut self,
        model: &'a Model,
        instances: Range<u32>,
        camera_bind_group: &'a wgpu::BindGroup,
        light_bind_group: &'a wgpu::BindGroup
    );

    // New function to draw only transparent objects
    fn draw_transparent_model_instanced(
        &mut self,
        model: &'a Model,
        instances: Range<u32>,
        camera_bind_group: &'a wgpu::BindGroup,
        light_bind_group: &'a wgpu::BindGroup
    );
}

impl<'a, 'b> DrawModel<'b> for wgpu::RenderPass<'a>
where
    'b: 'a,
{
    fn draw_mesh(&mut self, mesh: &'b Mesh, material: &'b Material, camera_bind_group: &'b wgpu::BindGroup, light_bind_group: &'a wgpu::BindGroup) {
        self.draw_mesh_instanced(mesh, material, 0..1, camera_bind_group, &light_bind_group);
    }

    fn draw_mesh_instanced(&mut self, mesh: &'b Mesh, material: &'b Material, instances: Range<u32>, camera_bind_group: &'b wgpu::BindGroup, light_bind_group: &'a wgpu::BindGroup) {
        self.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
        self.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        self.set_bind_group(0, &material.bind_group, &[]);
        self.set_bind_group(1, camera_bind_group, &[]);
        self.set_bind_group(2, &mesh.transform_bind_group, &[]);
        self.set_bind_group(3, &light_bind_group, &[]);
        self.draw_indexed(0..mesh.num_elements, 0, instances);
    }

    fn draw_model(&mut self, model: &'b Model, camera_bind_group: &'b wgpu::BindGroup, light_bind_group: &'a wgpu::BindGroup) {
        self.draw_model_instanced(model, 0..1, camera_bind_group, &light_bind_group);
    }

    fn draw_model_instanced(&mut self, model: &'b Model, instances: Range<u32>, camera_bind_group: &'b wgpu::BindGroup, light_bind_group: &'a wgpu::BindGroup) {
        let _ = &model.meshes.iter()
        .filter(|(_key, mesh)| mesh.alpha_mode != gltf::material::AlphaMode::Blend && mesh.alpha_mode != gltf::material::AlphaMode::Mask)
        .for_each(|(_key, mesh)| {
            let material = &model.materials[mesh.material];
            self.draw_mesh_instanced(mesh, material, instances.clone(), camera_bind_group, light_bind_group);
        });
    }
    
    fn draw_transparent_model_instanced( &mut self, model: &'b Model, instances: Range<u32>, camera_bind_group: &'b wgpu::BindGroup, light_bind_group: &'a wgpu::BindGroup) {
        let _ = &model.meshes.iter()
        .for_each(|(_key, mesh)| {
            if mesh.alpha_mode == gltf::material::AlphaMode::Blend || mesh.alpha_mode == gltf::material::AlphaMode::Mask {
                let material = &model.materials[mesh.material];
                self.draw_mesh_instanced(mesh, material, instances.clone(), camera_bind_group, light_bind_group);
            } 
        });
    }
}

// This trait will entirely be dedicated to draw the lighting of the scene
pub trait DrawLight<'a> {
    fn draw_light_mesh(
        &mut self,
        mesh: &'a Mesh,
        camera_bind_group: &'a wgpu::BindGroup,
        light_bind_group: &'a wgpu::BindGroup,
    );
    fn draw_light_mesh_instanced(
        &mut self,
        mesh: &'a Mesh,
        instances: Range<u32>,
        camera_bind_group: &'a wgpu::BindGroup,
        light_bind_group: &'a wgpu::BindGroup,
    );

    fn draw_light_model(
        &mut self,
        model: &'a Model,
        camera_bind_group: &'a wgpu::BindGroup,
        light_bind_group: &'a wgpu::BindGroup,
    );
    fn draw_light_model_instanced(
        &mut self,
        model: &'a Model,
        instances: Range<u32>,
        camera_bind_group: &'a wgpu::BindGroup,
        light_bind_group: &'a wgpu::BindGroup,
    );
}

impl<'a, 'b> DrawLight<'b> for wgpu::RenderPass<'a>
where
    'b: 'a,
{
    fn draw_light_mesh(
        &mut self,
        mesh: &'b Mesh,
        camera_bind_group: &'b wgpu::BindGroup,
        light_bind_group: &'b wgpu::BindGroup,
    ) {
        self.draw_light_mesh_instanced(mesh, 0..1, camera_bind_group, light_bind_group);
    }

    fn draw_light_mesh_instanced(
        &mut self,
        mesh: &'b Mesh,
        instances: Range<u32>,
        camera_bind_group: &'b wgpu::BindGroup,
        light_bind_group: &'b wgpu::BindGroup,
    ) {
        self.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
        self.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        self.set_bind_group(0, camera_bind_group, &[]);
        self.set_bind_group(1, light_bind_group, &[]);
        self.draw_indexed(0..mesh.num_elements, 0, instances);
    }

    fn draw_light_model(
        &mut self,
        model: &'b Model,
        camera_bind_group: &'b wgpu::BindGroup,
        light_bind_group: &'b wgpu::BindGroup,
    ) {
        self.draw_light_model_instanced(model, 0..1, camera_bind_group, light_bind_group);
    }
    fn draw_light_model_instanced(
        &mut self,
        model: &'b Model,
        instances: Range<u32>,
        camera_bind_group: &'b wgpu::BindGroup,
        light_bind_group: &'b wgpu::BindGroup,
    ) {
        for mesh in &model.meshes {
            self.draw_light_mesh_instanced(mesh.1, instances.clone(), camera_bind_group, light_bind_group);
        }
    }
}