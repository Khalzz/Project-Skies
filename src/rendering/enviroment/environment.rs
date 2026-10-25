use crate::rendering::enviroment::skybox_renderer::SkyboxRender;

/// The clear color of `Environment::default()`.
pub const DEFAULT_CLEAR_COLOR: wgpu::Color = wgpu::Color { r: 0.1, g: 0.2, b: 0.3, a: 1.0 };

/// Each face of a skybox cubemap, in +X, -X, +Y, -Y, +Z, -Z order - paths
/// relative to the assets folder (see `Assets::set_root`).
#[derive(Debug, Clone, PartialEq)]
pub struct SkyboxFaces {
    pub px: String,
    pub nx: String,
    pub py: String,
    pub ny: String,
    pub pz: String,
    pub nz: String,
}

/// What's drawn behind everything. The project has one
/// (`ProjectSettings::environment`) that every scene shows, unless a scene
/// sets its own (`SceneSettings::environment`).
#[derive(Debug, Clone, PartialEq)]
pub enum Environment {
    Color(wgpu::Color),
    Skybox(SkyboxFaces),
    /// Procedural clear-day "sea sky" - a computed atmospheric gradient with a
    /// sun disc and glow, no cubemap needed. See `sky.wgsl` /
    /// `SkyboxRender::new_procedural`. `sun`: whether the sun's disc and glow
    /// are drawn in it - pointing the way the lighting comes from - or it's
    /// just the sky.
    ProceduralSky { sun: bool },
}

impl Default for Environment {
    fn default() -> Self {
        Environment::Color(DEFAULT_CLEAR_COLOR)
    }
}

/// An `Environment` built and ready to draw - its skybox's textures loaded
/// (see `resources::prepare_environment`).
pub struct PreparedEnvironment {
    pub skybox: Option<SkyboxRender>,
    pub clear_color: wgpu::Color,
}
