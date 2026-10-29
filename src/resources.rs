use std::{collections::HashMap, collections::HashSet, path::Path};
use gltf::{image,  Gltf};
use nalgebra::{Quaternion, UnitQuaternion, Vector3};
use wgpu::{util::DeviceExt, BindGroupLayout, Buffer, Device, Queue, SurfaceConfiguration};
use serde::Deserialize;

use crate::{app::App, asset_manager::{asset_path, normalize}, game_nodes::game_object::GameObject, rendering::{enviroment::environment::{clear_color, Environment, PreparedEnvironment, DEFAULT_CLEAR_COLOR}, enviroment::skybox_renderer::SkyboxRender, instance_management::InstanceRaw, models::model::{self, Mesh, Model}, models::textures::Texture}, transform::Transform};
/// What `AssetManager::add_mesh` takes - one vertex: position, texture
/// coordinates, normal.
pub use crate::rendering::models::model::ModelVertex;

/// Flip to `true` to dump every model's materials/textures/primitives to
/// stdout as it loads (see `model_log!` uses in `load_model_with` /
/// `traverse_node`).
const DEBUG_MODEL_LOAD: bool = false;

macro_rules! model_log {
    ($($arg:tt)*) => { if DEBUG_MODEL_LOAD { println!($($arg)*); } };
}

/// Which of a model's meshes skip backface culling - `DoubleSided`, owned,
/// so it can be stored with a model's source until it's loaded (see
/// `Model::double_sided`).
#[derive(Debug, Clone, Default, Deserialize)]
pub enum DoubleSidedMeshes {
    #[default]
    None,
    All,
    Meshes(Vec<String>),
}

/// Where a model comes from - a node's `Model::new(source)`. Whichever it
/// is, it's loaded with the scene that uses it (under the loading screen) and
/// shared by every node using the same one.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelSource {
    /// A model file (`.glb`/`.gltf`) in the assets folder, by its path there -
    /// `ModelSource::file("models/f16.glb")`.
    File(String),
    /// A mesh the game made, by the name it was added under (see
    /// `AssetManager::add_mesh`) - `ModelSource::mesh("terrain")`.
    Mesh(String),
    /// A 1 x 1 x 1 cube, centered on its node.
    Cube,
    /// A 1 x 1 square across X and Z, facing up, centered on its node -
    /// `subdivisions` extra cuts along each side (0: one square), for a
    /// shader that moves its vertices (waves) to have some to move.
    Plane { subdivisions: u32 },
}

impl ModelSource {
    /// The model file at `path` inside the assets folder.
    pub fn file(path: impl Into<String>) -> Self {
        Self::File(normalize(&path.into()))
    }

    /// The mesh the game added as `name` (see `AssetManager::add_mesh`).
    pub fn mesh(name: impl Into<String>) -> Self {
        Self::Mesh(name.into())
    }

    /// What its loaded copy is kept under (`ModelCache`) - the same for every
    /// node using the same one, so it's loaded once.
    pub fn key(&self) -> String {
        match self {
            Self::File(path) => normalize(path),
            Self::Mesh(name) => format!("mesh:{name}"),
            Self::Cube => "engine:cube".to_owned(),
            Self::Plane { subdivisions } => format!("engine:plane:{subdivisions}"),
        }
    }
}

/// A mesh the game made - see `AssetManager::add_mesh`. Kept as it was given
/// while the game runs; each scene that uses it builds its own GPU copy.
#[derive(Debug)]
pub struct MeshData {
    pub vertices: Vec<ModelVertex>,
    pub indices: Vec<u32>,
}

impl MeshData {
    /// Whether its triangles make sense - `name` is for the message.
    pub(crate) fn check(&self, name: &str) -> Result<(), String> {
        if self.indices.len() % 3 != 0 {
            return Err(format!("mesh '{name}': {} indices isn't a whole number of triangles", self.indices.len()));
        }
        if let Some(index) = self.indices.iter().find(|index| **index as usize >= self.vertices.len()) {
            return Err(format!("mesh '{name}': index {index} is past its {} vertices", self.vertices.len()));
        }
        Ok(())
    }
}

/// One model to load - the shared copy of a `ModelSource`, with what it
/// needs to be built off the main thread (a scene's loading thread). Made by
/// `AssetManager::model_load`.
#[derive(Clone)]
pub(crate) struct ModelLoad {
    pub(crate) key: String,
    geometry: LoadGeometry,
    double_sided: DoubleSidedMeshes,
}

