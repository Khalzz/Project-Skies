//! The ocean's wave field - one set of waves, generated from a real wind-sea
//! spectrum, drawn by the ocean shader (assets/shaders/ocean.wgsl, through
//! its material's params - see `OceanWaves::params`). The waves are visual
//! only: gameplay treats the water as a flat plane at the sea's rest level
//! (`OceanWaves::sea_level`).
//!
//! Two groups of waves:
//! - geometry waves (`OceanSettings::geometry_waves`) - Gerstner waves that
//!   actually move the water mesh's vertices: height, plus a sideways push
//!   toward each crest (`choppiness`) that sharpens crests and flattens
//!   troughs like real wind waves;
//! - detail waves (`OceanSettings::detail_waves`) - shorter than the mesh can
//!   resolve, so they only bend the per-pixel normal (ripples, glints).
//!
//! Wavelengths are spread (log-spaced, jittered) across the band the wind
//! actually builds, directions are spread around the wind, and every
//! wavelength is unrelated to the others - so the sum never repeats, at any
//! distance. Amplitudes and speeds follow the Pierson-Moskowitz spectrum and
//! deep-water dispersion (longer waves travel faster), so a single wind
//! speed sets a believable sea state.
//!
//! Precision: phases are computed on the CPU in f64, relative to the camera,
//! every frame (`OceanWaves::params`) - the GPU only ever multiplies small
//! camera-relative distances, so waves stay crisp arbitrarily far from the
//! world origin, and the clock never wraps.

use std::collections::BTreeMap;
use std::f64::consts::TAU;

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

pub(super) const GRAVITY: f64 = 9.81;
/// Phillips constant of the Pierson-Moskowitz spectrum.
pub(super) const PM_ALPHA: f64 = 0.0081;

/// Must match `MAX_WAVES` in ocean.wgsl.
pub const MAX_WAVES: usize = 80;
/// How many trail points the plane's wake can send the shader - must match
/// `MAX_WAKE_POINTS` in ocean.wgsl. See game::ocean's wake.
pub const MAX_WAKE_POINTS: usize = 32;

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
    /// 4-unit cells - see main.rs's OCEAN_TIERS); shorter ones still shade
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
    /// Draw the detail waves from the ripple map (game::ocean::ripples) -
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
        // OceanQuality::Ultra - tuned in the ocean lab: a calm sea, no
        // summed swell at all, broad ripples from the ripple map (no far
        // layer - they reach 2-5 km before fading), soft depth patches.
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

/// How much the sea may cost to draw - presets for what OceanSettings has
/// on that (how the ripple map is read, flat water, shore foam); the rest of
/// the settings is the sea state, which they leave alone. The top one,
/// `Ultra`, is the default - the look the sea is tuned for - and each one
/// below it gives up a little more of it for speed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OceanQuality {
    /// For the weakest GPUs: the sea as a flat plane - no waves at all,
    /// only its color, sky reflection and sun glint - and no shore foam.
    VeryLow,
    /// One plain ripple layer (1 texture read - its tiling can show from
    /// high up), no shore foam.
    Low,
    /// One stochastic ripple layer (3 reads), no shore foam.
    Medium,
    /// One stochastic ripple layer (3 reads), shore foam.
    High,
    /// The default and the most detail: two stochastic ripple layers (6
    /// reads), shore foam.
    Ultra,
}

/// What a preset sets - see OceanQuality.
struct QualityValues {
    ripple_layers: u32,
    ripple_stochastic: bool,
    flat: bool,
    shore_foam: bool,
}

impl OceanQuality {
    pub const ALL: [OceanQuality; 5] = [OceanQuality::VeryLow, OceanQuality::Low, OceanQuality::Medium, OceanQuality::High, OceanQuality::Ultra];

