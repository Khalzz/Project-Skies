use std::{collections::HashMap, default, mem, ops::Range};


use gltf::material::AlphaMode;
use nalgebra::{UnitQuaternion, Vector3};
use wgpu::{BindGroup, BindGroupLayoutDescriptor, Device};

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
    pub alpha_mode: AlphaMode,
    /// Draw this mesh with backface culling OFF. Set at load from
    /// `resources::DoubleSided` (declared per model at registration). The
    /// glTF `doubleSided` material flag is deliberately NOT used.
    pub double_sided: bool,
    /// The geometry's own box (min, max), in the mesh's local space - before
    /// its transform. Lets code that moves a mesh work from where its shape
    /// actually is, whether or not its origin sits on it (an exporter often
    /// leaves the origin at the model's, with the geometry baked off to the
    /// side - see `local_center`).
    pub local_bounds: ([f32; 3], [f32; 3]),
}

/// # Model
/// A 3D model is defined by meshes, the "mesh_list" is for definition of different mesh types, for example separation of opaque and transparent ones.
pub struct Model {
    pub mesh_lists: HashMap<String, HashMap<String, Mesh>>,
    pub materials: Vec<Material>
}

impl Model {
    /// Every mesh of the object named `name` in list `list` ("opaque" /
    /// "transparent") - ALL of its materials. An object with several
    /// materials loads as one mesh per material: the first keyed by its
    /// plain name, the rest as "<name>#prim1", "#prim2"... (see
    /// resources::traverse_node). Anything animating an object by name has
    /// to move all of them, or its materials drift apart.
    pub fn meshes_named_mut<'a>(&'a mut self, list: &str, name: &'a str) -> impl Iterator<Item = &'a mut Mesh> + 'a {
        let prefix = format!("{name}#prim");
        self.mesh_lists.get_mut(list).into_iter().flat_map(move |meshes| {
            let prefix = prefix.clone();
            meshes.iter_mut().filter(move |(key, _)| key.as_str() == name || key.starts_with(&prefix)).map(|(_, mesh)| mesh)
        })
    }
}