#[derive(Clone)]
enum LoadGeometry {
    /// Where the file is on disk - its buffers and textures are next to it.
    File(std::path::PathBuf),
    Mesh(std::sync::Arc<MeshData>),
    Cube,
    Plane(u32),
}

impl ModelLoad {
    pub(crate) fn file(key: String, path: std::path::PathBuf, double_sided: DoubleSidedMeshes) -> Self {
        Self { key, geometry: LoadGeometry::File(path), double_sided }
    }

    pub(crate) fn mesh(key: String, mesh: std::sync::Arc<MeshData>, double_sided: DoubleSidedMeshes) -> Self {
        Self { key, geometry: LoadGeometry::Mesh(mesh), double_sided }
    }

    pub(crate) fn cube(key: String, double_sided: DoubleSidedMeshes) -> Self {
        Self { key, geometry: LoadGeometry::Cube, double_sided }
    }

    pub(crate) fn plane(key: String, subdivisions: u32, double_sided: DoubleSidedMeshes) -> Self {
        Self { key, geometry: LoadGeometry::Plane(subdivisions), double_sided }
    }

    /// Reads or builds the model and uploads it.
    pub(crate) fn load(&self, device: &wgpu::Device, queue: &wgpu::Queue, transform_bind_group_layout: &wgpu::BindGroupLayout) -> anyhow::Result<Model> {
        match &self.geometry {
            LoadGeometry::File(path) => {
                let names: Vec<&str> = match &self.double_sided {
                    DoubleSidedMeshes::Meshes(names) => names.iter().map(String::as_str).collect(),
                    _ => Vec::new(),
                };
                let double_sided = match &self.double_sided {
                    DoubleSidedMeshes::None => DoubleSided::None,
                    DoubleSidedMeshes::All => DoubleSided::All,
                    DoubleSidedMeshes::Meshes(_) => DoubleSided::Meshes(&names),
                };
                let file_name = path.to_string_lossy().into_owned();
                load_model_with(&file_name, &|file: &Path| Ok(std::fs::read(file)?), device, queue, transform_bind_group_layout, double_sided)
            }
            LoadGeometry::Mesh(mesh) => self.build(&mesh.vertices, &mesh.indices, device, queue, transform_bind_group_layout),
            LoadGeometry::Cube => {
                let (vertices, indices) = build_cube_mesh();
                self.build(&vertices, &indices, device, queue, transform_bind_group_layout)
            }
            LoadGeometry::Plane(subdivisions) => {
                let (vertices, indices) = build_plane_mesh(*subdivisions);
                self.build(&vertices, &indices, device, queue, transform_bind_group_layout)
            }
        }
    }

    fn build(&self, vertices: &[ModelVertex], indices: &[u32], device: &wgpu::Device, queue: &wgpu::Queue, transform_bind_group_layout: &wgpu::BindGroupLayout) -> anyhow::Result<Model> {
        let double_sided = match &self.double_sided {
            DoubleSidedMeshes::None => false,
            DoubleSidedMeshes::All => true,
            DoubleSidedMeshes::Meshes(names) => names.iter().any(|name| *name == self.key),
        };
        build_mesh_model(&self.key, vertices, indices, double_sided, device, queue, transform_bind_group_layout).map_err(anyhow::Error::msg)
    }
}

/// Reads a file inside the assets folder (see `AssetFolder::set_root`) - UI
/// images, particle textures, skybox faces...
pub fn load_asset_binary(file_name: &str) -> std::io::Result<Vec<u8>> {
    std::fs::read(asset_path(file_name))
}

/// Loads a skybox cubemap from 6 face image files inside the assets folder,
/// in +X, -X, +Y, -Y, +Z, -Z order.
pub fn load_texture_cube(file_names: [&str; 6], device: &wgpu::Device, queue: &wgpu::Queue) -> anyhow::Result<Texture> {
    let mut face_bytes: Vec<Vec<u8>> = Vec::with_capacity(6);
    for file_name in file_names {
        face_bytes.push(load_asset_binary(file_name)?);
    }
    let face_slices: [&[u8]; 6] = std::array::from_fn(|i| face_bytes[i].as_slice());
    Texture::from_cube_bytes(face_slices, device, queue, "skybox_cube_texture")
}

/// A linear color channel (0..1) encoded to sRGB (0..1) - the standard
/// transfer curve.
fn linear_to_srgb(linear: f32) -> f32 {
    let linear = linear.clamp(0.0, 1.0);
    if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055 * linear.powf(1.0 / 2.4) - 0.055
    }
}

