use std::{collections::HashMap, collections::HashSet, path::Path};
use gltf::{image,  Gltf};
use nalgebra::{vector, Quaternion, Unit, UnitQuaternion, Vector3};
use rapier3d::prelude::{ColliderBuilder, RigidBodyBuilder};
use ron::from_str;
use wgpu::{util::DeviceExt, BindGroupLayout, Buffer, Device, Queue, SurfaceConfiguration};

use crate::{app::App, engine::game_nodes::{game_object::GameObject, scene::Scene}, engine::rendering::{enviroment::environment::Environment, enviroment::skybox_renderer::SkyboxRender, instance_management::{InstanceData, InstanceRaw, ModelDataInstance}, models::model::{self, Mesh, Model, ModelVertex}, models::textures::Texture}, transform::Transform};
use crate::engine::scene_manager::scene::Scene as ManagedScene;

pub fn load_binary(file_name: &str) -> anyhow::Result<Vec<u8>> {
    let path = std::path::Path::new(env!("OUT_DIR"))
    .join("res")
    .join(file_name);
    let data = std::fs::read(path)?;
    Ok(data)
}

/// Reads a file relative to the project's assets/ directory - unlike load_binary
/// (which reads from OUT_DIR/res/, a build-time copy baked in at compile time),
/// this reads directly off the filesystem at runtime, the same convention
/// Ui::open_ui already uses for .ron UI files. Used for content meant to be edited
/// and picked up without a rebuild (UI images, UI/level definitions, ...).
pub fn load_asset_binary(file_name: &str) -> std::io::Result<Vec<u8>> {
    std::fs::read(std::path::Path::new("assets").join(file_name))
}

pub fn load_texture(file_name: &str, device: &wgpu::Device, queue: &wgpu::Queue) -> anyhow::Result<Texture> {
    let data = load_binary(file_name)?;
    Texture::from_bytes(&data, device, queue, file_name)
}

/// Loads a skybox cubemap from 6 face image files, in +X, -X, +Y, -Y, +Z, -Z order.
pub fn load_texture_cube(file_names: [&str; 6], device: &wgpu::Device, queue: &wgpu::Queue) -> anyhow::Result<Texture> {
    let mut face_bytes: Vec<Vec<u8>> = Vec::with_capacity(6);
    for file_name in file_names {
        face_bytes.push(load_binary(file_name)?);
    }
    let face_slices: [&[u8]; 6] = std::array::from_fn(|i| face_bytes[i].as_slice());
    Texture::from_cube_bytes(face_slices, device, queue, "skybox_cube_texture")
}

pub fn load_model_gltf(file_name: &str, device: &wgpu::Device, queue: &wgpu::Queue, transform_bind_group_layout: &wgpu::BindGroupLayout) -> anyhow::Result<Model> {
    // Gltf::from_slice auto-detects the format from the header, so this handles
    // both text .gltf (JSON) and binary .glb files.
    let gltf_data = load_binary(file_name)?;
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
                let bin = load_binary(full_path.to_str().unwrap())?;
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
                let diffuse_texture = load_texture(full_path.to_str().unwrap(), device, queue)?;

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

/// Extracts a mesh's raw triangle geometry (world-space vertex positions +
/// triangle indices, no GPU upload, no materials) from a `.gltf`/`.glb` file -
/// for building a `game_object::ColliderType::Trimesh` from a model's own
/// shape instead of a hand-authored primitive (see that variant's own doc
/// comment). Reads the file independently of `load_model_gltf`'s own
/// GPU-facing load rather than reusing its result - a `Mesh` only keeps
/// `wgpu::Buffer`s once loaded (see that struct's own fields), nothing
/// CPU-readable survives to reuse here, so rendering + physics both wanting
/// the same model means a small amount of duplicate file I/O, paid once at
/// scene-construction time.
pub fn load_trimesh_geometry(file_name: &str) -> anyhow::Result<(Vec<Vector3<f32>>, Vec<[u32; 3]>)> {
    let gltf_data = load_binary(file_name)?;
    let gltf = Gltf::from_slice(&gltf_data).unwrap();

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
                let bin = load_binary(full_path.to_str().unwrap())?;
                buffer_data.push(bin);
            }
        }
    }

    let mut vertices: Vec<Vector3<f32>> = Vec::new();
    let mut indices: Vec<[u32; 3]> = Vec::new();

    for scene in gltf.scenes() {
        for node in scene.nodes() {
            collect_trimesh_geometry(node, &buffer_data, None, &mut vertices, &mut indices);
        }
    }

    Ok((vertices, indices))
}

