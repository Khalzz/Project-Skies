//! The splash shown at start - "A Pankarta Software Production" on black,
//! fading out at the end - while the main menu loads behind it.

use glyphon::cosmic_text::Align;
use pankarta_software::prelude::*;
use pankarta_software::ui::ui_node::UiNode;
use pankarta_software::ui::ui_transform::PositionValue;

const NEXT_SCENE: &str = "main_menu";
const TEXT: &str = "A Pankarta Software Production";
/// How long it shows, fade included.
const DURATION_SECS: f32 = 3.0;
const FADE_OUT_SECS: f32 = 0.5;
const NODE: &str = "splash";

pub struct Splash {
    elapsed: f32,
}

impl Splash {
    pub fn new(scene: &mut Scene, app: &mut App) -> Self {
        app.renderer
            .set_environment(Environment::Color(UiColor::Rgb(0, 0, 0)));
        let text = UiNode::label(TEXT, Some(600.0), Some(60.0))
            .set_text_color(UiColor::WHITE)
            .set_align(Align::Center)
            .set_font_size(24.0)
            .set_position(PositionValue::Center(0.0), PositionValue::Center(0.0));
        if let Err(error) = scene.spawn_node(app, Node::new(NODE).with_property(UiLayer::new(text)))
        {
            eprintln!("{error}");
        }
        Splash { elapsed: 0.0 }
    }
}

impl SceneBehaviour for Splash {
    fn on_spawn(&mut self, _scene: &mut Scene, app: &mut App) {
        app.scene_manager.preload(NEXT_SCENE);
    }

    fn update(&mut self, scene: &mut Scene, app: &mut App, _ctx: &mut FrameContext) {
        self.elapsed += app.time.unscaled_delta_time;

        let alpha = ((DURATION_SECS - self.elapsed) / FADE_OUT_SECS).clamp(0.0, 1.0);
        if let Some(text) = scene
            .get_node_mut(NODE)
            .and_then(|node| node.get_property_mut::<UiLayer>())
            .and_then(|layer| layer.get_mut(""))
        {
            text.set_alpha(alpha);
        }

        if self.elapsed >= DURATION_SECS && app.scene_manager.is_ready(NEXT_SCENE) {
            app.scene_manager.open_scene(NEXT_SCENE);
        }
    }
}
