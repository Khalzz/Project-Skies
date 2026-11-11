//! Mesh data the game reads or builds itself: points straight from a glTF
//! file (a plane's wing outlines, its colliders), and the ocean's ring
//! meshes (registered with the engine as custom meshes).

use std::path::Path;

use gltf::Gltf;
use nalgebra::{Quaternion, UnitQuaternion, Vector3};
use pankarta_software::asset_manager::ModelVertex;
use pankarta_software::prelude::*;

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

/// One ring of the ocean mesh: square cells `cell` wide, reaching out to
/// `outer_half_extent` from the center - both in world units.
#[derive(Clone, Copy, Debug)]
pub struct WaterRing {
    pub cell: f32,
    pub outer_half_extent: f32,
}

/// One altitude tier of the ocean: the rings drawn while the camera is at or
/// above `min_altitude` (world Y) - and below the next tier's. Higher tiers
/// use coarser rings: from higher up, the nearest water is further away, so
/// bigger cells look just as fine on screen.
#[derive(Clone, Copy, Debug)]
pub struct WaterTier {
    pub min_altitude: f32,
    pub rings: &'static [WaterRing],
}

/// The custom mesh name of tier `tier` of the ocean registered as `model`.
pub fn ocean_tier_mesh(model: &str, tier: usize) -> String {
    format!("{model}#tier{tier}")
}

/// Which of `tiers` an eye at `altitude` uses.
pub fn ocean_tier_for(tiers: &[WaterTier], altitude: f32) -> usize {
    tiers.iter().rposition(|tier| altitude >= tier.min_altitude).unwrap_or(0)
}

/// Adds every tier of the ocean as a custom mesh (`ocean_tier_mesh`) - drawn
/// by a node with `Mesh::custom(..)`.
///
/// The game's own engine drew only the tiles of a tier in view; the engine
/// draws a mesh whole, so the whole tier is drawn every frame.
pub fn register_ocean(app: &mut App, model: &str, tiers: &[WaterTier]) {
    for (index, tier) in tiers.iter().enumerate() {
        let (vertices, indices) = build_water_rings_mesh(tier.rings);
        if let Err(error) = app.assets.add_mesh(ocean_tier_mesh(model, index), vertices, indices) {
            eprintln!("{error}");
        }
    }
}

/// A level-of-detail ocean mesh: nested square rings around the origin
/// (flat, Y=0), one per `rings` entry, innermost first - the first a full
/// square grid, every one after a square ring around the one before. Dense
/// near the center (the camera), coarse far away.
///
/// Each vertex carries what ocean.wgsl needs to blend rings together:
/// `tex_coords` = (its ring's cell size, its ring's outer half-extent), and
/// `normal.x` = the NEXT ring's cell size (the shader computes its own
/// normals, so the field is free). The shader uses them to morph each
/// ring's outer edge onto a coarser grid and to fade out the waves the next
/// ring can't carry, so rings meet without cracks.
///
/// Each ring's cell must be a whole multiple of the one inside it, and
/// every edge must land on its ring's own grid (panics otherwise).
pub fn build_water_rings_mesh(rings: &[WaterRing]) -> (Vec<ModelVertex>, Vec<u32>) {
    let mut vertices = Vec::new();
    let mut indices = Vec::new();
    let mut inner_half_extent = 0.0f32;
    for (level, ring) in rings.iter().enumerate() {
        let cell = ring.cell;
        let half = (ring.outer_half_extent / cell).round() as i32;
        let inner = (inner_half_extent / cell).round() as i32;
        assert!((half as f32 * cell - ring.outer_half_extent).abs() < 1e-3, "water ring {level}: outer edge isn't on its grid");
        assert!((inner as f32 * cell - inner_half_extent).abs() < 1e-3, "water ring {level}: inner edge isn't on its grid");
        assert!(half > inner, "water ring {level}: must reach further than the one inside it");
        let next_cell = rings.get(level + 1).map(|next| next.cell).unwrap_or(cell * 2.0);
        assert!((next_cell / cell).fract() == 0.0, "water ring {}: cell must be a whole multiple of ring {level}'s", level + 1);
        // Its outer edge slides onto the next ring's grid (ocean.wgsl's ring
        // morph) - so it must be on it.
        assert!((ring.outer_half_extent / next_cell).fract() == 0.0, "water ring {level}: outer edge isn't on the next ring's grid");
        let side = (half * 2 + 1) as usize;

        // Grid vertex index -> mesh vertex index, only for vertices a kept
        // cell actually uses.
        let mut vertex_of = vec![None::<u32>; side * side];
        let mut vertex = |i: i32, j: i32, vertices: &mut Vec<ModelVertex>| -> u32 {
            let slot = (j + half) as usize * side + (i + half) as usize;
            *vertex_of[slot].get_or_insert_with(|| {
                vertices.push(ModelVertex {
                    position: [i as f32 * cell, 0.0, j as f32 * cell],
                    tex_coords: [cell, ring.outer_half_extent],
                    normal: [next_cell, 0.0, 0.0],
                });
                (vertices.len() - 1) as u32
            })
        };

        // Rows along +Z, columns along +X, leaving out the middle the
        // previous ring already covers.
        for j in -half..half {
            for i in -half..half {
                if i >= -inner && i < inner && j >= -inner && j < inner {
                    continue;
                }
                let top_left = vertex(i, j, &mut vertices);
                let top_right = vertex(i + 1, j, &mut vertices);
                let bottom_left = vertex(i, j + 1, &mut vertices);
                let bottom_right = vertex(i + 1, j + 1, &mut vertices);
                indices.extend_from_slice(&[top_left, bottom_left, bottom_right, top_left, bottom_right, top_right]);
            }
        }
        inner_half_extent = ring.outer_half_extent;
    }
    (vertices, indices)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_ocean_tier_builds_and_stays_light() {
        // build_water_rings_mesh panics on a ring layout the shader's ring
        // morph can't join up. The vertex budget: every vertex sums the
        // geometry waves each frame (see ocean.wgsl's gerstner) - the old
        // 2 m-cells-to-2 km layout was ~7.9M and is what made the sea slow.
        for (index, tier) in crate::OCEAN_TIERS.iter().enumerate() {
            let (vertices, indices) = build_water_rings_mesh(tier.rings);
            println!("ocean tier {index}: {} vertices, {} triangles", vertices.len(), indices.len() / 3);
            assert!(vertices.len() < 1_000_000, "ocean tier {index} has {} vertices", vertices.len());
        }
    }
}