// Recurses the same way traverse_node does (child transforms compose with
// their parent's), but only ever touches positions/indices - no GPU device,
// no materials, since this is only ever building collision geometry, not
// anything rendered.
fn collect_trimesh_geometry(
    node: gltf::Node<'_>,
    buffer_data: &[Vec<u8>],
    parent_transform: Option<(Vector3<f32>, UnitQuaternion<f32>)>,
    vertices: &mut Vec<Vector3<f32>>,
    indices: &mut Vec<[u32; 3]>,
) {
    let (translation, rotation, _scale) = node.transform().decomposed();
    let local_translation = Vector3::from(translation);
    let local_rotation = UnitQuaternion::from_quaternion(Quaternion::from(rotation));

    let (world_translation, world_rotation) = match parent_transform {
        Some((parent_translation, parent_rotation)) => (
            parent_translation + parent_rotation * local_translation,
            parent_rotation * local_rotation,
        ),
        None => (local_translation, local_rotation),
    };

    if let Some(mesh) = node.mesh() {
        for primitive in mesh.primitives() {
            let reader = primitive.reader(|buffer| Some(&buffer_data[buffer.index()]));

            let base_index = vertices.len() as u32;
            if let Some(positions) = reader.read_positions() {
                for position in positions {
                    vertices.push(world_translation + world_rotation * Vector3::from(position));
                }
            }
            if let Some(indices_raw) = reader.read_indices() {
                let flat: Vec<u32> = indices_raw.into_u32().collect();
                for triangle in flat.chunks_exact(3) {
                    indices.push([base_index + triangle[0], base_index + triangle[1], base_index + triangle[2]]);
                }
            }
        }
    }

    for child in node.children() {
        collect_trimesh_geometry(child, buffer_data, Some((world_translation, world_rotation)), vertices, indices);
    }
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

/// Pure (no `&mut App`) version of the old `load_level` - safe to call from a
/// background thread given cloned `Device`/`Queue` handles (both cheap, Arc-backed,
/// and `Clone` in wgpu). `existing_models` is a snapshot of `app.game_models.keys()`
/// taken before this call, so already-loaded models are reused instead of reloaded,
/// same "load a model's GLTF once, keep it cached across scenes" behavior the
/// original had. Currently unused - kept alongside `load_instances`/
/// `physics_resources::load_physics_from_level` as available (data.ron-based)
/// infrastructure now that every registered scene spawns nodes directly instead
/// (see `SceneManager::create_loaded_scene`'s own doc comment for the code-first
/// equivalent of the background-load split this used to provide).
#[allow(dead_code)]
struct PreparedLevel {
    /// Freshly gltf-loaded models, keyed by model_ref - only for models that weren't
    /// already in `existing_models` when this was prepared.
    new_models: HashMap<String, ModelDataInstance>,
    /// For models that WERE already resident: the freshly-built instance buffer + count
    /// to swap onto the existing `ModelDataInstance` entry.
    instance_buffer_updates: HashMap<String, (Buffer, u32)>,
    renderizable_instances: HashMap<String, InstanceData>,
}

#[allow(dead_code)]
fn prepare_level_assets(device: &Device, queue: &Queue, camera_position: Vector3<f32>, existing_models: &HashSet<String>, mut level_path: String) -> PreparedLevel {
    level_path += "/data.ron";

    let mut new_models: HashMap<String, ModelDataInstance> = HashMap::new();
    let mut instance_buffer_updates: HashMap<String, (Buffer, u32)> = HashMap::new();
    let mut renderizable_instances: HashMap<String, InstanceData> = HashMap::new();

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
                let instance_buffer = create_instance_buffer(&model_instances, device, camera_position);
                let instance_count = model_instances.len() as u32;

                if existing_models.contains(model_name) {
                    instance_buffer_updates.insert(model_name.clone(), (instance_buffer, instance_count));
                } else {
                    match load_model_gltf(model_name, device, queue, &Mesh::create_bind_group_layout(device)) {
                        Ok(correct_model) => {
                            new_models.insert(
                                model_name.to_string(),
                                ModelDataInstance {
                                    model: correct_model,
                                    instance_count,
                                    instance_buffer,
                                }
                            );
                        },
                        Err(e) => eprintln!("The element was not loaded as an instance: {}", e),
                    }
                }

                for (i, instance_data) in model_instances.iter().enumerate() {
                    renderizable_instances.insert(ids[i].clone(), InstanceData { renderizable_transform: instance_data.transform.clone(), instance: (**instance_data).clone(), model_ref: model_name.clone() });
                }
            }
        },
        None => eprintln!("The instance data was not correctly loaded"),
    }

    PreparedLevel { new_models, instance_buffer_updates, renderizable_instances }
}

