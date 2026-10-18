//! Big meshes drawn a part at a time - what the ocean's ring mesh needs, but
//! nothing about water in it:
//! - altitude tiers: a mesh named into a tier (see resources::WaterTier)
//!   only draws while the camera's at that tier's altitude;
//! - tiles: a mesh split into tiles (see resources::WaterTile) only draws
//!   the tiles in view - neighbours merged into one draw.
//!
//! Which mesh a material draws with is the shader library's business; this
//! only says which parts of it to draw (see render_pass's draw_materials).

use std::collections::HashMap;
use std::ops::Range;

use nalgebra::{Matrix4, Vector3, Vector4};

use crate::resources::WaterTile;

/// What the last frame's tiled meshes drew vs. could have - see
/// `TiledMeshes::draw_stats` (shown on the F3 debug panel).
#[derive(Clone, Copy, Debug, Default)]
pub struct TileDrawStats {
    pub tiles_drawn: u32,
    pub tiles_total: u32,
    pub triangles_drawn: u64,
    pub triangles_total: u64,
}

/// The four side planes of a view volume, (a, b, c, d) with a point inside
/// when a*x + b*y + c*z + d >= 0 - in true world space. No near/far planes:
/// the four sides already meet at the camera, so they only enclose what's
/// in front of it, and a tiled mesh may reach past any far plane.
pub type Frustum = [Vector4<f32>; 4];

#[derive(Default)]
pub struct TiledMeshes {
    // Altitude tiers (see resources::WaterTier) - each tier's minimum camera
    // altitude, ascending - and the one being drawn.
    tier_altitudes: Vec<f32>,
    tier: usize,
    // Tiles per mesh name (see resources::WaterTile).
    tiles: HashMap<String, Vec<WaterTile>>,
    // F8 debug (see `toggle_culling_freeze`): while set, culling keeps using
    // the view it captured, so you can look around and see what's skipped.
    freeze_culling: bool,
    frozen_frustum: Option<Frustum>,
    stats: TileDrawStats,
}

impl TiledMeshes {
    /// Each tier's minimum camera altitude, ascending (see
    /// resources::WaterTier) - set once, when the tiered model is registered.
    pub fn set_tier_altitudes(&mut self, altitudes: Vec<f32>) {
        self.tier_altitudes = altitudes;
        self.tier = 0;
    }

    /// Which tier draws this frame.
    pub fn tier(&self) -> usize {
        self.tier
    }

    /// Steps up a tier once the camera reaches the next tier's altitude, and
    /// back down only once it's 10% below the current tier's - so hovering
    /// right at a boundary doesn't flip between the two every frame. Once a
    /// frame.
    pub fn update_tier(&mut self, altitude: f32) {
        const STEP_DOWN_MARGIN: f32 = 0.9;
        while self.tier + 1 < self.tier_altitudes.len() && altitude >= self.tier_altitudes[self.tier + 1] {
            self.tier += 1;
        }
        while self.tier > 0 && altitude < self.tier_altitudes[self.tier] * STEP_DOWN_MARGIN {
            self.tier -= 1;
        }
    }

    /// A mesh's tiles (see resources::WaterTile) - set once, when it's
    /// registered. Meshes without tiles always draw whole.
    pub fn set_mesh_tiles(&mut self, mesh_name: String, tiles: Vec<WaterTile>) {
        self.tiles.insert(mesh_name, tiles);
    }

    /// Whether the mesh `mesh_name` is tiled.
    pub fn is_tiled(&self, mesh_name: &str) -> bool {
        self.tiles.contains_key(mesh_name)
    }

    /// F8 debug: freezes culling at the current view (or unfreezes it) - fly
    /// or turn around while frozen and a tiled mesh only exists inside the
    /// view it was frozen at, so you can see exactly which tiles are skipped.
    pub fn toggle_culling_freeze(&mut self) {
        self.freeze_culling = !self.freeze_culling;
        self.frozen_frustum = None;
        println!("Tile culling: {}", if self.freeze_culling { "FROZEN at the current view - look around to see what's skipped" } else { "live" });
    }

    /// Last frame's culling results - see TileDrawStats.
    pub fn draw_stats(&self) -> TileDrawStats {
        self.stats
    }

    /// Resets the culling counters - once a frame, before the culling.
    pub fn begin_culling_stats(&mut self) {
        self.stats = TileDrawStats::default();
    }

    /// The view volume's side planes in true world space, from the camera's
    /// camera-relative `view_proj` (see CameraUniform) and its true position.
    pub fn world_frustum(view_proj: &Matrix4<f32>, camera_position: Vector3<f32>) -> Frustum {
        // Clip space keeps -w <= x, y <= w - each inequality is a plane
        // (Gribb/Hartmann): row 3 plus/minus row 0 (left/right) and row 1
        // (bottom/top). Then shifted from camera-relative to world space.
        let row = |i: usize| view_proj.row(i).transpose();
        let planes = [row(3) + row(0), row(3) - row(0), row(3) + row(1), row(3) - row(1)];
        planes.map(|plane| {
            let normal = plane.xyz();
            Vector4::new(plane.x, plane.y, plane.z, plane.w - normal.dot(&camera_position))
        })
    }

    /// The index ranges of `mesh_name` to draw this frame: its tiles that
    /// intersect `frustum` (or the frozen one, see `toggle_culling_freeze`),
    /// neighbours merged into one range. `mesh_position` is where the mesh's
    /// origin is in true world space. `None` if the mesh has no tiles (draw
    /// it whole). Also adds to the frame's `draw_stats`.
    pub fn visible_ranges(&mut self, mesh_name: &str, frustum: Frustum, mesh_position: Vector3<f32>) -> Option<Vec<Range<u32>>> {
        let frustum = if self.freeze_culling { *self.frozen_frustum.get_or_insert(frustum) } else { frustum };
        let tiles = self.tiles.get(mesh_name)?;

        let mut ranges: Vec<Range<u32>> = Vec::new();
        for tile in tiles {
            self.stats.tiles_total += 1;
            self.stats.triangles_total += (tile.index_count / 3) as u64;

            let min = mesh_position + Vector3::from(tile.min);
            let max = mesh_position + Vector3::from(tile.max);
            // A box is out of view once it's entirely behind any one plane -
            // test the corner furthest along that plane's normal.
            let outside = frustum.iter().any(|plane| {
                let corner = Vector3::new(
                    if plane.x >= 0.0 { max.x } else { min.x },
                    if plane.y >= 0.0 { max.y } else { min.y },
                    if plane.z >= 0.0 { max.z } else { min.z },
                );
                plane.xyz().dot(&corner) + plane.w < 0.0
            });
            if outside {
                continue;
            }

            self.stats.tiles_drawn += 1;
            self.stats.triangles_drawn += (tile.index_count / 3) as u64;
            let range = tile.first_index..tile.first_index + tile.index_count;
            match ranges.last_mut() {
                Some(last) if last.end == range.start => last.end = range.end,
                _ => ranges.push(range),
            }
        }
        Some(ranges)
    }
}
