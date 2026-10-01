use crate::app::App;
use crate::engine::rendering::ui::ui::Ui;
use crate::engine::scene_manager::scene::{FrameContext, Scene, SceneBehaviour};
use crate::engine::ui::color::UiColor;
use crate::engine::ui::ui_node::UiNode;
use crate::engine::ui::ui_transform::{Orientation, PositionValue, SizeValue};
use crate::game::ui::{card, label};

// pub(crate): App::run drives this scene's fade-out itself once a background
// load finishes (see PendingSceneLoad::ready), rather than this file owning that
// transition - see App::run's own comment on why (ordering vs. this scene's own
// still-ticking Scene::update pulse).
pub(crate) const BACKDROP_KEY: &str = "loading_backdrop";
pub(crate) const PANEL_KEY: &str = "loading_panel";
/// How long App::run spends fading this scene's UI back out once a background
/// load finishes, before swapping to the real scene - see PendingSceneLoad::ready.
pub(crate) const FADE_OUT_SECS: f32 = 0.35;

/// Stand-in `Scene` shown while a `SceneManager::create_loaded_scene`-registered
/// scene's background prepare thread is running (see `App::run`'s reset handling) -
/// occupies `active_scene` so the normal render path keeps presenting frames
/// instead of the window sitting frozen for however long the load takes.
pub struct LoadingScreenScene {
    elapsed: f32,
    /// The step last shown under the title (see App::loading_step).
    shown_step: Option<String>,
}

impl LoadingScreenScene {
    pub fn new(app: &mut App) -> Self {
        let screen_width = app.window_manager.size.width as f32;
        let screen_height = app.window_manager.size.height as f32;

        let mut backdrop = UiNode::container()
            .set_size(SizeValue::Percent(100.0), SizeValue::Percent(100.0))
            .set_position(PositionValue::Start(0.0), PositionValue::Start(0.0))
            .set_background_color(UiColor::Rgba(0, 0, 0, 255));
        backdrop.resolve(screen_width, screen_height);
        app.ui.add_to_ui(BACKDROP_KEY.to_owned(), backdrop);

        let mut panel = card()
            .set_position(PositionValue::Center(0.0), PositionValue::Center(0.0))
            .set_orientation(Orientation::Vertical)
            .set_gap(6.0)
            .set_child("label", label(app, "Loading..."))
            // What's loading right now - filled in from App::loading_step.
            .set_child("step", label(app, " ").set_font_size(&mut app.ui.text.font_system, 14.0).set_text_color(UiColor::Rgb(170, 170, 170)));
        panel.resolve(screen_width, screen_height);
        app.ui.add_to_ui(PANEL_KEY.to_owned(), panel);

        // Same ordering trick play::scene's mission-intro card uses: pushed in this
        // order so the panel (pushed second) lands on top of the backdrop.
        app.ui.always_on_top.push(BACKDROP_KEY.to_owned());
        app.ui.always_on_top.push(PANEL_KEY.to_owned());
        app.ui.has_changed = true;

        Self { elapsed: 0.0, shown_step: None }
    }
}

impl SceneBehaviour for LoadingScreenScene {
    fn update(&mut self, _scene: &mut Scene, app: &mut App, _ctx: &mut FrameContext) {
        self.elapsed += app.time.delta_time;

        // Indeterminate progress (no real byte/step counter is wired up) - a slow
        // pulse is enough to read as "still working" rather than frozen. Keeps
        // running unconditionally, even once the background load has actually
        // finished and App::run has started fading this same label back out
        // (PendingSceneLoad::ready) - that fade-out code runs later in the same
        // frame and overwrites whatever alpha this sets, so the pulse just has no
        // visible effect during those last few frames rather than needing to be
        // explicitly paused.
        let pulse = 0.55 + 0.45 * (self.elapsed * 3.0).sin();

        // What the background load is on now (see LoadReporter) - only
        // touched when it changes.
        if app.loading_step != self.shown_step {
            self.shown_step = app.loading_step.clone();
            let text = self.shown_step.clone().unwrap_or_else(|| " ".to_owned());
            if let Some(step) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &format!("{PANEL_KEY}/step")).and_then(|node| node.as_label_mut()) {
                step.set_text(&mut app.ui.text.font_system, &text, true);
            }
        }
        if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &format!("{PANEL_KEY}/label")) {
            node.set_alpha(pulse);
        }
        app.ui.has_changed = true;
    }
}