pub struct PreparedEnvironment {
    pub skybox: Option<SkyboxRender>,
    pub clear_color: wgpu::Color,
}

/// Pure (no `&mut App`) version of the old `apply_environment` - see
/// `prepare_level_assets` for why this shape is safe to call off the main thread.
/// `camera_bind_group_layout` is the one piece `SkyboxRender::new` needs out of the
/// full `CameraHandler` (see that fn's own comment).
pub fn prepare_environment(device: &Device, queue: &Queue, camera_bind_group_layout: &BindGroupLayout, config: &SurfaceConfiguration, environment: Environment) -> PreparedEnvironment {
    match environment {
        Environment::Color(color) => PreparedEnvironment { skybox: None, clear_color: color },
        Environment::Skybox(faces) => {
            let texture = load_texture_cube(
                [&faces.px, &faces.nx, &faces.py, &faces.ny, &faces.pz, &faces.nz],
                device,
                queue,
            );

            match texture {
                Ok(texture) => PreparedEnvironment {
                    skybox: Some(SkyboxRender::new(device, config, camera_bind_group_layout, texture)),
                    clear_color: crate::engine::rendering::enviroment::environment::DEFAULT_CLEAR_COLOR,
                },
                Err(err) => {
                    eprintln!("Skybox faces couldn't be loaded, falling back to clear color: {err}");
                    PreparedEnvironment { skybox: None, clear_color: crate::engine::rendering::enviroment::environment::DEFAULT_CLEAR_COLOR }
                }
            }
        }
    }
}

