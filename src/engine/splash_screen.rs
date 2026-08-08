use std::time::{Duration, Instant};

use glyphon::{cosmic_text::Align, Color};
use tokio::task;

use crate::app::App;
use crate::engine::input::input;
use crate::engine::ui::ui_node::UiNode;
use crate::engine::utils::lerps::smoothstep;

/// Configuration for App's optional startup splash screen - see `App::splash_screen`.
/// Entirely opt-in: `App::new()` leaves it as `None`, so nothing shows unless a
/// caller assigns one (e.g. `app.splash_screen = Some(SplashScreenConfig::new(...))`)
/// before calling `App::run()` - the same pattern already used for `app.scene_manager`.
pub struct SplashScreenConfig {
    pub background_color: wgpu::Color,
    pub image_path: Option<String>,
    pub text: Option<String>,
    pub duration: Duration,
}

impl SplashScreenConfig {
    pub fn new(background_color: wgpu::Color, duration: Duration) -> Self {
        Self { background_color, image_path: None, text: None, duration }
    }

    pub fn with_image(mut self, path: impl Into<String>) -> Self {
        self.image_path = Some(path.into());
        self
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.text = Some(text.into());
        self
    }
}

const SPLASH_IMAGE_KEY: &str = "__splash_image";
const SPLASH_TEXT_KEY: &str = "__splash_text";

// Entrance/exit fade length - capped to half the splash duration in run_splash_screen
// so a very short splash never dips below full opacity or fails to reach it.
const FADE_MS: u64 = 500;
// How far below its final resting spot each element starts, sliding up as it fades in.
const SLIDE_PX: f32 = 24.0;
// The text starts fading in a moment after the icon, rather than both popping in
// at once - purely cosmetic staggering.
const TEXT_STAGGER_MS: u64 = 150;

// Combined fade-in * fade-out alpha for an element, plus how "arrived" it is (0 at
// the very start of its entrance, 1 once fully faded in) so callers can also drive
// a slide-in from that same progress. `elapsed`/`remaining` are seconds into/left in
// the splash; `delay` staggers this element's entrance start relative to others.
fn animate(elapsed_secs: f32, delay_secs: f32, fade_secs: f32, remaining_secs: f32) -> (f32, f32) {
    let entrance = smoothstep((elapsed_secs - delay_secs) / fade_secs.max(0.001));
    let exit = smoothstep(remaining_secs / fade_secs.max(0.001));
    (entrance * exit, entrance)
}

impl App {
    // Shows the configured splash screen (if any) and blocks until its duration
    // elapses, then returns - a no-op if splash_screen is None. Called once from
    // App::run() before its main loop starts, so self.scene_manager (already
    // configured by the caller, same as always) takes over exactly as it would have
    // without this ever running - the splash phase doesn't know or care which scene
    // is "first", it just delays entering the normal loop for a bit.
    pub(crate) fn run_splash_screen(&mut self, event_pump: &mut sdl2::EventPump) {
        let Some(config) = self.splash_screen.take() else {
            return;
        };

        self.clear_color = config.background_color;
        self.skybox = None;

        // Resting (post-animation) position of each element - captured so the per-frame
        // animation below has a fixed target to slide/fade towards instead of having
        // to recompute layout every frame.
        let mut image_base: Option<(f32, f32)> = None;
        let mut text_base: Option<(f32, f32)> = None;

        if let Some(path) = &config.image_path {
            let result = task::block_in_place(|| {
                tokio::runtime::Runtime::new().unwrap()
                    .block_on(self.ui.load_image(path, &self.renderer.device, &self.renderer.queue))
            });

            match result {
                Ok(()) => {
                    if let Some(image) = self.ui.images.get(path) {
                        let size = image.texture.texture.size();
                        let (width, height) = (size.width as f32, size.height as f32);
                        let x = (self.window_manager.size.width as f32 - width) / 2.0;
                        let y = (self.window_manager.size.height as f32 - height) / 2.0;
                        self.ui.add_to_ui(SPLASH_IMAGE_KEY.to_owned(), UiNode::image(path.clone(), width, height).at(x, y));
                        image_base = Some((x, y));
                    }
                }
                Err(e) => eprintln!("Failed to load splash screen image '{}': {}", path, e),
            }
        }

        if let Some(text) = &config.text {
            let (width, height) = (600.0, 60.0);
            let x = (self.window_manager.size.width as f32 - width) / 2.0;
            // Below the image if there's one showing, otherwise dead center.
            let y = if image_base.is_some() {
                self.window_manager.size.height as f32 / 2.0 + 80.0
            } else {
                (self.window_manager.size.height as f32 - height) / 2.0
            };

            let node = UiNode::label(&mut self.ui.text.font_system, text, Some(width), Some(height))
                .at(x, y)
                .set_text_color(Color::rgba(255, 255, 255, 255))
                .set_align(Align::Center)
                .set_font_size(24.0);
            self.ui.add_to_ui(SPLASH_TEXT_KEY.to_owned(), node);
            text_base = Some((x, y));
        }

        // Fade length can't outlast the splash itself - halved so entrance and exit
        // both fit without overlapping into a dip below full opacity.
        let fade_secs = (FADE_MS as f32 / 1000.0).min(config.duration.as_secs_f32() / 2.0);
        let text_delay_secs = if image_base.is_some() { TEXT_STAGGER_MS as f32 / 1000.0 } else { 0.0 };

        self.ui.has_changed = true;

        let start = Instant::now();
        loop {
            self.time.update();
            input::update(event_pump, self.time.delta_time, false);

            let elapsed = start.elapsed();
            if elapsed >= config.duration {
                break;
            }
            let elapsed_secs = elapsed.as_secs_f32();
            let remaining_secs = (config.duration - elapsed).as_secs_f32();

            if let (Some((x, y)), Some(node)) = (image_base, self.ui.renderizable_elements.get_mut(SPLASH_IMAGE_KEY)) {
                let (alpha, entrance) = animate(elapsed_secs, 0.0, fade_secs, remaining_secs);
                node.set_alpha(alpha);
                node.move_to(x, y + (1.0 - entrance) * SLIDE_PX);
            }
            if let (Some((x, y)), Some(node)) = (text_base, self.ui.renderizable_elements.get_mut(SPLASH_TEXT_KEY)) {
                let (alpha, entrance) = animate(elapsed_secs, text_delay_secs, fade_secs, remaining_secs);
                node.set_alpha(alpha);
                node.move_to(x, y + (1.0 - entrance) * SLIDE_PX);
            }
            self.ui.has_changed = true;

            match self.render() {
                Ok(_) => {},
                Err(wgpu::SurfaceError::Outdated) => self.resize(),
                Err(wgpu::SurfaceError::Lost) => {
                    eprintln!("Device lost during splash screen!");
                    break;
                }
                Err(e) => eprintln!("Error: {}", e),
            }
        }

        // Splash-only nodes shouldn't linger into the first real scene.
        self.ui.renderizable_elements.remove(SPLASH_IMAGE_KEY);
        self.ui.renderizable_elements.remove(SPLASH_TEXT_KEY);
        self.ui.has_changed = true;
    }
}
