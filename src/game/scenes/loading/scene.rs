//! The loading screen: "Loading..." in a card on black, pulsing, with how
//! far the level has got under it - while the level builds and loads in the
//! background. An ordinary scene, opened with `go_to` each time (the engine
//! has no loading screen of its own - see its docs/scene_loading.md).

use glyphon::cosmic_text::Align;
use pankarta_software::prelude::*;
use pankarta_software::ui::ui_node::UiNode;
use pankarta_software::ui::ui_transform::{Anchor, Orientation, PositionValue, SizeValue};
use pankarta_software::utils::lerps::smoothstep;

use crate::game::ui::{card, label};

/// The loading screen's node - its UI layer.
const NODE: &str = "loading";
const PANEL_LABEL: &str = "panel/label";
const STEP: &str = "step";
/// How long the screen fades out once the level is ready, before the level
/// opens.
const FADE_OUT_SECS: f32 = 0.35;
/// The progress line's width and how far above the screen's bottom edge it
/// sits.
const STEP_WIDTH: f32 = 420.0;
const STEP_BOTTOM_MARGIN: f32 = 48.0;

/// Opens the loading screen, which loads `scene` (a registered scene's name)
/// and opens it once it's ready.
pub fn go_to(app: &mut App, scene: &str) {
    let target = scene.to_owned();
    // Built again from scratch - a restart doesn't reuse the old level.
    app.scene_manager.cancel_preload(&target);
    app.scene_manager.open(move |scene, app| LoadingScreenScene::new(scene, app, target.clone()));
}

pub struct LoadingScreenScene {
    target: String,
    elapsed: f32,
    /// Seconds into the fade-out, once the target is ready.
    fading: Option<f32>,
    /// The progress last shown.
    shown_percent: Option<u32>,
}

impl LoadingScreenScene {
    pub fn new(scene: &mut Scene, app: &mut App, target: String) -> Self {
        app.renderer.set_environment(Environment::Color(UiColor::Rgb(0, 0, 0)));

        let panel = card()
            .set_position(PositionValue::Center(0.0), PositionValue::Center(0.0))
            .set_orientation(Orientation::Vertical)
            .set_child_anchor(Anchor::Center, Anchor::Start)
            .set_child("label", label("Loading..."));
        // How far the level has got - at the bottom of the screen. A fixed
        // width, so longer text later still fits.
        let step = label(" ")
            .set_size(SizeValue::Pixels(STEP_WIDTH), SizeValue::Fit)
            .set_position(PositionValue::Center(0.0), PositionValue::End(-STEP_BOTTOM_MARGIN))
            .set_align(Align::Center)
            .set_font_size(14.0)
            .set_text_color(UiColor::Rgb(170, 170, 170));
        let root = UiNode::container()
            .set_size(SizeValue::Percent(100.0), SizeValue::Percent(100.0))
            .set_background_color(UiColor::Rgba(0, 0, 0, 255))
            .set_child("panel", panel)
            .set_child(STEP, step);
        if let Err(error) = scene.spawn_node(app, Node::new(NODE).with_property(UiLayer::new(root))) {
            eprintln!("{error}");
        }

        Self { target, elapsed: 0.0, fading: None, shown_percent: None }
    }
}

impl SceneBehaviour for LoadingScreenScene {
    fn on_spawn(&mut self, _scene: &mut Scene, app: &mut App) {
        app.scene_manager.preload(&self.target);
    }

    fn update(&mut self, scene: &mut Scene, app: &mut App, _ctx: &mut FrameContext) {
        let delta_time = app.time.unscaled_delta_time;
        self.elapsed += delta_time;

        // Indeterminate pulse - reads as "still working" rather than frozen.
        let pulse = 0.55 + 0.45 * (self.elapsed * 3.0).sin();
        let percent = (app.scene_manager.load_progress(&self.target) * 100.0).round() as u32;

        if self.fading.is_none() && app.scene_manager.is_ready(&self.target) {
            self.fading = Some(0.0);
        }
        let alpha = match &mut self.fading {
            Some(seconds) => {
                *seconds += delta_time;
                1.0 - smoothstep(*seconds / FADE_OUT_SECS)
            }
            None => 1.0,
        };

        let Some(layer) = scene.get_node_mut(NODE).and_then(|node| node.get_property_mut::<UiLayer>()) else { return };
        if self.shown_percent != Some(percent) {
            self.shown_percent = Some(percent);
            if let Some(step) = layer.label_mut(STEP) {
                step.set_text(format!("{percent}%"));
            }
        }
        if let Some(label) = layer.get_mut(PANEL_LABEL) {
            label.set_alpha(pulse * alpha);
        }
        if let Some(step) = layer.get_mut(STEP) {
            step.set_alpha(alpha);
        }

        if self.fading.is_some_and(|seconds| seconds >= FADE_OUT_SECS) {
            app.scene_manager.open_scene(&self.target);
        }
    }
}
