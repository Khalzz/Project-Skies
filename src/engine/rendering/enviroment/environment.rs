use crate::engine::rendering::enviroment::skybox_renderer::SkyboxRender;

/// Fallback clear color used before any scene has applied an `Environment` -
/// see `App::new` and `resources::apply_environment`.
pub const DEFAULT_CLEAR_COLOR: wgpu::Color = wgpu::Color { r: 0.1, g: 0.2, b: 0.3, a: 1.0 };

/// Paths (relative to res/, joined the same way as `resources::load_binary`) to each
/// face of a skybox cubemap, in +X, -X, +Y, -Y, +Z, -Z order.
#[derive(Clone)]
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
#[derive(Clone)]
pub enum Environment {
    Color(wgpu::Color),
    Skybox(SkyboxFaces),
    /// Procedural clear-day "sea sky" - a computed atmospheric gradient with a
    /// sun disc and glow, no cubemap needed. See `sky.wgsl` /
    /// `SkyboxRender::new_procedural`. Still lands in `SceneEnvironment::skybox`
    /// like a real skybox does - it renders through the same path.
    ProceduralSky,
}

impl Default for Environment {
    fn default() -> Self {
        Environment::Color(DEFAULT_CLEAR_COLOR)
    }
}

/// What a scene currently has on screen behind its geometry - the *applied*
/// result of an `Environment` (see `resources::apply_environment`/
/// `PreparedSceneAssets::apply`), not the declarative `Environment` a scene
/// hands those functions. Lives on `Scene` as `scene.environment` -
/// scene-scoped the same way `SceneCameras`/`SceneContent` are (see those
/// types' own doc comments), and for the same reason: a skybox one scene
/// loaded shouldn't keep rendering behind a different scene that never asked
/// for one - a fresh `Scene::new` naturally starts back at `Default::default()`
/// (no skybox, the plain fallback clear color) with no separate reset step
/// needed.
pub struct SceneEnvironment {
    pub skybox: Option<SkyboxRender>,
    pub clear_color: wgpu::Color,
}

impl Default for SceneEnvironment {
    fn default() -> Self {
        Self { skybox: None, clear_color: DEFAULT_CLEAR_COLOR }
    }
}
