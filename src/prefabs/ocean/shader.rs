//! The sea's side of the ocean shader (assets/shaders/ocean.wgsl, settings
//! in ocean.ron): its material, and the params that aren't the waves' or the
//! wake's - per-pixel cost, detail by altitude, the ripple map's layers and
//! the far look - written into the sea node's material every frame.

use std::collections::BTreeMap;

use pankarta_software::prelude::Mesh;
use pankarta_software::scene_manager::node::properties::Material;
use pankarta_software::scene_manager::scene::Scene;

use super::ripples;
use super::settings::{OceanSettings, ALTITUDE_DETAIL_START};
use super::waves::{OceanWaves, MAX_WAVES};

/// The shader's params by name (ocean.wgsl's `Ocean` struct fields).
pub(super) type Params = BTreeMap<String, Vec<f32>>;

/// The sea's shader, by name (assets/shaders/ocean.wgsl).
const SHADER: &str = "ocean";

pub(super) fn material() -> Material {
    Material::shader(SHADER)
}

/// Puts `params` into the material of the node `node`'s mesh.
pub(super) fn set_on(scene: &mut Scene, node: &str, params: &Params) {
    let Some(material) = scene
        .get_node_mut(node)
        .and_then(|node| node.get_property_mut::<Mesh>())
        .and_then(|mesh| mesh.material.as_mut())
    else {
        return;
    };
    for (name, values) in params {
        material.set(name, values.clone());
    }
}

/// The shortest wave drawn with the camera `height` above the sea - see
/// OceanSettings::altitude_detail.
fn shortest_wave_at(settings: &OceanSettings, height: f32) -> f32 {
    (height - ALTITUDE_DETAIL_START).max(0.0) * settings.altitude_detail.max(0.0)
}

/// Detail by the camera's altitude (ocean.wgsl's `detail`) - `height` is the
/// camera's above the sea.
pub(super) fn write_detail(params: &mut Params, settings: &OceanSettings, height: f32) {
    params.insert(
        "detail".to_owned(),
        vec![
            shortest_wave_at(settings, height),
            settings.ripple_layers.clamp(1, 2) as f32,
            if settings.ripple_stochastic { 1.0 } else { 0.0 },
            settings.far_variation.max(0.0),
        ],
    );
}

/// The per-pixel cost knobs and where the detail waves come from
/// (ocean.wgsl's `tuning`).
pub(super) fn write_tuning(params: &mut Params, settings: &OceanSettings) {
    params.insert(
        "tuning".to_owned(),
        vec![
            settings.pixel_full_pixels,
            settings.pixel_wave_cap.min(MAX_WAVES) as f32,
            0.0,
            if settings.textured_ripples && !settings.flat { 1.0 } else { 0.0 },
        ],
    );
}

/// Where the ripple map's layers are this frame (see ripples and
/// ocean.wgsl's ripple_layer): each one's place on the map under the camera
/// - true world XZ (`x`, `z`), in f64 and wrapped to one tile like the
/// waves' phases, so it stays precise anywhere - and its axes. Also the far
/// look (`look`).
pub(super) fn write_ripples(params: &mut Params, waves: &OceanWaves, x: f64, z: f64) {
    let map = ripples::ripple_map();
    let settings = waves.settings();
    let time = waves.time();
    let strength = settings.detail_strength as f64;
    let mut cells = Vec::with_capacity(6);
    // The near layers at `ripple_scale` - bigger, and drifting as much
    // faster (longer waves travel faster).
    let scale = settings.ripple_scale.max(0.1) as f64;
    let tile_of = |index: usize| if index < 2 { ripples::LAYERS[index].tile * scale } else { ripples::FAR_LAYER.tile };
    for (index, (name, layer)) in ["ripple_a", "ripple_b", "ripple_far"]
        .into_iter()
        .zip(ripples::LAYERS.iter().chain([&ripples::FAR_LAYER]))
        .enumerate()
    {
        let angle = settings.wind_direction as f64 + layer.angle;
        let (cos, sin) = (angle.cos(), angle.sin());
        let tile = tile_of(index);
        let drift = if index < 2 { layer.drift * scale.sqrt() } else { layer.drift };
        let u = (cos * x + sin * z - drift * time) / tile;
        let v = (-sin * x + cos * z) / tile;
        params.insert(name.to_owned(), vec![u.rem_euclid(1.0) as f32, v.rem_euclid(1.0) as f32, cos as f32, sin as f32]);
        // Its stochastic grid (ocean.wgsl's ripple_layer) moves with it -
        // the same skew as the shader's, wrapped to where its random places
        // repeat.
        let (grid_u, grid_v) = (u * ripples::HEX_SCALE, v * ripples::HEX_SCALE);
        let period = ripples::HEX_PERIOD as f64;
        cells.push((grid_u - 0.577_350_27 * grid_v).rem_euclid(period) as f32);
        cells.push((1.154_700_54 * grid_v).rem_euclid(period) as f32);
    }
    // The far layer's grid, and 1 / its tile size.
    let far_cells = vec![cells[4], cells[5], (1.0 / ripples::FAR_LAYER.tile) as f32, 0.0];
    cells.truncate(4);
    params.insert("ripple_cells".to_owned(), cells);
    params.insert("ripple_far_cells".to_owned(), far_cells);
    // One layer's slope variance, at full detail.
    let layer_variance = ripples::LAYER_WEIGHT.powi(2) * map.variance * strength * strength;
    params.insert(
        "ripple_info".to_owned(),
        vec![
            (1.0 / tile_of(0)) as f32,
            (1.0 / tile_of(1)) as f32,
            (2.0 * map.range * ripples::LAYER_WEIGHT * strength) as f32,
            (waves.geometry_slope_variance() + 2.0 * layer_variance) as f32,
        ],
    );
    params.insert("ripple_bias".to_owned(), vec![map.mip_bias[0] as f32, map.mip_bias[1] as f32, layer_variance as f32, 0.0]);
    params.insert(
        "look".to_owned(),
        vec![settings.far_start.max(0.0), settings.far_end.max(settings.far_start + 1.0), settings.depth_tint.clamp(0.0, 2.0), 0.0],
    );
}
