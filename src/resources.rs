use std::{collections::HashMap, path::Path};
use gltf::{image,  Gltf};
use nalgebra::{vector, Quaternion, Unit, Vector3};
use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use ron::from_str;
use tokio::task;
use wgpu::{util::DeviceExt, Buffer, Device};

use crate::{app::App, game_nodes::{game_object::{self, GameObject}, scene::Scene}, rendering::{instance_management::{InstanceData, InstanceRaw, ModelDataInstance}, model::{self, Mesh, Model, ModelVertex}, textures::Texture}, transform::Transform};

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

/// Loads a skybox cubemap from 6 face image files, in +X, -X, +Y, -Y, +Z, -Z order.
pub async fn load_texture_cube(file_names: [&str; 6], device: &wgpu::Device, queue: &wgpu::Queue) -> anyhow::Result<Texture> {
    let mut face_bytes: Vec<Vec<u8>> = Vec::with_capacity(6);
    for file_name in file_names {
        face_bytes.push(load_binary(file_name).await?);
    }
    let face_slices: [&[u8]; 6] = std::array::from_fn(|i| face_bytes[i].as_slice());
    Texture::from_cube_bytes(face_slices, device, queue, "skybox_cube_texture")
}

pub async fn load_model_gltf(file_name: &str, device: &wgpu::Device, queue: &wgpu::Queue, transform_bind_group_layout: &wgpu::BindGroupLayout) -> anyhow::Result<Model> {
    // Gltf::from_slice auto-detects the format from the header, so this handles
    // both text .gltf (JSON) and binary .glb files.
    let gltf_data = load_binary(file_name).await?;
    let gltf = Gltf::from_slice(&gltf_data).unwrap();

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
                let file_dir = Path::new(file_name).parent().unwrap_or(Path::new(""));
                let full_path = file_dir.join(uri);
                let bin = load_binary(full_path.to_str().unwrap()).await?;
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

        let texture_source = pbr
            .base_color_texture()
            .map(|tex| tex.texture().source().source());

        let Some(texture_source) = texture_source else {
            // No base color texture assigned (e.g. a flat-color/procedural material) -
            // fall back to a solid-color texture built from the material's base_color_factor.
            let [r, g, b, a] = pbr.base_color_factor();
            let solid_texture = Texture::from_image(
                &::image::DynamicImage::ImageRgba8(::image::RgbaImage::from_pixel(
                    1,
                    1,
                    ::image::Rgba([
                        (r * 255.0) as u8,
                        (g * 255.0) as u8,
                        (b * 255.0) as u8,
                        (a * 255.0) as u8,
                    ]),
                )),
                device,
                queue,
                Some("solid_color_texture"),
            ).expect("Couldn't create solid color texture");

            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                layout: &bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&solid_texture.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&solid_texture.sampler),
                    },
                ],
                label: None,
            });

            materials.push(model::Material {
                name: material.name().unwrap_or("Default Material").to_string(),
                diffuse_texture: solid_texture,
                bind_group,
            });

            continue;
        };

        match texture_source {
            gltf::image::Source::View { view, .. } => {
                    let buffer = &buffer_data[view.buffer().index()];
                    let start = view.offset();
                    let end = start + view.length();
                    let diffuse_texture = Texture::from_bytes(
                        &buffer[start..end],
                        device,
                        queue,
                        file_name,
                    )
                    .expect("Couldn't load diffuse");
                    
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
            image::Source::Uri { uri, mime_type: _ } => {
                let file_dir = Path::new(file_name).parent().unwrap_or(Path::new(""));

                // Join the GLTF directory with the URI to get the correct path.
                let full_path = file_dir.join(uri);
                let diffuse_texture = load_texture(full_path.to_str().unwrap(), device, queue).await?;

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
            },
        };
    }

    // Primitives with no material assigned (valid glTF - it means "use the default material")
    // fall back to this plain white material so they still render instead of indexing out of bounds.
    let default_material_index = materials.len();
    let default_texture = Texture::from_image(
        &::image::DynamicImage::ImageRgba8(::image::RgbaImage::from_pixel(1, 1, ::image::Rgba([255, 255, 255, 255]))),
        device,
        queue,
        Some("default_texture"),
    ).expect("Couldn't create default texture");
    let default_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&default_texture.view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&default_texture.sampler),
            },
        ],
        label: Some("default_material_bind_group"),
    });
    materials.push(model::Material {
        name: "Default Material".to_string(),
        diffuse_texture: default_texture,
        bind_group: default_bind_group,
    });

    let mut mesh_lists = HashMap::new();

    for scene in gltf.scenes() {
        for node in scene.nodes() {
            traverse_node(node, &buffer_data, device, queue, transform_bind_group_layout, &mut mesh_lists, file_name, None, default_material_index)?;
        }
    }

    Ok(model::Model {
        mesh_lists,
        materials,
    })
}

