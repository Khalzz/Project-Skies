//! What the pilot's eyes do to the picture - the colors draining, the tunnel
//! closing in, the view dimming or washing red (see plane::pilot::Pilot::
//! screen_effects) - drawn by the game's `g_vision` Fullscreen shader
//! (assets/shaders/g_vision.wgsl), which copies the world and the flight
//! HUD onto the screen with these over both. Menus, drawn after it, stay
//! sharp.

use std::collections::BTreeMap;

use crate::engine::rendering::shader_library::ParamValue;

/// The play scene's Fullscreen shader - its material, by the same name.
pub const G_VISION: &str = "g_vision";

/// One frame's effects - all zero is the picture untouched.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScreenEffects {
    /// 0 = full color .. 1 = black and white.
    pub desaturation: f32,
    /// 0 = none .. 1 = fully closed: a soft round tunnel closing in from
    /// the screen's edges toward its center, in `tunnel_color`.
    pub tunnel: f32,
    /// 0 = normal .. 1 = black: darkens the whole scene evenly.
    pub darkening: f32,
    /// What the tunnel closes in with (linear rgb, alpha unused).
    pub tunnel_color: [f32; 4],
    /// A flat color over the whole scene (linear rgb), alpha = how much.
    pub tint: [f32; 4],
}

impl ScreenEffects {
    /// These as g_vision's params (see g_vision.wgsl's `Params`), with the
    /// flight HUD drawn at `hud_opacity` (0 hidden .. 1 as drawn).
    pub fn params(&self, hud_opacity: f32) -> BTreeMap<String, ParamValue> {
        BTreeMap::from([
            ("params".to_owned(), ParamValue::from([self.desaturation, self.tunnel, 0.0, self.darkening])),
            ("tunnel_color".to_owned(), ParamValue::from(self.tunnel_color)),
            ("tint".to_owned(), ParamValue::from(self.tint)),
            ("hud_params".to_owned(), ParamValue::from([hud_opacity, 0.0, 0.0, 0.0])),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn g_vision_reads_the_params_the_game_writes() {
        // Every field of g_vision.wgsl's Params, and nothing else.
        let code = std::fs::read_to_string("assets/shaders/g_vision.wgsl").unwrap();
        let start = code.find("struct Params {").expect("g_vision has params");
        let body = &code[start..start + code[start..].find("};").unwrap()];
        let mut fields: Vec<&str> = body.lines().skip(1)
            .map(str::trim)
            .filter(|line| !line.starts_with("//"))
            .filter_map(|line| line.split_once(':').map(|(name, _)| name.trim()))
            .collect();
        fields.sort();
        let written = ScreenEffects::default().params(1.0);
        assert_eq!(written.keys().map(String::as_str).collect::<Vec<_>>(), fields);
    }
}
