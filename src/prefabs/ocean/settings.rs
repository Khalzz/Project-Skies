//! What the sea looks like - its sea state and how much of it is drawn. The
//! defaults are the sea as tuned for the game: a calm sea, no summed swell,
//! broad ripples from the ripple map, soft depth patches.

use super::waves::MAX_WAVES;

/// How high above the sea the camera can go before it starts taking detail
/// away (see OceanSettings::altitude_detail), m.
pub(super) const ALTITUDE_DETAIL_START: f32 = 50.0;

#[derive(Clone, Debug)]
pub struct OceanSettings {
    /// Wind speed (m/s) - the one knob that sets the whole sea state: the
    /// dominant wavelength grows with its square, and so does wave height
    /// (significant height ~= 0.021 * speed^2 - 6 m/s gives ~0.8 m).
    pub wind_speed: f32,
    /// Direction the wind (and so most waves) blows toward, radians in the
    /// XZ plane (0 = +X, PI/2 = +Z).
    pub wind_direction: f32,
    /// How tightly wave directions cluster around the wind - higher is more
    /// aligned swell, lower is more confused, choppy sea. 2 is typical.
    pub directional_spread: f32,
    /// 0..1 - how far crests are pulled together into sharp peaks (and how
    /// easily whitecaps form). Above ~0.8 the surface starts folding.
    pub choppiness: f32,
    /// Multiplies every wave's height - 1.0 is the physical spectrum.
    pub amplitude_scale: f32,
    /// Multiplies time - 1.0 is physical wave speed.
    pub time_scale: f32,
    /// Geometry waves: count and wavelength band (world units). The mesh only
    /// moves the ones at least ~3 of its cells long (its finest ring has
    /// 4-unit cells - see mesh::RINGS); shorter ones still shade
    /// every pixel.
    ///
    /// Every pixel near the camera sums every wave (geometry and detail) -
    /// the count is most of what the sea costs to draw. The total energy
    /// doesn't depend on it (each wave carries its band's share), only how
    /// busy the surface looks.
    pub geometry_waves: usize,
    pub geometry_min_wavelength: f32,
    /// Normal-only detail waves: count and wavelength band.
    pub detail_waves: usize,
    pub detail_min_wavelength: f32,
    /// Scales the detail waves' effect on the normal.
    pub detail_strength: f32,
    /// 0..1 - how strongly whitecaps show on the sharpest crests.
    pub whitecap_strength: f32,
    /// At most this many waves summed per pixel - the longest; the rest
    /// count as faded out there (ocean.wgsl's surface_waves). MAX_WAVES = no
    /// cap. Caps the cost of the water right under the camera.
    pub pixel_wave_cap: usize,
    /// A wave fully shows on a pixel once its wavelength spans this many
    /// pixels, and is gone by half that - higher drops short waves sooner
    /// (cheaper, softer), lower keeps them longer (sharper, may shimmer).
    pub pixel_full_pixels: f32,
    /// Draw the detail waves from the ripple map (see ripples) -
    /// two texture reads a pixel - instead of summing them one by one
    /// (`detail_waves` of them). The geometry waves are summed either way.
    pub textured_ripples: bool,
    /// Draw the sea as a flat plane: no wave moves the mesh or shades a
    /// pixel - only the water's color, sky reflection and sun glint (and
    /// the wake). The waves are still made, so the sea level is the same.
    pub flat: bool,
    /// How much detail the camera's altitude takes away: above
    /// ALTITUDE_DETAIL_START, waves shorter than this many meters per meter
    /// of height are dropped (and ones up to twice that faded) - 0.02: the
    /// ripples are gone by ~450 m up, only the longest swell is left at
    /// ~3 km, and the default sea is flat from ~4 km. 0 = off.
    pub altitude_detail: f32,
    /// How the ripple map is drawn (with `textured_ripples`): two layers
    /// (6 texture reads) or one (half that); stochastically tiled (each
    /// layer 3 reads, no visible repeat) or plainly (1 read, tiles show from
    /// a few hundred meters up).
    pub ripple_layers: u32,
    pub ripple_stochastic: bool,
    /// How strongly the far sea is shaped by the ripple map drawn 64x larger
    /// (30-500 m patterns), fading in as the near ripples fade out - so the
    /// open sea isn't one smooth plane. 0 = off (one layer less to read).
    pub far_variation: f32,
    /// How much bigger than the map's own size (ripple::TILE) the near
    /// ripples are drawn - bigger is broader, less skin-like ripples (their
    /// steepness doesn't change). The far layer keeps its own size.
    pub ripple_scale: f32,
    /// Where the far look takes over (m from the camera): the near ripples
    /// fade out and the far layer in between these two.
    pub far_start: f32,
    pub far_end: f32,
    /// 0..2 - how strongly the water's color drifts between its deep and
    /// light blues in broad patches pinned to the sea (tens to hundreds of
    /// meters), as if its depth changed - the color the swell used to give.
    pub depth_tint: f32,
    /// Same settings + same seed = the exact same ocean.
    pub seed: u64,
}

impl Default for OceanSettings {
    fn default() -> Self {
        // As tuned in the ocean lab (its top quality): a calm sea, no
        // summed swell at all, broad ripples from the ripple map - two
        // stochastic layers, no far layer: they reach 2-5 km before fading
        // - soft depth patches.
        Self {
            wind_speed: 2.0,
            wind_direction: 0.35,
            directional_spread: 2.0,
            choppiness: 0.6,
            amplitude_scale: 1.0,
            time_scale: 1.0,
            geometry_waves: 0,
            geometry_min_wavelength: 10.0,
            detail_waves: 24,
            detail_min_wavelength: 0.5,
            detail_strength: 1.5,
            whitecap_strength: 0.6,
            pixel_wave_cap: MAX_WAVES,
            pixel_full_pixels: 13.5,
            textured_ripples: true,
            flat: false,
            altitude_detail: 0.0,
            ripple_layers: 2,
            ripple_stochastic: true,
            far_variation: 0.0,
            ripple_scale: 4.9,
            far_start: 2_000.0,
            far_end: 5_000.0,
            depth_tint: 0.62,
            seed: 0x5EA5_1DE5,
        }
    }
}
