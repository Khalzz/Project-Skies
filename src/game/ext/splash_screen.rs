//! The splash shown at start - "A Pankarta Software Production" on black,
//! fading and sliding in, then out - while the main menu loads behind it.
//!
//! The engine has no built-in splash any more: it's an ordinary scene that
//! preloads the next one (see the engine's docs/scene_loading.md).

use glyphon::cosmic_text::Align;
use pankarta_software::prelude::*;
use pankarta_software::ui::ui_node::UiNode;
use pankarta_software::utils::lerps::smoothstep;

/// The scene opened when the splash is done.
const NEXT_SCENE: &str = "main_menu";
const TEXT: &str = "A Pankarta Software Production";
/// How long it shows, at the least - longer if the menu isn't loaded yet.
const DURATION_SECS: f32 = 3.0;
/// Entrance/exit fade length.
const FADE_SECS: f32 = 0.5;
/// How far below its resting spot the text starts, sliding up as it fades in.
const SLIDE_PX: f32 = 24.0;
const TEXT_SIZE: (f32, f32) = (600.0, 60.0);
/// The splash's node - its UI layer.
const NODE: &str = "splash";
const TEXT_PATH: &str = "text";

/// Combined fade-in * fade-out alpha, plus how "arrived" it is (0 at the very
/// start of its entrance, 1 once fully faded in) - for the slide.
fn animate(elapsed_secs: f32, fade_secs: f32, remaining_secs: f32) -> (f32, f32) {
    let entrance = smoothstep(elapsed_secs / fade_secs.max(0.001));
    let exit = smoothstep(remaining_secs / fade_secs.max(0.001));
    (entrance * exit, entrance)
}

pub struct Splash {
    elapsed: f32,
    /// Where the text rests once it's in.
    text_base: (f32, f32),
}

impl Splash {
    pub fn new(scene: &mut Scene, app: &mut App) -> Self {
        app.renderer.set_environment(Environment::Color(UiColor::Rgb(0, 0, 0)));
        // Camera-less on purpose - nothing 3D; the engine gives it its own.
        let (width, height) = app.ui.size();
        let text_base = ((width - TEXT_SIZE.0) / 2.0, (height - TEXT_SIZE.1) / 2.0);
        let mut text = UiNode::label(TEXT, Some(TEXT_SIZE.0), Some(TEXT_SIZE.1))
            .at(text_base.0, text_base.1 + SLIDE_PX)
            .set_text_color(UiColor::WHITE)
            .set_align(Align::Center)
            .set_font_size(24.0);
        text.set_alpha(0.0);
        let root = UiNode::container().set_child(TEXT_PATH, text);
        if let Err(error) = scene.spawn_node(app, Node::new(NODE).with_property(UiLayer::new(root))) {
            eprintln!("{error}");
        }
        Splash { elapsed: 0.0, text_base }
    }
}

impl SceneBehaviour for Splash {
    fn on_spawn(&mut self, _scene: &mut Scene, app: &mut App) {
        // The menu loads while the splash shows.
        app.scene_manager.preload(NEXT_SCENE);
    }

    fn update(&mut self, scene: &mut Scene, app: &mut App, _ctx: &mut FrameContext) {
        self.elapsed += app.time.unscaled_delta_time;
        let ready = app.scene_manager.is_ready(NEXT_SCENE);
        // Holds at full opacity past its time while the menu still loads.
        let remaining = if ready { DURATION_SECS - self.elapsed } else { DURATION_SECS.max(self.elapsed + FADE_SECS) - self.elapsed };
        let (alpha, entrance) = animate(self.elapsed, FADE_SECS, remaining.max(0.0));
        let (x, y) = self.text_base;
        if let Some(text) = scene.get_node_mut(NODE).and_then(|node| node.get_property_mut::<UiLayer>()).and_then(|layer| layer.get_mut(TEXT_PATH)) {
            text.set_alpha(alpha);
            text.move_to(x, y + (1.0 - entrance) * SLIDE_PX);
        }
        if ready && self.elapsed >= DURATION_SECS {
            app.scene_manager.open_scene(NEXT_SCENE);
        }
    }
}
