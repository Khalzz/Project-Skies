//! The ripple map: the sea's shortest waves (SHORTEST..LONGEST m) baked
//! into one tiling texture (RIPPLE_MAP), which the ocean shader reads twice
//! per pixel instead of summing its detail waves one by one, when
//! OceanSettings::textured_ripples is on.
//!
//! The map is slopes, not a picture: each texel is the ripples' slope at
//! that point, along the map's u (red) and v (green), 0.5 = flat (see
//! `pixels` for the encoding). It's made of WAVES waves from the same
//! spectrum as the detail waves (game::ocean::waves), each fitting the
//! tile a whole number of times in both directions so the map tiles
//! seamlessly. The GPU's mipmaps average it as it gets far away, so the
//! ripples fade smoothly with distance instead of being dropped one by one.
//!
//! The shader draws it as two LAYERS - different sizes, turned to different
//! directions, drifting differently - so they interfere and the ripples
//! seem to move and change rather than slide past as one sheet, and its
//! tiling doesn't line up.
//!
//! The file is generated and kept in the repo: after changing anything
//! here, write it again with `OCEAN_WRITE_RIPPLES=1 cargo test ripple_map`
//! (the test fails while it's out of date).

use std::collections::HashSet;
use std::f64::consts::TAU;
use std::sync::OnceLock;

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

use super::waves::{OceanWaves, GRAVITY, PM_ALPHA};

/// Where the map is - ocean.ron's `ripples` texture.
pub const RIPPLE_MAP: &str = "assets/textures/ocean_ripples.png";
/// Texels across (and down) - the shortest ripple spans 8 of them.
pub const SIZE: u32 = 512;
/// Meters the map covers before it repeats.
pub const TILE: f64 = 32.0;
/// The ripples' wavelength band, meters - ocean.wgsl's RIPPLE_SHORTEST /
/// RIPPLE_LONGEST are these over TILE (keep in sync).
pub const SHORTEST: f64 = 0.5;
pub const LONGEST: f64 = 8.0;
const WAVES: usize = 160;
/// How tightly the ripples' directions cluster around the map's u (the
/// wind, once drawn) - OceanSettings::directional_spread's default.
const SPREAD: f64 = 2.0;
/// The spectrum's wind. Ripples this short are on its tail, which barely
/// depends on the wind - so one map suits every sea.
const WIND_SPEED: f64 = 6.0;
/// Slopes this many standard deviations out still fit the texture.
const RANGE_SIGMAS: f64 = 4.0;
const SEED: u64 = 0x0121_9915;

/// One of the two ways the map is drawn over the sea.
pub struct Layer {
    /// Meters one copy of the map covers.
    pub tile: f64,
    /// Which way its u points, off the wind (radians).
    pub angle: f64,
    /// How fast it drifts along its u, m/s.
    pub drift: f64,
}

/// The map at full size along the wind, and smaller (an unrelated ratio, so
/// the two never repeat together) turned ~50 degrees off it, slower.
pub const LAYERS: [Layer; 2] = [Layer { tile: TILE, angle: 0.0, drift: 1.2 }, Layer { tile: TILE * 0.619, angle: 0.85, drift: 0.8 }];
/// The stochastic tiling (ocean.wgsl's ripple_layer): grid points per tile,
/// and how many grid points before their random places repeat - its
/// RIPPLE_HEX_SCALE / RIPPLE_HEX_PERIOD (keep in sync).
pub const HEX_SCALE: f64 = 1.5;
pub const HEX_PERIOD: i32 = 256;
/// The far layer (OceanSettings::far_variation): the map 64x larger - 32-512
/// m patterns - turned off the wind the other way, drifting faster (longer
/// waves travel faster).
pub const FAR_LAYER: Layer = Layer { tile: TILE * 64.0, angle: -0.6, drift: 6.0 };
/// What each layer's slope is scaled by - two unrelated layers at
/// 1/sqrt(2) together have the map's slope variance once.
pub const LAYER_WEIGHT: f64 = std::f64::consts::FRAC_1_SQRT_2;

struct RippleWave {
    /// Whole cycles across the tile, along u and v.
    cycles: (i32, i32),
    amplitude: f64,
    phase: f64,
}

