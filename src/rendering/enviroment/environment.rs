use crate::rendering::enviroment::skybox_renderer::SkyboxRender;
use crate::ui::color::UiColor;

/// The clear color of `Environment::default()`.
pub const DEFAULT_CLEAR_COLOR: UiColor = UiColor::Rgb(89, 124, 149);

/// `color` as the GPU clears to it: made linear first when the screen
/// stores sRGB (`srgb_target`), so it comes out as the color it names - the
/// same way UI and 2D colors are.
pub(crate) fn clear_color(color: UiColor, srgb_target: bool) -> wgpu::Color {
    let [r, g, b, a] = color.to_rgba_f32();
    let channel = |value: f32| f64::from(if srgb_target { crate::rendering::sdf_text::srgb_to_linear(value) } else { value });
    wgpu::Color { r: channel(r), g: channel(g), b: channel(b), a: f64::from(a) }
}

/// Each face of a skybox cubemap, in +X, -X, +Y, -Y, +Z, -Z order - paths
/// relative to the assets folder (see `AssetFolder::set_root`).
#[derive(Debug, Clone, PartialEq)]
pub struct SkyboxFaces {
    pub px: String,
    pub nx: String,
    pub py: String,
    pub ny: String,
    pub pz: String,
    pub nz: String,
}

/// What's drawn behind everything - `app.renderer.set_environment`. One for
/// every scene, until it's set again.
#[derive(Debug, Clone, PartialEq)]
pub enum Environment {
    /// A plain color - nothing drawn, the screen just cleared to it.
    Color(UiColor),
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
