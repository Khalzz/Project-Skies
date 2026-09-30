//! The ocean's wave field - one set of waves, generated from a real wind-sea
//! spectrum, drawn by the water shader (water.wgsl, via `OceanUniform`). The
//! waves are visual only: gameplay treats the water as a flat plane at the
//! sea's rest level (`OceanWaves::sea_level`).
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
//! every frame (`OceanWaves::uniform`) - the GPU only ever multiplies small
//! camera-relative distances, so waves stay crisp arbitrarily far from the
//! world origin, and the clock never wraps.

use std::f64::consts::TAU;

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};

const GRAVITY: f64 = 9.81;
/// Phillips constant of the Pierson-Moskowitz spectrum.
const PM_ALPHA: f64 = 0.0081;

/// Must match `MAX_WAVES` in water.wgsl.
pub const MAX_WAVES: usize = 80;
/// How many trail points the plane's wake can send the shader - must match
/// `MAX_WAKE_POINTS` in water.wgsl. See WaterRenderData's wake.
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
    /// Geometry waves: count and wavelength band (world units). The shortest
    /// must stay several mesh cells long - the play scene's water mesh has
    /// ~2-unit cells, so ~10 is the floor there.
    pub geometry_waves: usize,
    pub geometry_min_wavelength: f32,
    /// Normal-only detail waves: count and wavelength band.
    pub detail_waves: usize,
    pub detail_min_wavelength: f32,
    /// Scales the detail waves' effect on the normal.
    pub detail_strength: f32,
    /// 0..1 - how strongly whitecaps show on the sharpest crests.
    pub whitecap_strength: f32,
    /// Same settings + same seed = the exact same ocean.
    pub seed: u64,
}

impl Default for OceanSettings {
    fn default() -> Self {
        Self {
            wind_speed: 6.0,
            wind_direction: 0.35,
            directional_spread: 2.0,
            choppiness: 0.6,
            amplitude_scale: 1.0,
            time_scale: 1.0,
            geometry_waves: 40,
            geometry_min_wavelength: 10.0,
            detail_waves: 32,
            detail_min_wavelength: 0.5,
            detail_strength: 1.0,
            whitecap_strength: 0.6,
            seed: 0x5EA5_1DE5,
        }
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

/// GPU copy of the wave field - layout must match `Ocean` in water.wgsl.
/// Each wave is two vec4s: (dir.x, dir.z, k, amplitude) and
/// (steepness, phase at the camera, wavelength, unused).
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct OceanUniform {
    /// geometry wave count, detail wave count, base height, color height scale
    params: [f32; 4],
    /// choppiness, whitecap strength, detail strength, wind direction (radians)
    shading: [f32; 4],
    waves: [[f32; 4]; MAX_WAVES * 2],
    // The low-flying plane's wake (see WaterRenderData::update_wake) - all
    // zero when there's none. Filled in by the renderer, not here.
    /// Plane: camera-relative x, z, height above the water, intensity 0..1.
    pub wake_head: [f32; 4],
    /// Circle around the whole wake (camera-relative x, z, radius) - the
    /// shader skips pixels outside it - and the trail's max age (seconds).
    pub wake_bounds: [f32; 4],
    /// Trail point count, flight direction (horizontal unit x, z), unused.
    pub wake_info: [f32; 4],
    /// Trail, newest first: camera-relative x, z, age (s), intensity 0..1.
    pub wake_points: [[f32; 4]; MAX_WAKE_POINTS],
    /// Camera world x, z wrapped to WAKE_NOISE_PERIOD (so it stays precise
    /// anywhere), unused x2 - pins the foam's noise to the water.
    pub wake_anchor: [f32; 4],
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
    fn spread_angle(rng: &mut StdRng, spread: f64) -> f64 {
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

    /// This frame's GPU copy, with every wave's phase already evaluated at
    /// `camera` (true world XZ) in f64 - see this module's doc comment.
    pub fn uniform(&self, camera_x: f32, camera_z: f32) -> OceanUniform {
        let mut waves = [[0.0f32; 4]; MAX_WAVES * 2];
        for (slot, wave) in self.geometry.iter().chain(self.detail.iter()).enumerate() {
            let phase = wave.phase_at(camera_x as f64, camera_z as f64, self.time).rem_euclid(TAU);
            waves[slot * 2] = [wave.direction[0] as f32, wave.direction[1] as f32, wave.k as f32, wave.amplitude as f32];
            waves[slot * 2 + 1] = [wave.steepness as f32, phase as f32, wave.wavelength() as f32, 0.0];
        }

        OceanUniform {
            params: [self.geometry.len() as f32, self.detail.len() as f32, self.base_height as f32, self.color_height_scale as f32],
            shading: [self.settings.choppiness, self.settings.whitecap_strength, self.settings.detail_strength, self.settings.wind_direction],
            waves,
            wake_head: [0.0; 4],
            wake_bounds: [0.0; 4],
            wake_info: [0.0; 4],
            wake_points: [[0.0; 4]; MAX_WAKE_POINTS],
            wake_anchor: [0.0; 4],
        }
    }
}
