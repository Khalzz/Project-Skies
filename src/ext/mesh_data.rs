//! Mesh data the game reads or builds itself: points straight from a glTF
//! file (a plane's wing outlines, its colliders, the world's collision).

use std::path::Path;

use gltf::Gltf;
use nalgebra::{Quaternion, UnitQuaternion, Vector3};

/// Every point of the mesh on the node named `node_name` in the model file
/// at `path` (relative to where the game runs) - in the model's own space,
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
            collect_geometry(node, buffer_data, parent, points, &mut Vec::new());
            return;
        }
        // Its children are placed by it - the same composition as
        // collect_geometry's.
        let placed = compose(parent, &node);
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

/// `node`'s transform placed under `parent` (scale, then rotate, then
/// translate at each level).
fn compose(parent: Option<(Vector3<f32>, UnitQuaternion<f32>, Vector3<f32>)>, node: &gltf::Node<'_>) -> (Vector3<f32>, UnitQuaternion<f32>, Vector3<f32>) {
    let (translation, rotation, scale) = node.transform().decomposed();
    let (translation, rotation, scale) = (Vector3::from(translation), UnitQuaternion::from_quaternion(Quaternion::from(rotation)), Vector3::from(scale));
    match parent {
        Some((parent_translation, parent_rotation, parent_scale)) => (
            parent_translation + parent_rotation * parent_scale.component_mul(&translation),
            parent_rotation * rotation,
            parent_scale.component_mul(&scale),
        ),
        None => (translation, rotation, scale),
    }
}

/// Every triangle of `node` and its children - positions and indices only.
fn collect_geometry(
    node: gltf::Node<'_>,
    buffer_data: &[Vec<u8>],
    parent_transform: Option<(Vector3<f32>, UnitQuaternion<f32>, Vector3<f32>)>,
    vertices: &mut Vec<Vector3<f32>>,
    indices: &mut Vec<[u32; 3]>,
) {
    let (world_translation, world_rotation, world_scale) = compose(parent_transform, &node);

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
        collect_geometry(child, buffer_data, Some((world_translation, world_rotation, world_scale)), vertices, indices);
    }
}

/// Every triangle of the model file `path` (inside the assets folder) -
/// positions placed like they're drawn, and indices - for collision and the
/// tactical map's contours.
pub fn load_trimesh_geometry(path: &str) -> anyhow::Result<(Vec<Vector3<f32>>, Vec<[u32; 3]>)> {
    let file = pankarta_software::asset_manager::asset_path(path);
    let data = std::fs::read(&file)?;
    let gltf = Gltf::from_slice(&data)?;
    let buffer_data = gltf.buffers().map(|buffer| match buffer.source() {
        gltf::buffer::Source::Bin => Ok(gltf.blob.clone().unwrap_or_default()),
        gltf::buffer::Source::Uri(uri) => Ok(std::fs::read(file.parent().unwrap_or(Path::new("")).join(uri))?),
    }).collect::<anyhow::Result<Vec<Vec<u8>>>>()?;
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    for scene in gltf.scenes() {
        for node in scene.nodes() {
            collect_geometry(node, &buffer_data, None, &mut vertices, &mut indices);
        }
    }
    Ok((vertices, indices))
}