/// Loads a `.gltf`/`.glb` through `read` - which turns a path (the model's
/// own `file_name`, or one of its buffers/textures, resolved next to it)
/// into bytes.
fn load_model_with(file_name: &str, read: &dyn Fn(&Path) -> anyhow::Result<Vec<u8>>, device: &wgpu::Device, queue: &wgpu::Queue, transform_bind_group_layout: &wgpu::BindGroupLayout, double_sided: DoubleSided) -> anyhow::Result<Model> {
    // Gltf::from_slice auto-detects the format from the header, so this handles
    // both text .gltf (JSON) and binary .glb files.
    let gltf_data = read(Path::new(file_name))?;
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
                let bin = read(&full_path)?;
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
    
    model_log!(
        "[model {}] {} material(s), {} mesh(es), {} node(s)",
        file_name,
        gltf.materials().count(),
        gltf.meshes().count(),
        gltf.nodes().count(),
    );

    let mut materials = Vec::new();
    // Every image decoded once, by its index in the file - many materials
    // share one (the F-16's 64 materials use 10), and decoding a PNG is by
    // far the slowest part of loading a model. Textures are GPU handles,
    // so sharing one is just a clone.
    let mut decoded: HashMap<usize, Texture> = HashMap::new();
    for (mat_index, material) in gltf.materials().enumerate() {
        let pbr = material.pbr_metallic_roughness();

        let texture_source = pbr
            .base_color_texture()
            .map(|tex| (tex.texture().source().index(), tex.texture().source().source()));

        let Some((image_index, texture_source)) = texture_source else {
            // No base color texture assigned (e.g. a flat-color/procedural material) -
            // fall back to a solid-color texture built from the material's base_color_factor.
            let [r, g, b, a] = pbr.base_color_factor();
            model_log!(
                "[model {}]   material {} \"{}\": no base-color texture -> solid color rgba({:.2}, {:.2}, {:.2}, {:.2})",
                file_name, mat_index, material.name().unwrap_or("<unnamed>"), r, g, b, a,
            );
            // glTF's factor is linear, but the texture is sRGB (the GPU
            // decodes it to linear when sampling) - so it's encoded to sRGB
            // first, or the curve would be applied twice (darker, more
            // saturated than the color picked). Alpha is linear either way.
            let srgb = |channel: f32| (linear_to_srgb(channel) * 255.0).round() as u8;
            let solid_texture = Texture::from_image(
                &::image::DynamicImage::ImageRgba8(::image::RgbaImage::from_pixel(
                    1,
                    1,
                    ::image::Rgba([
                        srgb(r),
                        srgb(g),
                        srgb(b),
                        (a.clamp(0.0, 1.0) * 255.0).round() as u8,
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
                    model_log!(
                        "[model {}]   material {} \"{}\": embedded base-color texture ({} bytes in buffer {})",
                        file_name, mat_index, material.name().unwrap_or("<unnamed>"),
                        view.length(), view.buffer().index(),
                    );
                    let buffer = &buffer_data[view.buffer().index()];
                    let start = view.offset();
                    let end = start + view.length();
                    let diffuse_texture = match decoded.get(&image_index) {
                        Some(texture) => texture.clone(),
                        None => {
                            let texture = Texture::from_bytes(&buffer[start..end], device, queue, file_name).expect("Couldn't load diffuse");
                            decoded.insert(image_index, texture.clone());
                            texture
                        }
                    };
                    
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
                        bind_group,
                    });
                }
            image::Source::Uri { uri, mime_type: _ } => {
                let file_dir = Path::new(file_name).parent().unwrap_or(Path::new(""));

                // Join the GLTF directory with the URI to get the correct path.
                let full_path = file_dir.join(uri);
                model_log!(
                    "[model {}]   material {} \"{}\": external base-color texture \"{}\"",
                    file_name, mat_index, material.name().unwrap_or("<unnamed>"),
                    full_path.display(),
                );
                let diffuse_texture = match decoded.get(&image_index) {
                    Some(texture) => texture.clone(),
                    None => {
                        let texture = Texture::from_bytes(&read(&full_path)?, device, queue, &full_path.to_string_lossy())?;
                        decoded.insert(image_index, texture.clone());
                        texture
                    }
                };

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
                    bind_group,
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
    let mut geometry = ModelGeometry::default();

    for scene in gltf.scenes() {
        for node in scene.nodes() {
            traverse_node(node, &buffer_data, device, queue, transform_bind_group_layout, &mut mesh_lists, &mut geometry, file_name, None, default_material_index, double_sided)?;
        }
    }
    let (vertex_buffer, index_buffer) = geometry.upload(device, file_name);

    Ok(model::Model {
        mesh_lists,
        materials,
        vertex_buffer,
        index_buffer,
    })
}

/// A model's meshes' geometry, gathered while it loads - uploaded as one
/// vertex and one index buffer for the whole model (see
/// `model::Model::vertex_buffer`).
#[derive(Default)]
struct ModelGeometry {
    vertices: Vec<ModelVertex>,
    indices: Vec<u32>,
}

impl ModelGeometry {
    /// Adds one mesh's vertices and indices - its indices kept as they are,
    /// counting from its own first vertex. Its `(first_index, base_vertex)`
    /// (see `model::Mesh`).
    fn add(&mut self, vertices: &[ModelVertex], indices: &[u32]) -> (u32, i32) {
        let placed = (self.indices.len() as u32, self.vertices.len() as i32);
        self.vertices.extend_from_slice(vertices);
        self.indices.extend_from_slice(indices);
        placed
    }

    /// The model's vertex and index buffers. One with no geometry still gets
    /// a (one-element) buffer each - wgpu won't make an empty one.
    fn upload(mut self, device: &wgpu::Device, label: &str) -> (wgpu::Buffer, wgpu::Buffer) {
        if self.vertices.is_empty() {
            self.vertices.push(ModelVertex { position: [0.0; 3], tex_coords: [0.0; 2], normal: [0.0; 3] });
        }
        if self.indices.is_empty() {
            self.indices.push(0);
        }
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{label} vertex buffer")),
            contents: bytemuck::cast_slice(&self.vertices),
            // STORAGE: read as plain numbers by the wireframe debug view.
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::STORAGE,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{label} index buffer")),
            contents: bytemuck::cast_slice(&self.indices),
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::STORAGE,
        });
        (vertex_buffer, index_buffer)
    }
}

/// Extracts a mesh's raw triangle geometry (world-space vertex positions +
/// triangle indices, no GPU upload, no materials) from a `.gltf`/`.glb` file -
/// for building a `game_object::ColliderShape::Trimesh` from a model's own
/// shape instead of a hand-authored primitive (see that variant's own doc
/// comment). Reads the file independently of `load_model_with`'s own
/// GPU-facing load rather than reusing its result - a `Mesh` only keeps
/// `wgpu::Buffer`s once loaded (see that struct's own fields), nothing
/// CPU-readable survives to reuse here, so rendering + physics both wanting
/// the same model means a small amount of duplicate file I/O, paid once at
/// scene-construction time.
pub fn load_trimesh_geometry(file_name: &str) -> anyhow::Result<(Vec<Vector3<f32>>, Vec<[u32; 3]>)> {
    let gltf_data = load_asset_binary(file_name)?;
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
                let bin = load_asset_binary(full_path.to_str().unwrap())?;
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

/// Every point of the mesh on the node named `node_name` in the model file
/// at `path` (as `ModelSource::asset` takes it) - in the model's own space,
/// placed by its node (and parents) like it's drawn.
pub fn load_mesh_points(path: &Path, node_name: &str) -> anyhow::Result<Vec<Vector3<f32>>> {
    let data = std::fs::read(path)?;
    let gltf = Gltf::from_slice(&data)?;
    let buffer_data = gltf.buffers().map(|buffer| match buffer.source() {
        gltf::buffer::Source::Bin => Ok(gltf.blob.clone().unwrap_or_default()),
        gltf::buffer::Source::Uri(uri) => Ok(std::fs::read(path.parent().unwrap_or(Path::new("")).join(uri))?),
    }).collect::<anyhow::Result<Vec<Vec<u8>>>>()?;

    fn find(node: gltf::Node<'_>, node_name: &str, buffer_data: &[Vec<u8>], parent: Option<(Vector3<f32>, UnitQuaternion<f32>, Vector3<f32>)>, points: &mut Vec<Vector3<f32>>) {
        if node.name() == Some(node_name) {
            collect_trimesh_geometry(node, buffer_data, parent, points, &mut Vec::new());
            return;
        }
        // Its children are placed by it - the same composition as
        // collect_trimesh_geometry's.
        let (translation, rotation, scale) = node.transform().decomposed();
        let (translation, rotation, scale) = (Vector3::from(translation), UnitQuaternion::from_quaternion(Quaternion::from(rotation)), Vector3::from(scale));
        let placed = match parent {
            Some((parent_translation, parent_rotation, parent_scale)) => (
                parent_translation + parent_rotation * parent_scale.component_mul(&translation),
                parent_rotation * rotation,
                parent_scale.component_mul(&scale),
            ),
            None => (translation, rotation, scale),
        };
        for child in node.children() {
            find(child, node_name, buffer_data, Some(placed), points);
        }
    }

    let mut points = Vec::new();
    for scene in gltf.scenes() {
        for node in scene.nodes() {
            find(node, node_name, &buffer_data, None, &mut points);
        }
    }
    if points.is_empty() {
        anyhow::bail!("{} has no mesh named '{node_name}'", path.display());
    }
    Ok(points)
}

// Recurses the same way traverse_node does (child transforms compose with
// their parent's), but only ever touches positions/indices - no GPU device,
// no materials, since this is only ever building collision geometry, not
// anything rendered.
fn collect_trimesh_geometry(
    node: gltf::Node<'_>,
    buffer_data: &[Vec<u8>],
    parent_transform: Option<(Vector3<f32>, UnitQuaternion<f32>, Vector3<f32>)>,
    vertices: &mut Vec<Vector3<f32>>,
    indices: &mut Vec<[u32; 3]>,
) {
    let (translation, rotation, scale) = node.transform().decomposed();
    let local_translation = Vector3::from(translation);
    let local_rotation = UnitQuaternion::from_quaternion(Quaternion::from(rotation));
    let local_scale = Vector3::from(scale);

    // Compose parent * local (scale then rotate then translate at each level).
    // Non-uniform scale through a rotated hierarchy isn't perfectly
    // representable this way, but it's exact for the common single-level case
    // (a root mesh node with its own scale, e.g. Runway.glb's "Island") and a
    // good approximation otherwise - and it matches what the collider needs to
    // line up with the rendered mesh, which had the same scale-dropping bug
    // (see traverse_node).
    let (world_translation, world_rotation, world_scale) = match parent_transform {
        Some((parent_translation, parent_rotation, parent_scale)) => (
            parent_translation + parent_rotation * parent_scale.component_mul(&local_translation),
            parent_rotation * local_rotation,
            parent_scale.component_mul(&local_scale),
        ),
        None => (local_translation, local_rotation, local_scale),
    };

    if let Some(mesh) = node.mesh() {
        for primitive in mesh.primitives() {
            let reader = primitive.reader(|buffer| Some(&buffer_data[buffer.index()]));

            let base_index = vertices.len() as u32;
            if let Some(positions) = reader.read_positions() {
                for position in positions {
                    vertices.push(world_translation + world_rotation * world_scale.component_mul(&Vector3::from(position)));
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
        collect_trimesh_geometry(child, buffer_data, Some((world_translation, world_rotation, world_scale)), vertices, indices);
    }
}

fn traverse_node(node: gltf::Node<'_>, buffer_data: &[Vec<u8>], device: &wgpu::Device, queue: &wgpu::Queue, transform_bind_group_layout: &wgpu::BindGroupLayout, mesh_lists: &mut HashMap<String, HashMap<String, Mesh>>, geometry: &mut ModelGeometry, file_name: &str, parent_transform: Option<([f32; 3], [f32; 4], [f32; 3])>, default_material_index: usize, double_sided: DoubleSided) -> anyhow::Result<()> {
        // Nodes without a mesh (Blender Empties, armatures, pure parent/group
        // nodes) are valid glTF - just recurse past them into their children.
        let Some(mesh) = node.mesh() else {
            for child in node.children() {
                traverse_node(child, buffer_data, device, queue, transform_bind_group_layout, mesh_lists, geometry, file_name, Some(node.transform().decomposed()), default_material_index, double_sided)?;
            }
            return Ok(());
        };
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

            // Into the model's shared buffers (see ModelGeometry).
            let (first_index, base_vertex) = geometry.add(&vertices, &indices);

            let transform: Transform;
            let mut parent_values = None;

            match parent_transform {
                Some(parent_data) => {
                    let (parent_translation, parent_rotation, parent_scale) = parent_data;
                    let (translation, rotation, scale) = node.transform().decomposed();

                    let position = Vector3::from(parent_translation) + Vector3::from(translation);
                    let rotation = Quaternion::from(parent_rotation) * Quaternion::from(rotation);
                    // Keep this node's own scale - parent scale is carried
                    // separately in `parent_values` below. Forcing (1,1,1) here
                    // silently dropped any per-node scale a glTF exporter left
                    // on a mesh node (Blender routinely does, e.g. Runway.glb's
                    // "Island" node), rendering it ~1/scale too small.
                    transform = Transform::new(position, rotation, Vector3::from(scale));
                    parent_values = Some(Transform::new(parent_translation.into(), parent_rotation.into(), parent_scale.into()));
                },
                None => {
                    let (translation, rotation, scale) = node.transform().decomposed();
                    transform = Transform::new(translation.into(), rotation.into(), Vector3::from(scale));
                },
            }

            // The corner is filled in once the geometry's bounds are known,
            // below.
            let uniform = model::MeshUniform { matrix: transform.to_matrix_bufferable(), bounds_min: [0.0; 4] };
            let transform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(&format!("Transform Buffer")),
                contents: bytemuck::cast_slice(&[uniform]),
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

            // A mesh split across several materials in the DCC tool exports as
            // ONE node -> ONE mesh -> multiple primitives. The mesh map is
            // keyed by node name, so without this the primitives all land on
            // the same key and clobber each other (only the last material
            // would render). First primitive keeps the plain node name (so
            // the `wheel-f` / `left_elevator` / ... name lookups elsewhere
            // still resolve); extras get a "#primN" suffix. Single-primitive
            // meshes - the common case - are completely unchanged.
            // Local-space AABB of this primitive (kept on the mesh - see
            // Mesh::local_bounds - and logged below).
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for v in &vertices {
                for a in 0..3 {
                    lo[a] = lo[a].min(v.position[a]);
                    hi[a] = hi[a].max(v.position[a]);
                }
            }
            if vertices.is_empty() {
                (lo, hi) = ([0.0; 3], [0.0; 3]);
            }
            queue.write_buffer(&transform_buffer, 0, bytemuck::cast_slice(&[model::MeshUniform::new(&transform, (lo, hi))]));

            let node_name = node.name().map(str::to_owned).unwrap_or_else(|| format!("node{}", node.index()));
            let key = if primitive.index() == 0 {
                node_name.clone()
            } else {
                format!("{}#prim{}", node_name, primitive.index())
            };

            let mesh = model::Mesh {
                name: file_name.to_string(),
                first_index,
                num_elements: indices.len() as u32,
                base_vertex,
                material: primitive.material().index().unwrap_or(default_material_index),
                transform_buffer,
                transform_bind_group,
                transform,
                base_transform: transform,
                parent_transform: parent_values,
                alpha_mode: primitive.material().alpha_mode(),
                // Node name matches all of a node's primitives; the "#primN"
                // key matches just one.
                double_sided: double_sided.covers(&node_name) || double_sided.covers(&key),
                local_bounds: (lo, hi),
            };

            let list = if primitive.material().alpha_mode() == gltf::material::AlphaMode::Blend || primitive.material().alpha_mode() == gltf::material::AlphaMode::Mask {
                "transparent"
            } else {
                "opaque"
            };

            // Where the node's own transform (already baked into `transform`
            // above) puts the primitive's AABB.
            let corner_after = |p: [f32; 3]| {
                let m = transform.to_matrix_bufferable();
                // column-major mat * (p, 1)
                [
                    m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2] + m[3][0],
                    m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2] + m[3][1],
                    m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2] + m[3][2],
                ]
            };
            let a_lo = corner_after(lo);
            let a_hi = corner_after(hi);
            model_log!(
                "[model {}]   node \"{}\" / mesh \"{}\" prim {}: {} verts, {} indices, material {} ({:?}) -> \"{}\" list as key \"{}\"",
                file_name,
                node_name,
                node.mesh().and_then(|m| m.name().map(str::to_owned)).unwrap_or_else(|| "<unnamed>".to_owned()),
                primitive.index(),
                vertices.len(),
                indices.len(),
                primitive.material().index().map(|i| i.to_string()).unwrap_or_else(|| format!("default({default_material_index})")),
                primitive.material().alpha_mode(),
                list,
                key,
            );
            {
                let (t, r, s) = node.transform().decomposed();
                model_log!(
                    "[model {}]       node transform T={:?} R={:?} S={:?} | local AABB [{:.2},{:.2},{:.2}]..[{:.2},{:.2},{:.2}] -> after node xf [{:.1},{:.1},{:.1}]..[{:.1},{:.1},{:.1}]",
                    file_name, t, r, s,
                    lo[0], lo[1], lo[2], hi[0], hi[1], hi[2],
                    a_lo[0], a_lo[1], a_lo[2], a_hi[0], a_hi[1], a_hi[2],
                );
            }
            add_or_init_mesh_list(mesh_lists, &list.to_string(), key, mesh);

        });
    for child in node.children() {
        traverse_node(child, buffer_data, device, queue, transform_bind_group_layout, mesh_lists, geometry, file_name, Some(node.transform().decomposed()), default_material_index, double_sided)?;
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

/// Builds `environment` so it can be drawn (a skybox's textures loaded) -
/// no `&mut App`, so it can run on a scene's loading thread (Device/Queue are
/// cheap, Arc-backed clones). `camera_bind_group_layout` is the one piece
/// `SkyboxRender::new` needs out of the camera's resources.
pub fn prepare_environment(device: &Device, queue: &Queue, camera_bind_group_layout: &BindGroupLayout, config: &SurfaceConfiguration, environment: Environment) -> PreparedEnvironment {
    let srgb = config.format.is_srgb();
    let default_clear = clear_color(DEFAULT_CLEAR_COLOR, srgb);
    match environment {
        Environment::Color(color) => PreparedEnvironment { skybox: None, clear_color: clear_color(color, srgb) },
        Environment::ProceduralSky { sun } => PreparedEnvironment {
            skybox: Some(SkyboxRender::new_procedural(device, config, camera_bind_group_layout, sun)),
            clear_color: default_clear,
        },
        Environment::Skybox(faces) => {
            let texture = load_texture_cube(
                [&faces.px, &faces.nx, &faces.py, &faces.ny, &faces.pz, &faces.nz],
                device,
                queue,
            );

            match texture {
                Ok(texture) => PreparedEnvironment {
                    skybox: Some(SkyboxRender::new(device, config, camera_bind_group_layout, texture)),
                    clear_color: default_clear,
                },
                Err(err) => {
                    eprintln!("Skybox faces couldn't be loaded, falling back to clear color: {err}");
                    PreparedEnvironment { skybox: None, clear_color: default_clear }
                }
            }
        }
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

/// Which of a model's meshes render with backface culling OFF (two-sided).
/// Culling is ON by default (`None`). Declared here at registration rather
/// than poked in later - it's a property of the asset. Mesh names are glTF
/// node names (the `Model::mesh_lists` inner key - e.g. "Afterburner",
/// "canopy_glass", "Foo#prim1" for a multi-primitive mesh's extra prims).
#[derive(Clone, Copy)]
pub enum DoubleSided<'a> {
    None,
    All,
    Meshes(&'a [&'a str]),
}

impl DoubleSided<'_> {
    fn covers(&self, mesh_name: &str) -> bool {
        match self {
            DoubleSided::None => false,
            DoubleSided::All => true,
            DoubleSided::Meshes(names) => names.contains(&mesh_name),
        }
    }
}

/// The model `load` describes - `variant`'s copy of it if there's one (see
/// `Model::variant`) - taken from what the scene's loading step loaded, or
/// loaded now if nothing has. Remembered as the scene's, so
/// `unload_level_models` drops it when the scene ends.
///
/// A variant is a light copy of its base model (see `Model::share`): its own
/// mesh transforms, the base's geometry and textures - the base loaded first
/// if it isn't yet, and kept for whatever uses it plainly.
pub(crate) fn load_scene_model(app: &mut App, load: &ModelLoad, variant: Option<&str>) -> Result<Model, String> {
    let base_key = load.key.clone();
    let key = variant_key(&base_key, variant);
    if let Some(existing) = app.assets.models.idle.remove(&key) {
        return Ok(existing);
    }
    if !app.assets.models.is_loaded(&base_key) {
        let bind_group_layout = Mesh::create_bind_group_layout(&app.renderer.device);
        let model = load.load(&app.renderer.device, &app.renderer.queue, &bind_group_layout)
            .map_err(|e| format!("failed to load model '{base_key}': {e}"))?;
        app.assets.models.scene_owned.insert(base_key.clone());
        if variant.is_none() {
            return Ok(model);
        }
        app.assets.models.idle.insert(base_key.clone(), model);
    }
    let base_model = app.assets.models.drawn.get(&base_key).map(|instanced| &instanced.model)
        .or_else(|| app.assets.models.idle.get(&base_key))
        .ok_or_else(|| format!("model '{base_key}' vanished while sharing it"))?;
    let model = base_model.share(&app.renderer.device, &app.renderer.queue);
    app.assets.models.scene_owned.insert(key);
    Ok(model)
}

/// The key a `variant` of the model kept under `key` is kept under.
pub(crate) fn variant_key(key: &str, variant: Option<&str>) -> String {
    match variant {
        Some(variant) => format!("{key}#{variant}"),
        None => key.to_owned(),
    }
}

/// Takes models a scene's loading thread already built (see
/// `App::start_scene_load`) as this level's own - kept ready for
/// `load_scene_model` to hand out, and dropped with the level like any
/// other. One that failed is reported; spawning it then tries again itself.
pub fn adopt_level_models(app: &mut App, models: Vec<(String, Result<Model, String>)>) {
    for (key, model) in models {
        match model {
            Ok(model) => {
                app.assets.models.idle.insert(key.clone(), model);
                app.assets.models.scene_owned.insert(key);
            }
            Err(error) => eprintln!("{error}"),
        }
    }
}

/// Drops every model a scene loaded (see `load_scene_model`) - called as the
/// scene ends, so the next one starts from only what it uses. A game's mesh
/// goes too - its data stays in `AssetManager::add_mesh`'s, for the next
/// scene using it to build again.
///
/// Models in `keep` (keys the next scene uses too) stay loaded, handed over
/// to it: a restart of the same level reloads nothing.
pub fn unload_level_models(app: &mut App, keep: &HashSet<String>) {
    let level_keys = std::mem::take(&mut app.assets.models.scene_owned);
    for (key, instanced) in std::mem::take(&mut app.assets.models.drawn) {
        if !level_keys.contains(&key) || keep.contains(&key) {
            app.assets.models.idle.insert(key, instanced.model);
        }
    }
    app.assets.models.idle.retain(|key, _| !level_keys.contains(key) || keep.contains(key));
    // Kept ones still belong to a level - the next one - and go when it ends
    // (unless the one after keeps them too).
    app.assets.models.scene_owned = level_keys.into_iter().filter(|key| keep.contains(key)).collect();
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

/// A model of one mesh, `vertices` and `indices` (three a triangle,
/// counter-clockwise seen from the front), plain white until a `Material`
/// says otherwise - a `ModelSource::Cube`/`Plane`, or a game's mesh (see
/// `AssetManager::add_mesh`). `name` is its mesh's name.
pub(crate) fn build_mesh_model(name: &str, vertices: &[ModelVertex], indices: &[u32], double_sided: bool, device: &wgpu::Device, queue: &wgpu::Queue, transform_bind_group_layout: &wgpu::BindGroupLayout) -> Result<Model, String> {

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

    let mut geometry = ModelGeometry::default();
    let (first_index, base_vertex) = geometry.add(vertices, indices);

    // Identity - a primitive's actual placement/size comes from its node's
    // own Transform3D, same as every mesh loaded via load_model_with.
    let transform = Transform::new(Vector3::zeros(), Quaternion::identity(), Vector3::new(1.0, 1.0, 1.0));
    let mut local_bounds = ([f32::MAX; 3], [f32::MIN; 3]);
    for vertex in vertices {
        for axis in 0..3 {
            local_bounds.0[axis] = local_bounds.0[axis].min(vertex.position[axis]);
            local_bounds.1[axis] = local_bounds.1[axis].max(vertex.position[axis]);
        }
    }
    let transform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("primitive transform buffer"),
        contents: bytemuck::cast_slice(&[model::MeshUniform::new(&transform, local_bounds)]),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let transform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("primitive transform bind group"),
        layout: transform_bind_group_layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: transform_buffer.as_entire_binding() }],
    });

    let mesh = model::Mesh {
        name: name.to_owned(),
        first_index,
        num_elements: indices.len() as u32,
        base_vertex,
        material: 0,
        transform_buffer,
        transform_bind_group,
        transform,
        base_transform: transform,
        parent_transform: None,
        alpha_mode: gltf::material::AlphaMode::Opaque,
        double_sided,
        local_bounds,
    };
    let mut opaque = HashMap::new();
    opaque.insert(name.to_owned(), mesh);

    let mut mesh_lists = HashMap::new();
    mesh_lists.insert("opaque".to_owned(), opaque);

    let (vertex_buffer, index_buffer) = geometry.upload(device, name);
    Ok(model::Model { mesh_lists, materials, vertex_buffer, index_buffer })
}

#[cfg(test)]
mod tests {
    use super::linear_to_srgb;

    #[test]
    fn a_flat_color_is_encoded_to_srgb_once() {
        // Black, white, and linear 0.214 - what Blender shows as 0.5 grey.
        assert_eq!(linear_to_srgb(0.0), 0.0);
        assert!((linear_to_srgb(1.0) - 1.0).abs() < 1e-5);
        assert!((linear_to_srgb(0.214) - 0.5).abs() < 0.005);
    }
}
