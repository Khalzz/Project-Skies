//! The engine's debug views - `app.renderer.debug`, each off by default:
//!
//!     app.renderer.debug.physics = true;   // collider shapes, and what physics behaviors draw
//!     app.renderer.debug.wireframe = true; // every model's edges
//!     app.renderer.debug.depth = true;     // the depth buffer, near dark to far light
//!
//! A view that's off costs nothing.

use std::collections::{HashMap, HashSet};

use wgpu::util::DeviceExt;

use crate::rendering::instance_management::{InstanceRaw, ModelDataInstance};
use crate::rendering::models::model::Mesh;
use crate::rendering::models::textures::Texture;

/// Which debug views are on - see the module doc.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DebugViews {
    pub physics: bool,
    pub wireframe: bool,
    pub depth: bool,
}

/// One mesh's edges, ready to draw - its model's own buffers, read by the
/// wireframe shader (see wireframe.wgsl), and where its vertices start.
struct MeshEdges {
    bind_group: wgpu::BindGroup,
    _base_vertex: wgpu::Buffer,
}

/// Draws `DebugViews::wireframe`: every edge of every drawn model, over the
/// world, where it isn't hidden. All on the GPU - each mesh drawn as lines
/// straight from its model's own vertex and index buffers and its instances,
/// so it follows physics and animation with no work for the CPU.
pub(crate) struct WireframeRenderer {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    /// By model key, its entry's stamp (see ModelDataInstance::stamp), and
    /// mesh - made the first time a mesh is drawn, dropped with its model.
    meshes: HashMap<(String, u64, String), MeshEdges>,
}

impl WireframeRenderer {
    pub(crate) fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat, camera_layout: &wgpu::BindGroupLayout) -> Self {
        let storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Wireframe mesh layout"),
            entries: &[
                storage(0),
                storage(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Wireframe shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/debug/wireframe.wgsl").into()),
        });
        let transform_layout = Mesh::create_bind_group_layout(device);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Wireframe pipeline layout"),
            bind_group_layouts: &[&layout, camera_layout, &transform_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Wireframe"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_main"), buffers: &[InstanceRaw::desc()], compilation_options: Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState { format: color_format, blend: Some(wgpu::BlendState::REPLACE), write_mask: wgpu::ColorWrites::ALL })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::LineList, ..Default::default() },
            // Against the world's depth, so hidden edges stay hidden - not
            // written, so edges never hide each other.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: Texture::DEPTH_FORMAT,
                depth_write_enabled: false,
                // Reversed-Z: nearer is greater.
                depth_compare: wgpu::CompareFunction::GreaterEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        Self { pipeline, layout, meshes: HashMap::new() }
    }

    /// Draws every drawn model's edges into `view`, tested against `depth`.
    pub(crate) fn draw(&mut self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, depth: &wgpu::TextureView, camera: &wgpu::BindGroup, models: &HashMap<String, ModelDataInstance>) {
        let drawn: Vec<(&String, &ModelDataInstance)> = models.iter().filter(|(_, model)| model.instance_count > 0).collect();
        self.prepare(device, &drawn);

        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Wireframe pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: depth, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }), stencil_ops: None }),
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(1, camera, &[]);
        for (key, model) in drawn {
            render_pass.set_vertex_buffer(0, model.instance_buffer.slice(..));
            for (mesh_key, mesh) in model.model.mesh_lists.values().flatten() {
                let Some(edges) = self.meshes.get(&(key.clone(), model.stamp, mesh_key.clone())) else { continue };
                render_pass.set_bind_group(0, &edges.bind_group, &[]);
                render_pass.set_bind_group(2, &mesh.transform_bind_group, &[]);
                // Two points an edge, three edges a triangle - the shader
                // finds which from the vertex's number.
                render_pass.draw(mesh.first_index * 2..(mesh.first_index + mesh.num_elements) * 2, 0..model.instance_count);
            }
        }
    }

    /// The edges of every mesh of `drawn` not made yet - and the ones of
    /// models gone (or made anew) let go of.
    fn prepare(&mut self, device: &wgpu::Device, drawn: &[(&String, &ModelDataInstance)]) {
        let current: HashSet<(&str, u64)> = drawn.iter().map(|(key, model)| (key.as_str(), model.stamp)).collect();
        self.meshes.retain(|(key, stamp, _), _| current.contains(&(key.as_str(), *stamp)));
        for (key, model) in drawn {
            for (mesh_key, mesh) in model.model.mesh_lists.values().flatten() {
                let entry = (key.to_string(), model.stamp, mesh_key.clone());
                if self.meshes.contains_key(&entry) {
                    continue;
                }
                let base_vertex = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Wireframe base vertex"),
                    contents: bytemuck::cast_slice(&[mesh.base_vertex, 0, 0, 0]),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Wireframe mesh"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: model.model.vertex_buffer.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: model.model.index_buffer.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 2, resource: base_vertex.as_entire_binding() },
                    ],
                });
                self.meshes.insert(entry, MeshEdges { bind_group, _base_vertex: base_vertex });
            }
        }
    }
}
