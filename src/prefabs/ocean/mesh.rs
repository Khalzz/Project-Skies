//! The sea's mesh: nested square rings around the camera, finest in the
//! middle, much coarser each ring out, the last one flat out past the
//! horizon. Registered once as a custom mesh (`MESH`) and kept centred under
//! the camera by `Ocean::update`.
//!
//! A ring only moves the waves at least ~3 of its cells long (ocean.wgsl's
//! GEOMETRY_MIN_CELLS), and the water's look is per pixel everywhere - so
//! the mesh only has to carry the swell's shape near the camera. Each ring's
//! cell is a whole multiple of the one inside it, and its edges land on its
//! own and the next ring's grid (checked by `build`); ocean.wgsl's ring
//! morph joins them without cracks however big the jump.

use pankarta_software::asset_manager::ModelVertex;
use pankarta_software::prelude::App;

/// The custom mesh's name - what `super::surface` draws.
pub(super) const MESH: &str = "OceanSurface";

/// One ring: square cells `cell` wide, reaching out to `outer_half_extent`
/// from the center - both in world units.
struct Ring {
    cell: f32,
    outer_half_extent: f32,
}

/// Innermost first (~35k triangles):
/// - 4 m cells, 128 m across - moves every wave 12 m and longer;
/// - 16 m cells out to 512 m - the longest waves, faded out by its edge;
/// - 128 m cells to 4 km, 2 km cells to 65 km - flat for the default sea,
///   longer waves for strong winds;
/// - 32 km cells to 1,048 km - flat, out past the horizon.
///
/// A ring's inner corners stay unmorphed (meeting the ring inside) only
/// while its half-extent is at least ~2x the next ring's cell - each is.
const RINGS: &[Ring] = &[
    Ring { cell: 4.0, outer_half_extent: 64.0 },
    Ring { cell: 16.0, outer_half_extent: 512.0 },
    Ring { cell: 128.0, outer_half_extent: 4_096.0 },
    Ring { cell: 2_048.0, outer_half_extent: 65_536.0 },
    Ring { cell: 32_768.0, outer_half_extent: 1_048_576.0 },
];

/// Adds the mesh to the engine's assets, as `MESH`.
pub(super) fn register(app: &mut App) {
    let (vertices, indices) = build(RINGS);
    if let Err(error) = app.assets.add_mesh(MESH.to_owned(), vertices, indices) {
        eprintln!("{error}");
    }
}

/// The rings as one flat (Y=0) mesh around the origin - the first a full
/// square grid, every one after a square ring around the one before.
///
/// Each vertex carries what ocean.wgsl needs to blend rings together:
/// `tex_coords` = (its ring's cell size, its ring's outer half-extent), and
/// `normal.x` = the NEXT ring's cell size (the shader computes its own
/// normals, so the field is free).
///
/// Panics if a ring's cell isn't a whole multiple of the one inside it, or
/// an edge isn't on its ring's own grid.
fn build(rings: &[Ring]) -> (Vec<ModelVertex>, Vec<u32>) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rings_build_and_stay_light() {
        // `build` panics on a layout the shader's ring morph can't join up.
        // Every vertex sums the geometry waves each frame (ocean.wgsl's
        // gerstner), so the vertex count is most of what the mesh costs.
        let (vertices, indices) = build(RINGS);
        println!("ocean mesh: {} vertices, {} triangles", vertices.len(), indices.len() / 3);
        assert!(vertices.len() < 1_000_000, "the ocean mesh has {} vertices", vertices.len());
    }
}