impl Mesh {
    /// The middle of the geometry, in the mesh's local space.
    pub fn local_center(&self) -> Vector3<f32> {
        let (lo, hi) = self.local_bounds;
        Vector3::new((lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, (lo[2] + hi[2]) * 0.5)
    }

    /// Half the geometry's height (local Y) - e.g. a wheel's radius.
    pub fn local_half_height(&self) -> f32 {
        let (lo, hi) = self.local_bounds;
        (hi[1] - lo[1]) * 0.5
    }

    /// How far the geometry reaches below its middle, straight down in the
    /// model (not the mesh's own local Y) - under its authored rotation and
    /// scale. A wheel's radius, whichever way its mesh was modelled (e.g.
    /// turned 90° with its axle along local Y).
    pub fn depth_below_center(&self) -> f32 {
        let (lo, hi) = self.local_bounds;
        let base = &self.base_transform;
        let rotation = UnitQuaternion::from_quaternion(base.rotation);
        let lowest = (0..8).map(|corner| {
            let local = Vector3::new(
                if corner & 1 == 0 { lo[0] } else { hi[0] },
                if corner & 2 == 0 { lo[1] } else { hi[1] },
                if corner & 4 == 0 { lo[2] } else { hi[2] },
            );
            (rotation * base.scale.component_mul(&local)).y
        }).fold(f32::MAX, f32::min);
        self.center_offset().y - lowest
    }

    /// Where the geometry's middle ends up relative to the mesh's position,
    /// under its authored rotation/scale - add a position to it to get where
    /// the shape's middle is drawn.
    pub fn center_offset(&self) -> Vector3<f32> {
        let base = &self.base_transform;
        UnitQuaternion::from_quaternion(base.rotation) * base.scale.component_mul(&self.local_center())
    }

    pub fn update_transform(&self, queue: &wgpu::Queue) {
        let transform_data: Transform;

        match &self.parent_transform {
            Some(parent_transform) => {
                transform_data = Transform::new(
                    parent_transform.position + UnitQuaternion::from_quaternion(parent_transform.rotation) * (self.transform.position - parent_transform.position),
                    parent_transform.rotation * self.transform.rotation,
                    parent_transform.scale,
                );
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

     pub fn create_bind_group_layout(device: &Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("transform_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        })
    }
}

pub trait DrawModel<'a> {
    // Draw multiple instances of a single mesh
    fn draw_mesh_instanced(
        &mut self,
        mesh: &'a Mesh,
        material: &'a Material,
        instances: Range<u32>,
        camera_bind_group: &'a wgpu::BindGroup,
        light_bind_group: &'a wgpu::BindGroup
    );

    // Draw multiple instances of the entire model. Sets the pipeline itself,
    // per mesh - the caller must NOT set one.
    //
    //  - `cull_back`  : single-sided meshes (and the near-side pass of a
    //                   double-sided one).
    //  - `no_cull`    : `Some` for the opaque pass - a double-sided mesh is
    //                   drawn once with no culling. `None` for the transparent
    //                   pass.
    //  - `cull_front` : `Some` for the transparent pass - a double-sided mesh
    //                   is drawn twice, back faces (far side) with this then
    //                   front faces (near side) with `cull_back`, so the near
    //                   wall blends over the far wall for a convex shell.
    //
    // Which meshes are double-sided comes from `Mesh::double_sided` (set at
    // load from `resources::DoubleSided`).
    fn draw_model_instanced_from_list(
        &mut self,
        model: &'a Model,
        instances: Range<u32>,
        camera_bind_group: &'a wgpu::BindGroup,
        light_bind_group: &'a wgpu::BindGroup,
        list_name: &String,
        cull_back: &'a wgpu::RenderPipeline,
        no_cull: Option<&'a wgpu::RenderPipeline>,
        cull_front: Option<&'a wgpu::RenderPipeline>,
    );
}

impl<'a, 'b> DrawModel<'b> for wgpu::RenderPass<'a>
where
    'b: 'a,
{
    fn draw_mesh_instanced(&mut self, mesh: &'b Mesh, material: &'b Material, instances: Range<u32>, camera_bind_group: &'b wgpu::BindGroup, light_bind_group: &'a wgpu::BindGroup) {
        self.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
        self.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        self.set_bind_group(0, &material.bind_group, &[]);
        self.set_bind_group(1, camera_bind_group, &[]);
        self.set_bind_group(2, &mesh.transform_bind_group, &[]);
        self.set_bind_group(3, light_bind_group, &[]);
        self.draw_indexed(0..mesh.num_elements, 0, instances);
    }

    // The `active` tracker is a loop-carried "which pipeline is bound" that
    // skips redundant set_pipeline calls; its store on the final iteration is
    // unavoidably dead.
    #[allow(unused_assignments)]
    fn draw_model_instanced_from_list(
        &mut self,
        model: &'b Model,
        instances: Range<u32>,
        camera_bind_group: &'b wgpu::BindGroup,
        light_bind_group: &'a wgpu::BindGroup,
        list_name: &String,
        cull_back: &'b wgpu::RenderPipeline,
        no_cull: Option<&'b wgpu::RenderPipeline>,
        cull_front: Option<&'b wgpu::RenderPipeline>,
    ) {
        let Some(meshes) = model.mesh_lists.get(list_name) else { return };
        // 0 = cull_back, 1 = no_cull, 2 = cull_front. `None` = nothing set yet
        // (the caller doesn't set a pipeline), so the first draw always sets one.
        let mut active: Option<u8> = None;
        for (_key, mesh) in meshes {
            let material = &model.materials[mesh.material];
            if mesh.double_sided {
                if let Some(front) = cull_front {
                    // Two-pass: far side (back faces) then near side (front faces).
                    self.set_pipeline(front);
                    self.draw_mesh_instanced(mesh, material, instances.clone(), camera_bind_group, light_bind_group);
                    self.set_pipeline(cull_back);
                    active = Some(0);
                    self.draw_mesh_instanced(mesh, material, instances.clone(), camera_bind_group, light_bind_group);
                    continue;
                }
                if let Some(nc) = no_cull {
                    if active != Some(1) { self.set_pipeline(nc); active = Some(1); }
                    self.draw_mesh_instanced(mesh, material, instances.clone(), camera_bind_group, light_bind_group);
                    continue;
                }
            }
            if active != Some(0) { self.set_pipeline(cull_back); active = Some(0); }
            self.draw_mesh_instanced(mesh, material, instances.clone(), camera_bind_group, light_bind_group);
        }
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
        for (_id, mesh_lists) in &model.mesh_lists {
            for mesh in mesh_lists {
                self.draw_light_mesh_instanced(mesh.1, instances.clone(), camera_bind_group, light_bind_group);
            }
        }
    }
}