/// Synchronous environment setup for scenes registered via `create_scene`
/// (no level to load alongside it, so no reason to defer to a background
/// thread the way `create_loaded_scene`/`PreparedSceneAssets::apply` do) -
/// `prepare_environment` plus applying its result to `scene.environment` in
/// one call, instead of every such scene's own `new` repeating the
/// `camera_bind_group_layout` clone + apply steps by hand.
pub fn apply_environment(scene: &mut ManagedScene, app: &mut App, environment: Environment) {
    let camera_bind_group_layout = app.camera_resources.bind_group_layout.clone();
    let prepared = prepare_environment(&app.renderer.device, &app.renderer.queue, &camera_bind_group_layout, &app.renderer.config, environment);
    scene.environment.skybox = prepared.skybox;
    scene.environment.clear_color = prepared.clear_color;
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

// Named resource registration - register once, up front (e.g. right after
// App::new in main.rs, before any scene opens), then reference the chosen
// name from wherever a resource's actually needed instead of repeating its
// file path. Not tied to any particular scene - a name registered here is
// available to every scene equally.

/// Registers a model's gltf once under a chosen `name`, decoupling the short
/// name referenced throughout game code (a Node's `Model { model_ref: name }`
/// - see `engine::scene_manager::render_bridge::register_static_model`, and
/// `App::spawn_node` for how a spawned `Model` property triggers that
/// automatically) from the actual asset path, which only needs to be written
/// once, right here.
///
/// Stores into `app.loaded_models`, not `app.game_models` directly - a model
/// only gets a real `ModelDataInstance` (which owns a GPU instance buffer)
/// once something actually instances it; see `loaded_models`'s own doc
/// comment on `App` for why an empty one can't just be created here instead.
pub fn register_model(app: &mut App, name: &str, path: &str) -> Result<(), String> {
    if app.loaded_models.contains_key(name) || app.game_models.contains_key(name) {
        return Err(format!("a model named '{name}' is already loaded"));
    }

    let bind_group_layout = Mesh::create_bind_group_layout(&app.renderer.device);
    let loaded_model = load_model_gltf(path, &app.renderer.device, &app.renderer.queue, &bind_group_layout)
        .map_err(|e| format!("failed to load model '{name}' from '{path}': {e}"))?;

    app.loaded_models.insert(name.to_owned(), loaded_model);
    Ok(())
}

/// A procedurally-generated shape for `register_primitive_model` - the
/// code-first alternative to authoring a model in Blender and loading it via
/// `register_model`, for a quick test mesh (a subdivided plane to try a water
/// shader's vertex displacement against, without round-tripping through a
/// .glb file every time the subdivision count changes).
pub enum PrimitiveShape {
    /// A unit cube (-0.5..0.5 on each axis) - scale it via a node's own
    /// `Transform3D::scale`, same as every other model.
    Cube,
    /// A unit square (-0.5..0.5 on X/Z, Y=0, normal +Y) - `subdivisions` is
    /// how many extra cuts per side beyond the base single quad (0 = one
    /// quad/4 vertices, 1 = a 2x2 grid/9 vertices, ...), so a shader driving
    /// per-vertex displacement (waves, ...) has geometry to actually move.
    Plane { subdivisions: u32 },
}

/// Builds a `Cube`'s raw geometry - 4 vertices per face (24 total) rather
/// than 8 shared ones, since each face needs its own flat normal/UV, which a
/// shared-corner vertex can't hold. Winding is CCW as viewed from outside
/// each face, matching the engine's `FrontFace::Ccw` + back-face culling
/// convention (see e.g. `light.rs`'s pipeline).
fn build_cube_mesh() -> (Vec<ModelVertex>, Vec<u32>) {
    // (normal, corners) per face - corners already in CCW-from-outside order.
    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        ([0.0, 0.0, 1.0], [[-0.5, -0.5, 0.5], [0.5, -0.5, 0.5], [0.5, 0.5, 0.5], [-0.5, 0.5, 0.5]]),
        ([0.0, 0.0, -1.0], [[0.5, -0.5, -0.5], [-0.5, -0.5, -0.5], [-0.5, 0.5, -0.5], [0.5, 0.5, -0.5]]),
        ([1.0, 0.0, 0.0], [[0.5, -0.5, 0.5], [0.5, -0.5, -0.5], [0.5, 0.5, -0.5], [0.5, 0.5, 0.5]]),
        ([-1.0, 0.0, 0.0], [[-0.5, -0.5, -0.5], [-0.5, -0.5, 0.5], [-0.5, 0.5, 0.5], [-0.5, 0.5, -0.5]]),
        ([0.0, 1.0, 0.0], [[-0.5, 0.5, 0.5], [0.5, 0.5, 0.5], [0.5, 0.5, -0.5], [-0.5, 0.5, -0.5]]),
        ([0.0, -1.0, 0.0], [[-0.5, -0.5, -0.5], [0.5, -0.5, -0.5], [0.5, -0.5, 0.5], [-0.5, -0.5, 0.5]]),
    ];
    let uvs = [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]];

    let mut vertices = Vec::with_capacity(24);
    let mut indices = Vec::with_capacity(36);
    for (normal, corners) in faces {
        let base = vertices.len() as u32;
        for (corner, uv) in corners.iter().zip(uvs.iter()) {
            vertices.push(ModelVertex { position: *corner, tex_coords: *uv, normal });
        }
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    (vertices, indices)
}

/// Builds a `Plane { subdivisions }`'s raw geometry - a `(subdivisions + 2)`-
/// per-side grid of shared vertices (a flat plane has no per-face normal/UV
/// seams to worry about, unlike `build_cube_mesh`), UVs spanning 0..1 across
/// the whole grid. Winding is CCW as viewed from +Y, matching `normal`.
fn build_plane_mesh(subdivisions: u32) -> (Vec<ModelVertex>, Vec<u32>) {
    let segments = subdivisions + 1;
    let verts_per_side = segments + 1;

    let mut vertices = Vec::with_capacity((verts_per_side * verts_per_side) as usize);
    for row in 0..verts_per_side {
        for col in 0..verts_per_side {
            let u = col as f32 / segments as f32;
            let v = row as f32 / segments as f32;
            vertices.push(ModelVertex { position: [u - 0.5, 0.0, v - 0.5], tex_coords: [u, v], normal: [0.0, 1.0, 0.0] });
        }
    }

    let mut indices = Vec::with_capacity((segments * segments * 6) as usize);
    for row in 0..segments {
        for col in 0..segments {
            let top_left = row * verts_per_side + col;
            let top_right = top_left + 1;
            let bottom_left = top_left + verts_per_side;
            let bottom_right = bottom_left + 1;
            indices.extend_from_slice(&[top_left, bottom_left, bottom_right, top_left, bottom_right, top_right]);
        }
    }
    (vertices, indices)
}