    pub fn name(self) -> &'static str {
        match self {
            OceanQuality::VeryLow => "Very low",
            OceanQuality::Low => "Low",
            OceanQuality::Medium => "Medium",
            OceanQuality::High => "High",
            OceanQuality::Ultra => "Ultra",
        }
    }

    /// One line on what it trades - for menus.
    pub fn description(self) -> &'static str {
        match self {
            OceanQuality::VeryLow => "flat water - no waves, no shore foam",
            OceanQuality::Low => "1 plain ripple layer (1 read), no shore foam",
            OceanQuality::Medium => "1 ripple layer (3 reads), no shore foam",
            OceanQuality::High => "1 ripple layer (3 reads), shore foam",
            OceanQuality::Ultra => "2 ripple layers (6 reads), shore foam (the default)",
        }
    }

    fn values(self) -> QualityValues {
        let (ripple_layers, ripple_stochastic, flat, shore_foam) = match self {
            OceanQuality::VeryLow => (1, false, true, false),
            OceanQuality::Low => (1, false, false, false),
            OceanQuality::Medium => (1, true, false, false),
            OceanQuality::High => (1, true, false, true),
            OceanQuality::Ultra => (2, true, false, true),
        };
        QualityValues { ripple_layers, ripple_stochastic, flat, shore_foam }
    }

    /// `settings` at this quality - the sea state kept. The waves
    /// themselves are the default's (Ultra's) at every level: only how
    /// they're drawn changes.
    pub fn apply(self, settings: &mut OceanSettings) {
        let defaults = OceanSettings::default();
        let values = self.values();
        settings.geometry_waves = defaults.geometry_waves;
        settings.detail_waves = defaults.detail_waves;
        settings.pixel_wave_cap = defaults.pixel_wave_cap;
        settings.pixel_full_pixels = defaults.pixel_full_pixels;
        settings.textured_ripples = true;
        settings.ripple_layers = values.ripple_layers;
        settings.ripple_stochastic = values.ripple_stochastic;
        settings.flat = values.flat;
    }

    /// The preset `settings` are at, if any - the first one, when two only
    /// differ by their shore foam (High and Medium; see `matches`).
    pub fn of(settings: &OceanSettings) -> Option<Self> {
        Self::ALL.into_iter().find(|quality| quality.matches(settings))
    }

    /// Whether `settings` are at this preset (its shore foam is the frame
    /// plan's, not theirs - see `shore_foam`).
    pub fn matches(self, settings: &OceanSettings) -> bool {
        let defaults = OceanSettings::default();
        let values = self.values();
        settings.geometry_waves == defaults.geometry_waves
                && settings.detail_waves == defaults.detail_waves
                && settings.pixel_wave_cap == defaults.pixel_wave_cap
                && settings.pixel_full_pixels == defaults.pixel_full_pixels
                && settings.textured_ripples
                && settings.ripple_layers == values.ripple_layers
                && settings.ripple_stochastic == values.ripple_stochastic
                && settings.flat == values.flat
    }

    /// Whether the sea is drawn flat at this quality (OceanSettings::flat).
    pub fn flat(self) -> bool {
        self.values().flat
    }

    /// Whether the shore foam - and the depth copy it reads every frame
    /// (frame_plans::standard's CopyDepth) - is drawn at this quality.
    pub fn shore_foam(self) -> bool {
        self.values().shore_foam
    }
}

#[derive(Clone, Copy, Debug)]
struct Wave {
    /// Unit travel direction in the XZ plane.
    direction: [f64; 2],
    /// Wavenumber, 2*PI / wavelength.
    k: f64,
    /// Angular frequency (deep-water dispersion: sqrt(g * k)).
    omega: f64,
    amplitude: f64,
    /// Gerstner steepness - how far this wave pushes the surface sideways
    /// toward its crests (0 = plain sine wave).
    steepness: f64,
    phase: f64,
}

impl Wave {
    fn wavelength(&self) -> f64 {
        TAU / self.k
    }

    /// Phase at world point (x, z), time t.
    fn phase_at(&self, x: f64, z: f64, t: f64) -> f64 {
        self.k * (self.direction[0] * x + self.direction[1] * z) - self.omega * t + self.phase
    }
}

/// The wake foam's noise repeats every this many meters (too far apart to
/// notice) - the shader's WAKE_NOISE_PERIOD, keep in sync.
pub const WAKE_NOISE_PERIOD: f64 = 4096.0;

pub struct OceanWaves {
    settings: OceanSettings,
    geometry: Vec<Wave>,
    detail: Vec<Wave>,
    /// Added to every height, so the surface sits around this level instead
    /// of dipping below Y=0 - 3 standard deviations of the geometry waves'
    /// height, which troughs essentially never exceed. Also the flat plane
    /// gameplay treats as the water (see `sea_level`).
    base_height: f64,
    /// Typical crest height (2 standard deviations) - the shader maps
    /// -this..+this onto its deep..light color ramp.
    color_height_scale: f64,
    /// Seconds since this ocean was created, scaled by `time_scale`.
    time: f64,
}

