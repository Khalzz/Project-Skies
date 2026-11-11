//! The sea - its waves, and the wake a jet leaves on it flying fast and low -
//! drawn with the game's ocean shader (assets/shaders/ocean.wgsl).
//!
//! Using it:
//! - `register` once at startup (main.rs) - the mesh, and the sea level and
//!   wind the particles splash and drift with;
//! - a node with `surface()` in the scene;
//! - an `Ocean` the scene keeps, and `Ocean::update` from the scene's
//!   `after_cameras` every frame: that's when the camera is where it's drawn
//!   from, and the waves are worked out around exactly that point (in f64 -
//!   see `waves`), so they don't swim against the mesh.
//!
//! The parts: `settings` (what the sea looks like), `waves` (the wave
//! field), `ripples` (the ripple map's numbers), `wake` (the foam trail),
//! `mesh` (the ring mesh) and `shader` (the material and the rest of its
//! params).

mod mesh;
mod ripples;
mod settings;
mod shader;
mod wake;
mod waves;

use nalgebra::Vector3;

use pankarta_software::prelude::{App, Mesh};
use pankarta_software::scene_manager::scene::Scene;

pub use settings::OceanSettings;
pub use wake::WakeSource;

use shader::Params;
use wake::Wake;
use waves::OceanWaves;

/// Adds the sea's mesh to the engine, and gives the particles the default
/// sea's level and wind (they splash on it and drift with it).
pub fn register(app: &mut App) {
    mesh::register(app);
    let ocean = Ocean::default();
    app.particles.set_sea_level(Some(ocean.sea_level()));
    app.particles.set_wind(ocean.wind());
}

/// What the sea's node is drawn with - the ring mesh (in world units, so at
/// scale 1) with the ocean shader, its values filled in by `Ocean::update`.
pub fn surface() -> Mesh {
    Mesh::custom(mesh::MESH).material(shader::material()).casts_shadows(false)
}

pub struct Ocean {
    waves: OceanWaves,
    wake: Wake,
    // Extra XZ added to the camera when phasing the waves, then cleared.
    // The title uses it so the sea streams past a camera that stays put.
    flow: (f32, f32),
}

impl Default for Ocean {
    fn default() -> Self {
        Self::new(OceanSettings::default())
    }
}

impl Ocean {
    pub fn new(settings: OceanSettings) -> Self {
        Self { waves: OceanWaves::new(settings), wake: Wake::default(), flow: (0.0, 0.0) }
    }

    /// The sea's rest level (world Y) - gameplay treats the water as this
    /// flat plane; the waves are visual only.
    pub fn sea_level(&self) -> f32 {
        self.waves.sea_level()
    }

    /// The wind over it - toward where it blows, m/s.
    pub fn wind(&self) -> Vector3<f32> {
        let settings = self.waves.settings();
        Vector3::new(settings.wind_direction.cos(), 0.0, settings.wind_direction.sin()) * settings.wind_speed
    }

    /// Slides the wave field by `x`/`z` (world units) for this frame, as if
    /// the camera had moved there. Cleared by `update`, so a scene sets it
    /// every frame it wants the sea to stream.
    pub fn set_flow(&mut self, x: f32, z: f32) {
        self.flow = (x, z);
    }

    /// Slides the foam trail already laid by `x`/`z` (world units). The title
    /// jet doesn't move, so its wake is carried along with the sea instead.
    pub fn scroll_wake(&mut self, x: f32, z: f32) {
        self.wake.scroll(x, z);
    }

    /// What's stirring up the water this frame (`None` when nothing is) -
    /// set every frame, before `update`. How strongly it shows depends on
    /// its speed and how close it is to the water (see `wake_strength`).
    pub fn set_wake_source(&mut self, source: Option<WakeSource>) {
        self.wake.source = source;
    }

    /// How hard `source` stirs the water, 0..1 - stronger the lower and
    /// faster it goes. The same strength the wake is drawn with, for
    /// anything that should match it (e.g. spray particles).
    pub fn wake_strength(&self, source: &WakeSource) -> f32 {
        wake::strength(source, self.sea_level())
    }

    /// Advances the waves by `delta_time`, keeps the sea node `node` centred
    /// under the camera (only XZ), and puts this frame's waves and wake into
    /// its material - worked out around where the camera is now. From the
    /// scene's `after_cameras`, every frame (see the module doc).
    pub fn update(&mut self, scene: &mut Scene, node: &str, delta_time: f32) {
        let camera = scene.cameras.active().camera.position().coords;
        // The rings are around the camera, so the water never runs out from
        // under it - and the waves are pinned to the world, not the mesh, so
        // sliding it doesn't move them.
        if let Some(transform) = scene.get_node_mut(node).and_then(|node| node.transform_3d_mut()) {
            transform.position.x = camera.x;
            transform.position.z = camera.z;
        }
        self.waves.advance(delta_time);
        let intensity = self.wake.update(delta_time, self.sea_level());
        let params = self.params(camera, intensity);
        self.flow = (0.0, 0.0);
        shader::set_on(scene, node, &params);
    }

    /// Every param the shader reads, for a camera at `camera` (true world
    /// position) and a wake source stirring the water `intensity` hard now.
    fn params(&self, camera: Vector3<f32>, intensity: f32) -> Params {
        let (x, z) = (camera.x + self.flow.0, camera.z + self.flow.1);
        let settings = self.waves.settings();
        let mut params = self.waves.params(x, z);
        self.wake.write(&mut params, camera, intensity, self.sea_level());
        shader::write_tuning(&mut params, settings);
        shader::write_detail(&mut params, settings, camera.y - self.sea_level());
        shader::write_ripples(&mut params, &self.waves, x as f64, z as f64);
        params
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_writes_every_param_the_ocean_shader_reads() {
        let params = Ocean::default().params(Vector3::zeros(), 0.0);
        let mut names: Vec<&str> = params.keys().map(String::as_str).collect();
        names.sort_unstable();
        let mut expected = [
            "params",
            "shading",
            "waves",
            "wake_head",
            "wake_bounds",
            "wake_info",
            "wake_points",
            "wake_anchor",
            "tuning",
            "ripple_a",
            "ripple_b",
            "ripple_info",
            "ripple_bias",
            "ripple_cells",
            "detail",
            "ripple_far",
            "ripple_far_cells",
            "look",
        ];
        expected.sort_unstable();
        assert_eq!(names, expected);
        assert_eq!(params["waves"].len(), waves::MAX_WAVES * 2 * 4);
        assert_eq!(params["wake_points"].len(), wake::MAX_WAKE_POINTS * 4);
    }
}
