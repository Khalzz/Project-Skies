/// Fallback clear color used before any scene has applied an `Environment` -
/// see `App::new` and `resources::apply_environment`.
pub const DEFAULT_CLEAR_COLOR: wgpu::Color = wgpu::Color { r: 0.1, g: 0.2, b: 0.3, a: 1.0 };

/// Paths (relative to res/, joined the same way as `resources::load_binary`) to each
/// face of a skybox cubemap, in +X, -X, +Y, -Y, +Z, -Z order.
pub struct SkyboxFaces {
    pub px: String,
    pub nx: String,
    pub py: String,
    pub ny: String,
    pub pz: String,
    pub nz: String,
}

/// What a scene renders behind its geometry - declared by the scene itself (see
/// `Scene::reset`) and applied via `resources::apply_environment`, rather than
/// loaded automatically at startup.
pub enum Environment {
    Color(wgpu::Color),
    Skybox(SkyboxFaces),
}

impl Default for Environment {
    fn default() -> Self {
        Environment::Color(DEFAULT_CLEAR_COLOR)
    }
}