impl OceanWaves {
    pub fn new(settings: OceanSettings) -> Self {
        let mut rng = StdRng::seed_from_u64(settings.seed);
        let geometry_count = settings.geometry_waves.min(MAX_WAVES);
        let detail_count = settings.detail_waves.min(MAX_WAVES - geometry_count);

        let wind = settings.wind_speed.max(0.5) as f64;
        // Pierson-Moskowitz peak - where most of the wind's energy sits.
        let peak_omega = 0.855 * GRAVITY / wind;
        let peak_wavelength = TAU * GRAVITY / (peak_omega * peak_omega);

        // Geometry band: from the mesh's floor up to where the spectrum's
        // long side has faded out (~2.5x the peak). Detail band: from its
        // own floor up to where geometry waves take over.
        let geometry_min = settings.geometry_min_wavelength as f64;
        let geometry_max = (peak_wavelength * 2.5).max(geometry_min * 1.5);
        let detail_min = settings.detail_min_wavelength as f64;

        let mut geometry = Self::generate(&mut rng, &settings, peak_omega, geometry_min, geometry_max, geometry_count);
        let detail = Self::generate(&mut rng, &settings, peak_omega, detail_min, geometry_min, detail_count);

        // Gerstner steepness split evenly across the geometry waves (each
        // one's k*a*Q adds up to `choppiness` in total) - keeps the sum from
        // folding over as long as choppiness stays under 1.
        let choppiness = settings.choppiness.clamp(0.0, 1.0) as f64;
        let wave_count = geometry.len().max(1) as f64;
        for wave in &mut geometry {
            let ka = wave.k * wave.amplitude;
            wave.steepness = if ka > 0.0 { choppiness / (ka * wave_count) } else { 0.0 };
        }

        // Each wave's variance is a^2 / 2 - their sum is the surface's.
        let variance: f64 = geometry.iter().map(|wave| wave.amplitude * wave.amplitude * 0.5).sum();
        let deviation = variance.sqrt();

        Self {
            settings,
            geometry,
            detail,
            base_height: deviation * 3.0,
            color_height_scale: (deviation * 2.0).max(1e-3),
            time: 0.0,
        }
    }

    /// `count` waves across [min_wavelength, max_wavelength]: log-spaced
    /// bins, one wave per bin at a random point inside it (so no two
    /// wavelengths line up), direction spread around the wind, random phase,
    /// amplitude carrying its bin's share of the spectrum's energy.
    ///
    /// Shortest to longest - the shader relies on it: it walks them longest
    /// first and stops at the first one too short to show (ocean.wgsl's
    /// gerstner and surface_waves).
    fn generate(rng: &mut StdRng, settings: &OceanSettings, peak_omega: f64, min_wavelength: f64, max_wavelength: f64, count: usize) -> Vec<Wave> {
        if count == 0 || max_wavelength <= min_wavelength {
            return Vec::new();
        }
        let (log_min, log_max) = (min_wavelength.ln(), max_wavelength.ln());
        let bin = (log_max - log_min) / count as f64;
        let omega_of = |wavelength: f64| (GRAVITY * TAU / wavelength).sqrt();

        (0..count).map(|i| {
            let (low, high) = ((log_min + bin * i as f64).exp(), (log_min + bin * (i + 1) as f64).exp());
            let wavelength = rng.gen_range(low..high);
            let k = TAU / wavelength;
            let omega = omega_of(wavelength);
            // Shorter wavelength = higher frequency, so the bin's frequency
            // width runs high..low.
            let bin_omega = omega_of(low) - omega_of(high);

            let spectrum = PM_ALPHA * GRAVITY * GRAVITY * omega.powi(-5) * (-1.25 * (peak_omega / omega).powi(4)).exp();
            let amplitude = (2.0 * spectrum * bin_omega).sqrt() * settings.amplitude_scale as f64;

            let angle = settings.wind_direction as f64 + Self::spread_angle(rng, settings.directional_spread as f64);
            Wave {
                direction: [angle.cos(), angle.sin()],
                k,
                omega,
                amplitude,
                steepness: 0.0,
                phase: rng.gen_range(0.0..TAU),
            }
        }).collect()
    }

    /// Angle off the wind, drawn with density ~ cos(angle)^spread over
    /// -90..90 degrees (rejection sampling).
    pub(super) fn spread_angle(rng: &mut StdRng, spread: f64) -> f64 {
        loop {
            let angle = rng.gen_range(-std::f64::consts::FRAC_PI_2..std::f64::consts::FRAC_PI_2);
            if rng.gen::<f64>() <= angle.cos().max(0.0).powf(spread) {
                return angle;
            }
        }
    }

    pub fn settings(&self) -> &OceanSettings {
        &self.settings
    }

    /// The waves made again from `settings`, on the same clock - same
    /// settings and seed give the very same waves, so only what changed
    /// does.
    pub fn set_settings(&mut self, settings: OceanSettings) {
        let time = self.time;
        *self = Self::new(settings);
        self.time = time;
    }

    /// Advances the ocean's clock - once per frame.
    pub fn advance(&mut self, delta_time: f32) {
        self.time += delta_time as f64 * self.settings.time_scale as f64;
    }

    /// The sea's rest level (world Y) - the middle the waves move around.
    /// Gameplay treats the water as this flat plane (water-death check,
    /// splash); the waves themselves are visual only, nothing evaluates them
    /// on the CPU.
    pub fn sea_level(&self) -> f32 {
        self.base_height as f32
    }