fn traverse_node(node: gltf::Node<'_>, buffer_data: &[Vec<u8>], device: &wgpu::Device, queue: &wgpu::Queue, transform_bind_group_layout: &wgpu::BindGroupLayout, mesh_lists: &mut HashMap<String, HashMap<String, Mesh>>, file_name: &str, parent_transform: Option<([f32; 3], [f32; 4], [f32; 3])>, default_material_index: usize) -> anyhow::Result<()> {
        let mesh = node.mesh().expect("Got mesh");
        let primitives = mesh.primitives();
        primitives.for_each(|primitive| {
            let reader = primitive.reader(|buffer| Some(&buffer_data[buffer.index()]));

            let mut vertices = Vec::new();
                if let Some(vertex_attribute) = reader.read_positions() {
                    vertex_attribute.for_each(|vertex| {
                        vertices.push(ModelVertex {
                            position: vertex,
                            tex_coords: Default::default(),
                            normal: Default::default(),
                        })
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

            let transform: Transform;
            let mut parent_values = None;

            match parent_transform {
                Some(parent_data) => {
                    let (parent_translation, parent_rotation, parent_scale) = parent_data;
                    let (translation, rotation, _scale) = node.transform().decomposed();

                    let position = Vector3::from(parent_translation) + Vector3::from(translation);
                    let rotation = Quaternion::from(parent_rotation) * Quaternion::from(rotation);
                    transform = Transform::new(position, rotation, Vector3::new(1.0, 1.0, 1.0));
                    parent_values = Some(Transform::new(parent_translation.into(), parent_rotation.into(), parent_scale.into()));
                },
                None => {
                    transform = Transform::new(node.transform().decomposed().0.into(), node.transform().decomposed().1.into(), Vector3::new(1.0, 1.0, 1.0));
                },
            }

            let transform_matrix = transform.to_matrix_bufferable();
            let transform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(&format!("Transform Buffer")),
                contents: bytemuck::cast_slice(&[transform_matrix]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });

            let transform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("transform bind group"),
                layout: &transform_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: transform_buffer.as_entire_binding(),
                    },
                ],
            });

            let mesh = model::Mesh {
                name: file_name.to_string(),
                vertex_buffer,
                index_buffer,
                num_elements: indices.len() as u32,
                material: primitive.material().index().unwrap_or(default_material_index),
                transform_buffer,
                transform_bind_group,
                transform,
                base_transform: transform,
                parent_transform: parent_values,
                alpha_mode: primitive.material().alpha_mode(),
            };

            if primitive.material().alpha_mode() == gltf::material::AlphaMode::Blend || primitive.material().alpha_mode() == gltf::material::AlphaMode::Mask {
                add_or_init_mesh_list(mesh_lists, &"transparent".to_string(), node.name().unwrap().to_owned(), mesh);
            } else {
                add_or_init_mesh_list(mesh_lists, &"opaque".to_string(), node.name().unwrap().to_owned(), mesh);
            }
            
        });
    for child in node.children() {
        traverse_node(child, buffer_data, device, queue, transform_bind_group_layout, mesh_lists, file_name, Some(node.transform().decomposed()), default_material_index)?;
    }

    Ok(())
}