/// Registers a procedurally-generated `PrimitiveShape` once under a chosen
/// `name` - the same registration point/pattern as `register_model`
/// (`app.loaded_models`, referenced later via a node's `Model { model_ref:
/// name }`), just building the mesh's vertex/index data in code instead of
/// parsing it out of a .glb file. Uses a single flat-white material (see
/// `load_model_gltf`'s own default-material fallback for the same texture) -
/// swap it out per-instance with a real shader/material system once one
/// exists; for now this is meant for trying geometry (a water shader's
/// vertex displacement, ...) against, not for a shippable-looking primitive.
pub fn register_primitive_model(app: &mut App, name: &str, shape: PrimitiveShape) -> Result<(), String> {
    if app.loaded_models.contains_key(name) || app.game_models.contains_key(name) {
        return Err(format!("a model named '{name}' is already loaded"));
    }

    let device = &app.renderer.device;
    let queue = &app.renderer.queue;

    let (vertices, indices) = match shape {
        PrimitiveShape::Cube => build_cube_mesh(),
        PrimitiveShape::Plane { subdivisions } => build_plane_mesh(subdivisions),
    };

    let texture_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
        label: Some("primitive_texture_bind_group_layout"),
    });

    let default_texture = Texture::from_image(
        &::image::DynamicImage::ImageRgba8(::image::RgbaImage::from_pixel(1, 1, ::image::Rgba([255, 255, 255, 255]))),
        device,
        queue,
        Some("primitive_default_texture"),
    ).map_err(|e| format!("failed to build a default texture for primitive '{name}': {e}"))?;

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &texture_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&default_texture.view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&default_texture.sampler) },
        ],
        label: Some("primitive_material_bind_group"),
    });

    let materials = vec![model::Material { name: "Default Material".to_owned(), diffuse_texture: default_texture, bind_group }];

    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(&format!("{name} primitive vertex buffer")),
        contents: bytemuck::cast_slice(&vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(&format!("{name} primitive index buffer")),
        contents: bytemuck::cast_slice(&indices),
        usage: wgpu::BufferUsages::INDEX,
    });

    // Identity - a primitive's actual placement/size comes from its node's
    // own Transform3D, same as every mesh loaded via load_model_gltf.
    let transform = Transform::new(Vector3::zeros(), Quaternion::identity(), Vector3::new(1.0, 1.0, 1.0));
    let transform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("primitive transform buffer"),
        contents: bytemuck::cast_slice(&[transform.to_matrix_bufferable()]),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let transform_bind_group_layout = Mesh::create_bind_group_layout(device);
    let transform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("primitive transform bind group"),
        layout: &transform_bind_group_layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: transform_buffer.as_entire_binding() }],
    });

    let mesh = model::Mesh {
        name: name.to_owned(),
        vertex_buffer,
        index_buffer,
        num_elements: indices.len() as u32,
        material: 0,
        transform_buffer,
        transform_bind_group,
        transform,
        base_transform: transform,
        parent_transform: None,
        alpha_mode: gltf::material::AlphaMode::Opaque,
    };

    let mut opaque = HashMap::new();
    opaque.insert(name.to_owned(), mesh);
    let mut mesh_lists = HashMap::new();
    mesh_lists.insert("opaque".to_owned(), opaque);

    app.loaded_models.insert(name.to_owned(), model::Model { mesh_lists, materials });
    Ok(())
}

/// Registers an image once under a chosen `name` - the same pattern as
/// `register_model`, just simpler: a texture has no equivalent of a model's
/// shared per-instance GPU buffer to size/rebuild, so there's no "loaded but
/// not yet instanced" split needed - it goes straight into `app.textures`
/// and is immediately ready to use wherever `name` is referenced (a
/// material, a UI image, ...).
pub fn register_texture(app: &mut App, name: &str, path: &str) -> Result<(), String> {
    if app.textures.contains_key(name) {
        return Err(format!("a texture named '{name}' is already loaded"));
    }

    let texture = load_texture(path, &app.renderer.device, &app.renderer.queue)
        .map_err(|e| format!("failed to load texture '{name}' from '{path}': {e}"))?;

    app.textures.insert(name.to_owned(), texture);
    Ok(())
}
