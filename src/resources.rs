use std::io::{BufReader, Cursor};

use cgmath::{Matrix4, Quaternion, Vector3, Zero};
use gltf::{buffer, image, import, Document, Gltf};
use wgpu::{util::DeviceExt, Buffer};

use crate::{rendering::{model::{self, Material, Model, ModelVertex, Vertex}, textures::Texture}, transform::Transform};

pub struct BaseModel {
gltf: Document,
buffers: Vec<buffer::Data>,
images: Vec<image::Data>
}

pub async fn load_string(file_name: &str) -> anyhow::Result<String> {
    let path = std::path::Path::new(env!("OUT_DIR")).join("res").join(file_name);
    let txt = std::fs::read_to_string(path).unwrap();
    Ok(txt)
}

pub async fn load_binary(file_name: &str) -> anyhow::Result<Vec<u8>> {
    let path = std::path::Path::new(env!("OUT_DIR"))
    .join("res")
    .join(file_name);
    let data = std::fs::read(path)?;
Ok(data)
}

pub async fn load_texture(file_name: &str, device: &wgpu::Device, queue: &wgpu::Queue) -> anyhow::Result<Texture> {
    let data = load_binary(file_name).await?;
    Texture::from_bytes(&data, device, queue, file_name)
}


pub async fn load_model_gltf(file_name: &str, device: &wgpu::Device, queue: &wgpu::Queue, transform_bind_group_layout: &wgpu::BindGroupLayout) -> anyhow::Result<Model> {
    let gltf_text = load_string(file_name).await.unwrap();
    let gltf_cursor = Cursor::new(gltf_text);
    let gltf_reader = BufReader::new(gltf_cursor);
    let gltf = Gltf::from_reader(gltf_reader)?;

    // Load buffers
    let mut buffer_data = Vec::new();
    for buffer in gltf.buffers() {
        match buffer.source() {
            gltf::buffer::Source::Bin => {
                if let Some(blob) = gltf.blob.as_deref() {
                    buffer_data.push(blob.to_vec());
                }
            }
            gltf::buffer::Source::Uri(uri) => {
                let bin = load_binary(uri).await?;
                buffer_data.push(bin);
            }
        }
    }

    // Load materials
    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
        label: Some("texture_bind_group_layout"),
    });

    let mut materials = Vec::new();
    for material in gltf.materials() {
        let pbr = material.pbr_metallic_roughness();
        let texture_source = pbr.base_color_texture()
            .map(|tex| tex.texture().source().source())
            .expect("texture");

        match texture_source {
            gltf::image::Source::View { view, .. } => {
                let diffuse_texture = Texture::from_bytes(
                    &buffer_data[view.buffer().index()],
                    device,
                    queue,
                    file_name,
                ).expect("Couldn't load diffuse");

                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    layout: &bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&diffuse_texture.view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&diffuse_texture.sampler),
                        },
                    ],
                    label: None,
                });

                materials.push(model::Material {
                    name: material.name().unwrap_or("Default Material").to_string(),
                    diffuse_texture,
                    bind_group
                });
            }
            gltf::image::Source::Uri { uri, mime_type } => {
                let diffuse_texture = load_texture(uri, device, queue).await?;

                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    layout: &bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&diffuse_texture.view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&diffuse_texture.sampler),
                        },
                    ],
                    label: None,
                });

                materials.push(model::Material {
                    name: material.name().unwrap_or("Default Material").to_string(),
                    diffuse_texture,
                    bind_group
                });
            }
        };
    }

    // Load meshes
    let mut meshes = Vec::new();
    for scene in gltf.scenes() {
        for node in scene.nodes() {
            // Recursively process each node and its children
            process_node(&node, &gltf, &buffer_data, device, queue, &transform_bind_group_layout, &mut meshes, file_name)?;
        }
    }

    Ok(model::Model {
        meshes,
        materials,
    })
}

fn process_node(node: &gltf::Node,gltf: &Gltf,buffer_data: &[Vec<u8>],device: &wgpu::Device,queue: &wgpu::Queue,bind_group_layout: &wgpu::BindGroupLayout,meshes: &mut Vec<model::Mesh>,file_name: &str,
) -> anyhow::Result<()> {
    if let Some(mesh) = node.mesh() {
        for primitive in mesh.primitives() {
            let reader = primitive.reader(|buffer| Some(&buffer_data[buffer.index()]));

            let mut vertices = Vec::new();
            if let Some(vertex_attribute) = reader.read_positions() {
                vertex_attribute.for_each(|vertex| {
                    vertices.push(ModelVertex {
                        position: vertex,
                        tex_coords: Default::default(),
                        normal: Default::default(),
                    });
                });
            }
            if let Some(normal_attribute) = reader.read_normals() {
                let mut normal_index = 0;
                normal_attribute.for_each(|normal| {
                    vertices[normal_index].normal = normal;
                    normal_index += 1;
                });
            }
            if let Some(tex_coord_attribute) = reader.read_tex_coords(0).map(|v| v.into_f32()) {
                let mut tex_coord_index = 0;
                tex_coord_attribute.for_each(|tex_coord| {
                    vertices[tex_coord_index].tex_coords = tex_coord;
                    tex_coord_index += 1;
                });
            }

            let mut indices = Vec::new();
            if let Some(indices_raw) = reader.read_indices() {
                indices.append(&mut indices_raw.into_u32().collect::<Vec<u32>>());
            }

            let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(&format!("{:?} Vertex Buffer", file_name)),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(&format!("{:?} Index Buffer", file_name)),
                contents: bytemuck::cast_slice(&indices),
                usage: wgpu::BufferUsages::INDEX,
            });

            let transform = Transform::new(Vector3::zero(), Quaternion::zero(), Vector3::new(1.0, 1.0, 1.0));
            let transform_matrix = transform.to_matrix_bufferable();
            let transform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(&format!("Transform Buffer")),
                contents: bytemuck::cast_slice(&[transform_matrix]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });

            let transform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("transform bind group"),
                layout: bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: transform_buffer.as_entire_binding(),
                    },
                ],
            });

            meshes.push(model::Mesh {
                name: file_name.to_string(),
                vertex_buffer,
                index_buffer,
                num_elements: indices.len() as u32,
                material: primitive.material().index().unwrap_or(0),
                transform_buffer,
                transform_bind_group,
                transform,
            });
        }
    }

    // Process child nodes recursively
    for child in node.children() {
        process_node(&child, gltf, buffer_data, device, queue, bind_group_layout, meshes, file_name)?;
    }

    Ok(())
}