pub struct RippleMap {
    waves: Vec<RippleWave>,
    /// Slope variance of the whole map, both directions together.
    pub variance: f64,
    /// Slopes from -range to +range fit the texture.
    pub range: f64,
    /// Where a fully averaged mipmap sits, along u and v, relative to the
    /// middle (0.5) - a little below it: the engine averages the mipmaps
    /// in sRGB, and the texture is sRGB. The shader takes it off as the
    /// mipmaps take over.
    pub mip_bias: [f64; 2],
}

/// The map, worked out once.
pub fn ripple_map() -> &'static RippleMap {
    static MAP: OnceLock<RippleMap> = OnceLock::new();
    MAP.get_or_init(RippleMap::new)
}

impl RippleMap {
    fn new() -> Self {
        let mut rng = StdRng::seed_from_u64(SEED);
        let peak_omega = 0.855 * GRAVITY / WIND_SPEED;
        let omega_of = |wavelength: f64| (GRAVITY * TAU / wavelength).sqrt();
        let (log_min, log_max) = (SHORTEST.ln(), LONGEST.ln());
        let bin = (log_max - log_min) / WAVES as f64;

        // Like OceanWaves::generate - one wave per log-spaced bin, its share
        // of the spectrum's energy - but each wave the nearest one that
        // fits the tile a whole number of times. Long ripples have few such
        // waves, so a bin whose tries are all taken is left out.
        let mut taken = HashSet::new();
        let mut waves = Vec::with_capacity(WAVES);
        for index in 0..WAVES {
            let (low, high) = ((log_min + bin * index as f64).exp(), (log_min + bin * (index + 1) as f64).exp());
            for _ in 0..8 {
                let wavelength = rng.gen_range(low..high);
                let angle = OceanWaves::spread_angle(&mut rng, SPREAD);
                let phase = rng.gen_range(0.0..TAU);
                let across = TILE / wavelength;
                let cycles = ((across * angle.cos()).round() as i32, (across * angle.sin()).round() as i32);
                if cycles == (0, 0) || !taken.insert(cycles) {
                    continue;
                }
                let k = TAU * ((cycles.0 * cycles.0 + cycles.1 * cycles.1) as f64).sqrt() / TILE;
                let omega = (GRAVITY * k).sqrt();
                let spectrum = PM_ALPHA * GRAVITY * GRAVITY * omega.powi(-5) * (-1.25 * (peak_omega / omega).powi(4)).exp();
                let amplitude = (2.0 * spectrum * (omega_of(low) - omega_of(high))).sqrt();
                waves.push(RippleWave { cycles, amplitude, phase });
                break;
            }
        }

        // Each wave's slope, along u and v: k * a * its direction.
        let mut along = [0.0f64; 2];
        for wave in &waves {
            let (slope_u, slope_v) = Self::slope_amplitude(wave);
            along[0] += 0.5 * slope_u * slope_u;
            along[1] += 0.5 * slope_v * slope_v;
        }
        let range = RANGE_SIGMAS * along[0].max(along[1]).sqrt();
        let mip_bias = along.map(|variance| srgb_mip_bias(variance.sqrt() / (2.0 * range)));
        Self { waves, variance: along[0] + along[1], range, mip_bias }
    }

    fn slope_amplitude(wave: &RippleWave) -> (f64, f64) {
        let k = TAU / TILE;
        (wave.amplitude * k * wave.cycles.0 as f64, wave.amplitude * k * wave.cycles.1 as f64)
    }