    /// The geometry waves' wavelengths, shortest first - the ones a mesh
    /// ring can move (see ocean.wgsl's geometry_share).
    pub fn geometry_wavelengths(&self) -> impl Iterator<Item = f32> + '_ {
        self.geometry.iter().map(|wave| wave.wavelength() as f32)
    }

    /// How far the geometry waves can move a point of the water at most:
    /// sideways (every wave's Gerstner push together) and up/down from the
    /// sea level (every amplitude together) - for culling the mesh.
    pub fn reach(&self) -> (f32, f32) {
        let sideways = self.geometry.iter().map(|wave| wave.steepness * wave.amplitude).sum::<f64>();
        let vertical = self.geometry.iter().map(|wave| wave.amplitude).sum::<f64>();
        (sideways as f32, vertical as f32)
    }

    /// The slope variance of every wave together - each one's (k * a)^2 / 2,
    /// the detail waves' scaled by `detail_strength` like the shader does.
    /// The shader compares what still shows at a pixel against it (see
    /// ocean.wgsl's surface_waves) - it's the same every pixel, so it's
    /// summed once here instead.
    fn total_slope_variance(&self) -> f64 {
        let strength = self.settings.detail_strength as f64;
        let variance = |wave: &Wave, scale: f64| 0.5 * (wave.k * wave.amplitude * scale).powi(2);
        self.geometry_slope_variance() + self.detail.iter().map(|wave| variance(wave, strength)).sum::<f64>()
    }

    /// The geometry waves' share of `total_slope_variance` - what the
    /// ripple map's variance is added to when it stands in for the detail
    /// waves (see game::ocean::ripples).
    pub fn geometry_slope_variance(&self) -> f64 {
        self.geometry.iter().map(|wave| 0.5 * (wave.k * wave.amplitude).powi(2)).sum()
    }

    /// The ocean's clock (seconds, time-scaled) - what the waves and the
    /// ripple map's drift run on.
    pub fn time(&self) -> f64 {
        self.time
    }

    /// This frame's waves for the ocean shader's `Ocean` params - `params`,
    /// `shading`, `waves` (see ocean.wgsl) - with every wave's phase already
    /// evaluated at `camera` (true world XZ) in f64 - see this module's doc
    /// comment. Each wave is two vec4s: (dir.x, dir.z, k, amplitude) and
    /// (steepness, phase at the camera, wavelength, unused).
    pub fn params(&self, camera_x: f32, camera_z: f32) -> BTreeMap<String, Vec<f32>> {
        let mut waves = vec![0.0f32; MAX_WAVES * 2 * 4];
        for (slot, wave) in self.geometry.iter().chain(self.detail.iter()).enumerate() {
            let phase = wave.phase_at(camera_x as f64, camera_z as f64, self.time).rem_euclid(TAU);
            waves[slot * 8..slot * 8 + 8].copy_from_slice(&[
                wave.direction[0] as f32, wave.direction[1] as f32, wave.k as f32, wave.amplitude as f32,
                wave.steepness as f32, phase as f32, wave.wavelength() as f32, 0.0,
            ]);
        }
        BTreeMap::from([
            // Flat: no waves for the shader to walk, in the mesh or per pixel.
            ("params".to_owned(), if self.settings.flat {
                vec![0.0, 0.0, self.base_height as f32, self.color_height_scale as f32]
            } else {
                vec![self.geometry.len() as f32, self.detail.len() as f32, self.base_height as f32, self.color_height_scale as f32]
            }),
            ("shading".to_owned(), vec![self.settings.choppiness, self.settings.whitecap_strength, self.settings.detail_strength, self.total_slope_variance() as f32]),
            ("waves".to_owned(), waves),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_settings_are_the_top_quality() {
        assert_eq!(OceanQuality::of(&OceanSettings::default()), Some(OceanQuality::Ultra));
        for quality in OceanQuality::ALL {
            let mut settings = OceanSettings::default();
            quality.apply(&mut settings);
            assert!(quality.matches(&settings), "{quality:?}");
            assert!(settings.geometry_waves + settings.detail_waves <= MAX_WAVES);
        }
    }

    #[test]
    fn each_wave_group_is_sorted_shortest_to_longest() {
        // ocean.wgsl stops at the first wave too short to show, walking each
        // group longest first - out of order, it would drop visible waves.
        // (The default sea has no geometry waves - give it some.)
        let waves = OceanWaves::new(OceanSettings { geometry_waves: 28, wind_speed: 6.0, ..OceanSettings::default() });
        for group in [&waves.geometry, &waves.detail] {
            assert!(group.windows(2).all(|pair| pair[0].wavelength() < pair[1].wavelength()));
        }
        // And every detail wave is shorter than every geometry wave.
        let longest_detail = waves.detail.last().unwrap().wavelength();
        assert!(longest_detail < waves.geometry[0].wavelength());
    }
}