/// # Add or init mesh list
/// This function is used to create a mesh_list, here we define a list and if it exists we add data, else we create it and add data later
fn add_or_init_mesh_list(mesh_lists: &mut HashMap<String, HashMap<String, Mesh>>, list_name: &String, key: String, mesh_to_add: Mesh) {
    match mesh_lists.get_mut(list_name) {
        Some(inner_mesh_list) => {
            inner_mesh_list.insert(key, mesh_to_add);
        },
        None => {
            mesh_lists.insert(list_name.to_string(), HashMap::new());
            add_or_init_mesh_list(mesh_lists, list_name, key, mesh_to_add);
        },
    }
}

pub fn load_level(app: &mut App, mut level_path: String) {

    app.scene_openned = Some(level_path.clone());
    level_path += "/data.ron";

    // i get the json data
    app.renderizable_instances = HashMap::new();

    for (_key, model) in &mut app.game_models {
        model.instance_count = 0;
    }

    let instances_data_to_load = load_instances(level_path);
    match instances_data_to_load {
        Some(instances) => {
            // models to load
            let mut models: Vec<String> = vec![];

            for data in &instances {
                if !models.contains(&data.model.to_string()) {
                    models.push(data.model.to_string())
                }
            }

            // we get all data id and game_objects
            for model_name in &models {
                let mut ids: Vec<String> = vec![];
                let mut model_instances:Vec<&GameObject> = vec![];

                for game_object in &instances {
                    if &game_object.model == model_name {
                        ids.push(game_object.id.clone());
                        model_instances.push(game_object);
                    }
                }

                // Create instance buffer once per model
                let instance_buffer = create_instance_buffer(&model_instances, &app.device, app.camera.camera.position.coords);

                for (i, instance_data) in model_instances.iter().enumerate() {
                    match app.game_models.get_mut(model_name) {
                        Some(model_data) => {
                            model_data.instance_count += 1;
                            // Update the instance buffer with the new one created above
                            model_data.instance_buffer = instance_buffer.clone();
                        },
                        None => {
                            let model = task::block_in_place( || {
                                tokio::runtime::Runtime::new()
                                    .unwrap()
                                    .block_on(load_model_gltf(&model_name, &app.device, &app.queue, &app.transform_bind_group_layout))
                            });

                            match model {
                                Ok(correct_model) => {
                                    app.game_models.insert(
                                        model_name.to_string(), 
                                        ModelDataInstance {
                                            model: correct_model,
                                            instance_count: 1,
                                            instance_buffer: instance_buffer.clone()
                                        }
                                    );
                                },
                                Err(e) => eprintln!("The element was not loaded as an instance: {}", e),
                            }
                        },
                    }

                    // println!("loaded data: {}", ids[i]);
                    app.renderizable_instances.insert(ids[i].clone(), InstanceData { renderizable_transform: instance_data.transform.clone(), instance: (**instance_data).clone(), model_ref: model_name.clone() });
                }
            }
        },
        None => eprintln!("The instance data was not correctly loaded"),
    }
}

pub fn create_instance_buffer(instances: &Vec<&GameObject>, device: &Device, camera_position: Vector3<f32>) -> Buffer {
    let raw_instances: Vec<InstanceRaw> = instances.iter()
    .map(|instance| instance.transform.to_raw(camera_position))
    .collect();

    device.create_buffer_init(
        &wgpu::util::BufferInitDescriptor {
            label: Some("Instance Buffer"),
            contents: bytemuck::cast_slice(&raw_instances),
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        }
    )
}

pub fn load_instances(path: String) -> Option<Vec<GameObject>> {
    match std::fs::read_to_string(path) {
        Ok(file_contents) => {
            match from_str::<Scene>(&file_contents) {
                Ok(level) => {
                    return Some(level.children);
                },
                Err(e) => {
                    // Handle the error if deserialization fails
                    eprintln!("Error deserializing RON: {}", e);
                }
            }
        },
        _ => {}
    }
    return None
}