    /// The map's texels, RGBA8 row by row (v down the rows, u across):
    /// red/green the slope along u/v, mapped -range..range to 0..1 and
    /// sRGB-encoded (the engine loads textures as sRGB, so the shader reads
    /// back that 0..1), blue a flat 0.5, alpha 1.
    pub fn pixels(&self) -> Vec<u8> {
        let size = SIZE as usize;
        // cos(a + b) = cos a cos b - sin a sin b, with a along u and b along
        // v tabled per wave - every texel is then a few multiplies a wave.
        let tables: Vec<_> = self.waves.iter().map(|wave| {
            let table = |cycles: i32, phase: f64| -> (Vec<f64>, Vec<f64>) {
                (0..size).map(|texel| {
                    let angle = TAU * cycles as f64 * (texel as f64 + 0.5) / size as f64 + phase;
                    (angle.cos(), angle.sin())
                }).unzip()
            };
            (table(wave.cycles.0, wave.phase), table(wave.cycles.1, 0.0), Self::slope_amplitude(wave))
        }).collect();

        let encode = |slope: f64| (255.0 * linear_to_srgb((0.5 + slope / (2.0 * self.range)).clamp(0.0, 1.0))).round() as u8;
        let flat = encode(0.0);
        let mut pixels = Vec::with_capacity(size * size * 4);
        for v in 0..size {
            for u in 0..size {
                let (mut slope_u, mut slope_v) = (0.0, 0.0);
                for ((cos_u, sin_u), (cos_v, sin_v), (amplitude_u, amplitude_v)) in &tables {
                    let cos = cos_u[u] * cos_v[v] - sin_u[u] * sin_v[v];
                    slope_u += amplitude_u * cos;
                    slope_v += amplitude_v * cos;
                }
                pixels.extend_from_slice(&[encode(slope_u), encode(slope_v), flat, 255]);
            }
        }
        pixels
    }
}

fn linear_to_srgb(x: f64) -> f64 {
    if x <= 0.003_130_8 { 12.92 * x } else { 1.055 * x.powf(1.0 / 2.4) - 0.055 }
}

fn srgb_to_linear(x: f64) -> f64 {
    if x <= 0.040_45 { x / 12.92 } else { ((x + 0.055) / 1.055).powf(2.4) }
}

/// Where averaging a channel spread around 0.5 (standard deviation
/// `deviation`, 0..1 units) in sRGB, as the engine's mipmaps do, puts it -
/// relative to 0.5, once read back as linear.
fn srgb_mip_bias(deviation: f64) -> f64 {
    let (mut weight, mut encoded) = (0.0, 0.0);
    for step in -600..=600 {
        let z = step as f64 * 0.01;
        let density = (-0.5 * z * z).exp();
        weight += density;
        encoded += density * linear_to_srgb((0.5 + deviation * z).clamp(0.0, 1.0));
    }
    srgb_to_linear(encoded / weight) - 0.5
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ripple_map_on_disk_is_up_to_date() {
        let pixels = ripple_map().pixels();
        if std::env::var_os("OCEAN_WRITE_RIPPLES").is_some() {
            image::RgbaImage::from_raw(SIZE, SIZE, pixels).unwrap().save(RIPPLE_MAP).unwrap();
            return;
        }
        let on_disk = image::open(RIPPLE_MAP).unwrap_or_else(|error| panic!("{RIPPLE_MAP}: {error} - write it with OCEAN_WRITE_RIPPLES=1 cargo test ripple_map")).to_rgba8();
        assert_eq!(on_disk.dimensions(), (SIZE, SIZE));
        // Within one step - another machine's sin/cos may round a texel the
        // other way.
        let worst = on_disk.as_raw().iter().zip(&pixels).map(|(a, b)| a.abs_diff(*b)).max().unwrap_or(0);
        assert!(worst <= 1, "{RIPPLE_MAP} is out of date - write it with OCEAN_WRITE_RIPPLES=1 cargo test ripple_map");
    }

    #[test]
    fn the_map_tiles_and_the_shader_knows_its_band() {
        let map = ripple_map();
        assert!(map.waves.len() > WAVES * 3 / 4, "only {} ripple waves fit the tile", map.waves.len());
        let wgsl = std::fs::read_to_string("assets/shaders/ocean.wgsl").unwrap();
        assert!(wgsl.contains(&format!("const RIPPLE_SHORTEST: f32 = {};", SHORTEST / TILE)));
        assert!(wgsl.contains(&format!("const RIPPLE_LONGEST: f32 = {};", LONGEST / TILE)));
        assert!(wgsl.contains(&format!("const RIPPLE_HEX_SCALE: f32 = {HEX_SCALE};")));
        assert!(wgsl.contains(&format!("const RIPPLE_HEX_PERIOD: i32 = {HEX_PERIOD};")));
        println!("{} ripple waves, slope variance {:.4}, range +-{:.3}, mip bias {:?}", map.waves.len(), map.variance, map.range, map.mip_bias);
    }
}
