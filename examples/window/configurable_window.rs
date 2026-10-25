//! A window set up from an on-screen settings panel: its mode, picked from a
//! dropdown; the resolution, stepped through the screen's own with [<] and
//! [>]; vsync, a switch; and the graphics backend, from the ones this machine
//! can use. Changes wait until Apply is clicked - it lights up only while
//! something differs from what the window is using. The backend can only
//! change when the app starts, so Apply saves it (to BACKEND_FILE) for the
//! next launch. The frame rate is shown under them, so turning vsync off
//! shows its effect.
//!
//! cargo run --example configurable_window

use std::cell::RefCell;
use std::rc::Rc;

use pankarta_software::app::App;
use pankarta_software::rendering::graphics_backend::GraphicsBackend;
use pankarta_software::rendering::ui::ui::Ui;
use pankarta_software::scene_manager::scene::{FrameContext, Scene, SceneBehaviour};
use pankarta_software::ui::color::UiColor;
use pankarta_software::ui::layer::Layer;
use pankarta_software::ui::ui_node::UiNode;
use pankarta_software::ui::ui_transform::{Anchor, Orientation, PositionValue, SizeValue};
use pankarta_software::ui::widgets::{button, card, dropdown, text, toggle, ButtonStyle, CardStyle, DropdownStyle, ToggleStyle};
use pankarta_software::ui::{Align, TextStyle, UiScaling, Weight};
use pankarta_software::window::window::WindowMode;

// Where the values are in the UI tree - each setting row holds its control
// under "control" (see `setting_row`).
const RESOLUTION_VALUE: &str = "settings/resolution/control/value";
const RESOLUTION_BACK: &str = "settings/resolution/control/back";
const RESOLUTION_AHEAD: &str = "settings/resolution/control/ahead";
const FPS_VALUE: &str = "settings/fps/value";
const APPLY_BUTTON: &str = "settings/apply";
const NOTE: &str = "settings/note";

/// Where the chosen backend is kept between launches.
const BACKEND_FILE: &str = "settings/configurable_window_backend.txt";

/// How wide the names on the left are, so the controls line up.
const NAME_WIDTH: f32 = 110.0;
/// How wide the controls are - the resolution's [<] value [>] adds up to it.
const CONTROL_WIDTH: f32 = 188.0;

/// The window modes, in the dropdown's order.
const WINDOW_MODES: [(&str, WindowMode); 3] = [
    ("Windowed", WindowMode::Windowed),
    ("Borderless", WindowMode::BorderlessFullscreen),
    ("Fullscreen", WindowMode::Fullscreen),
];

/// One set of window settings - indexes into WINDOW_MODES, the screen's
/// resolutions and the usable backends.
#[derive(Clone, Copy, PartialEq)]
struct WindowSettings {
    mode: usize,
    resolution: usize,
    vsync: bool,
    backend: usize,
}

/// What the window is using, and what the panel shows - they differ until
/// Apply is clicked.
struct Settings {
    applied: WindowSettings,
    pending: WindowSettings,
    /// The screen's resolutions, biggest first (see
    /// Window::available_resolutions).
    resolutions: Vec<(u32, u32)>,
    /// The backends this machine can use, Auto first (see
    /// GraphicsBackend::available).
    backends: Vec<GraphicsBackend>,
    /// The backend asked for when the app started - a different one applied
    /// waits for the next start.
    launched_with: GraphicsBackend,
}

impl Settings {
    fn changed(&self) -> bool {
        self.pending != self.applied
    }
}

type SharedSettings = Rc<RefCell<Settings>>;

fn body() -> TextStyle {
    TextStyle::default().size(15.0)
}

/// One setting: its name on the left, its control after it.
fn setting_row(app: &mut App, name: &str, control: UiNode) -> UiNode {
    UiNode::container()
        .set_orientation(Orientation::Horizontal)
        .set_child_anchor(Anchor::Start, Anchor::Center)
        .set_gap(8.0)
        .set_child("name", text(app, name, &body()).set_size(SizeValue::Pixels(NAME_WIDTH), SizeValue::Fit))
        .set_child("control", control)
}

struct ConfigurableWindow {
    settings: SharedSettings,
}

impl ConfigurableWindow {
    /// `launched_with`: the backend the app was started with; `backends`: the
    /// ones this machine can use (see `main`).
    fn new(app: &mut App, launched_with: GraphicsBackend, backends: Vec<GraphicsBackend>) -> Self {
        // Only the screen's 16:9 resolutions - all of them if it has none.
        let all = app.window.available_resolutions();
        let wide: Vec<(u32, u32)> = all.iter().copied().filter(|&size| is_16_9(size)).collect();
        let resolutions = if wide.is_empty() { all } else { wide };
        let current = WindowSettings {
            mode: WINDOW_MODES.iter().position(|(_, mode)| *mode == app.window.window_mode()).unwrap_or(0),
            resolution: closest_resolution(&resolutions, (app.window.size.width, app.window.size.height)),
            vsync: app.renderer.vsync(),
            backend: backends.iter().position(|backend| *backend == launched_with).unwrap_or(0),
        };
        let backend_names: Vec<&str> = backends.iter().map(|backend| backend.name()).collect();
        let settings: SharedSettings = Rc::new(RefCell::new(Settings { applied: current, pending: current, resolutions, backends, launched_with }));

        // Each control only changes the pending settings.
        let names: Vec<&str> = WINDOW_MODES.iter().map(|(name, _)| *name).collect();
        let mode = {
            let settings = Rc::clone(&settings);
            dropdown(app, &names, current.mode, &DropdownStyle::default().width(CONTROL_WIDTH))
                .on_select(move |_, index| settings.borrow_mut().pending.mode = index)
        };

        let arrow = ButtonStyle::default().width(28.0);
        let (smaller, bigger) = (Rc::clone(&settings), Rc::clone(&settings));
        let resolution = UiNode::container()
            .set_orientation(Orientation::Horizontal)
            .set_child_anchor(Anchor::Start, Anchor::Center)
            .set_gap(6.0)
            // The list is biggest first - [<] goes down it, [>] up. Each is
            // disabled at its end of the list (see `update`).
            .set_child("back", button(app, "<", &arrow).on_click(move |_| step_resolution(&smaller, 1)))
            .set_child("value", text(app, "-", &body().align(Align::Center)).set_size(SizeValue::Pixels(120.0), SizeValue::Fit))
            .set_child("ahead", button(app, ">", &arrow).on_click(move |_| step_resolution(&bigger, -1)));

        let vsync = {
            let settings = Rc::clone(&settings);
            toggle(current.vsync, &ToggleStyle::default())
                .on_toggle(move |_, on| settings.borrow_mut().pending.vsync = on)
        };

        let backend = {
            let settings = Rc::clone(&settings);
            dropdown(app, &backend_names, current.backend, &DropdownStyle::default().width(CONTROL_WIDTH))
                .on_select(move |_, index| settings.borrow_mut().pending.backend = index)
        };

        let fps = text(app, "-", &body()).set_size(SizeValue::Pixels(80.0), SizeValue::Fit);
        // Which backend is drawing, or which one waits for the next start.
        let note = text(app, "-", &body().size(13.0).color(UiColor::Rgb(170, 170, 170)))
            .set_size(SizeValue::Pixels(NAME_WIDTH + 8.0 + CONTROL_WIDTH), SizeValue::Fit);

        // Disabled while there's nothing to apply (see `update`).
        let apply = {
            let settings = Rc::clone(&settings);
            button(app, "Apply", &ButtonStyle::default().width(NAME_WIDTH + 8.0 + CONTROL_WIDTH))
                .on_click(move |app| apply(app, &settings))
                .enabled(false)
        };

        let heading = TextStyle::default().size(20.0).weight(Weight::BOLD);
        let settings_panel = card(&CardStyle::default().gap(12.0).padding(20.0))
            .set_position(PositionValue::Center(0.0), PositionValue::Center(0.0))
            .set_child("title", text(app, "Window", &heading))
            .set_child("mode", setting_row(app, "Mode", mode))
            .set_child("resolution", setting_row(app, "Resolution", resolution))
            .set_child("vsync", setting_row(app, "VSync", vsync))
            .set_child("backend", setting_row(app, "Backend", backend))
            .set_child("fps", UiNode::container()
                .set_orientation(Orientation::Horizontal)
                .set_child_anchor(Anchor::Start, Anchor::Center)
                .set_gap(8.0)
                .set_child("name", text(app, "FPS", &body().color(UiColor::Rgb(170, 170, 170))).set_size(SizeValue::Pixels(NAME_WIDTH), SizeValue::Fit))
                .set_child("value", fps))
            .set_child("note", note)
            .set_child("apply", apply);

        Layer::new(app).set_child("settings", settings_panel).build(app);
        ConfigurableWindow { settings }
    }
}

impl SceneBehaviour for ConfigurableWindow {
    // What's shown, every frame: the frame rate, the pending resolution,
    // which arrows can still move, and whether Apply has anything to do.
    fn update(&mut self, _scene: &mut Scene, app: &mut App, _ctx: &mut FrameContext) {
        let settings = self.settings.borrow();
        let fps = format!("{:.0}", app.time.fps());
        let resolution = match settings.resolutions.get(settings.pending.resolution) {
            Some((width, height)) => format!("{width} x {height}"),
            None => format!("{} x {}", app.window.size.width, app.window.size.height),
        };
        let chosen = settings.backends.get(settings.applied.backend).copied().unwrap_or_default();
        let note = if chosen != settings.launched_with {
            format!("{chosen} will be used the next time it starts")
        } else {
            format!("Drawing with {}", app.renderer.backend())
        };

        let font_system = &mut app.ui.text.font_system;
        let elements = &mut app.ui.renderizable_elements;
        if let Some(label) = Ui::get_ui_node(elements, FPS_VALUE).and_then(|node| node.as_label_mut()) {
            label.set_text(font_system, &fps, false);
        }
        if let Some(label) = Ui::get_ui_node(elements, RESOLUTION_VALUE).and_then(|node| node.as_label_mut()) {
            label.set_text(font_system, &resolution, true);
        }
        if let Some(label) = Ui::get_ui_node(elements, NOTE).and_then(|node| node.as_label_mut()) {
            label.set_text(font_system, &note, false);
        }
        if let Some(apply) = Ui::get_ui_node(elements, APPLY_BUTTON) {
            apply.set_enabled(settings.changed());
        }
        // The list is biggest first: [<] stops at its last (the smallest),
        // [>] at its first (the biggest).
        let last = settings.resolutions.len().saturating_sub(1);
        if let Some(back) = Ui::get_ui_node(elements, RESOLUTION_BACK) {
            back.set_enabled(settings.pending.resolution < last);
        }
        if let Some(ahead) = Ui::get_ui_node(elements, RESOLUTION_AHEAD) {
            ahead.set_enabled(settings.pending.resolution > 0);
        }
    }
}

/// Applies whatever pending setting differs from the window's - the size
/// before the mode, since Fullscreen takes the screen to the size set.
fn apply(app: &mut App, settings: &SharedSettings) {
    let mut settings = settings.borrow_mut();
    if !settings.changed() {
        return;
    }
    let (pending, applied) = (settings.pending, settings.applied);

    if pending.resolution != applied.resolution {
        if let Some(&(width, height)) = settings.resolutions.get(pending.resolution) {
            if let Err(err) = app.window.set_size(width, height) {
                eprintln!("Couldn't change the size: {err}");
            }
        }
    }
    if pending.mode != applied.mode {
        if let Err(err) = app.window.set_window_mode(WINDOW_MODES[pending.mode].1) {
            eprintln!("Couldn't change the window mode: {err}");
        }
    }
    if pending.vsync != applied.vsync {
        app.renderer.set_vsync(pending.vsync);
    }
    // Only a new app can draw with another backend - kept for the next start.
    if pending.backend != applied.backend {
        if let Some(backend) = settings.backends.get(pending.backend) {
            if let Err(err) = std::fs::write(BACKEND_FILE, backend.key()) {
                eprintln!("Couldn't save the backend: {err}");
            }
        }
    }
    settings.applied = pending;
}

/// The backend saved by a previous run (see `apply`) - Auto if there's none.
fn saved_backend() -> GraphicsBackend {
    std::fs::read_to_string(BACKEND_FILE).ok()
        .and_then(|key| GraphicsBackend::from_key(&key))
        .unwrap_or_default()
}

/// Whether `width` x `height` is 16:9 - within 1%, so the odd ones sold as
/// 16:9 (1366x768, 1360x768) count too.
fn is_16_9((width, height): (u32, u32)) -> bool {
    height > 0 && ((width as f32 / height as f32) - 16.0 / 9.0).abs() < 16.0 / 9.0 * 0.01
}

/// The index in `resolutions` nearest `size` (by area) - the window may not
/// be at one of the screen's own resolutions.
fn closest_resolution(resolutions: &[(u32, u32)], size: (u32, u32)) -> usize {
    let area = |(w, h): (u32, u32)| (w as i64) * (h as i64);
    resolutions.iter()
        .enumerate()
        .min_by_key(|(_, r)| (area(**r) - area(size)).abs())
        .map_or(0, |(index, _)| index)
}

/// Moves the pending resolution `step` along the screen's, stopping at either
/// end (its arrow is disabled there anyway).
fn step_resolution(settings: &SharedSettings, step: isize) {
    let mut settings = settings.borrow_mut();
    let count = settings.resolutions.len() as isize;
    if count == 0 {
        return;
    }
    settings.pending.resolution = (settings.pending.resolution as isize + step).clamp(0, count - 1) as usize;
}

#[tokio::main]
async fn main() {
    // The backend chosen last time - back to Auto if this machine can't
    // use it any more.
    let mut launched_with = saved_backend();
    let app = match App::new_with_backend(launched_with).await {
        Ok(app) => Ok(app),
        Err(err) if launched_with != GraphicsBackend::Auto => {
            eprintln!("Couldn't start on {launched_with} ({err}) - trying Auto");
            launched_with = GraphicsBackend::Auto;
            App::new().await
        }
        Err(err) => Err(err),
    };
    let mut app = match app {
        Ok(app) => app,
        Err(err) => {
            eprintln!("Couldn't create the app: {err}");
            return;
        }
    };

    // What this machine can draw with (worked out as the app started).
    let backends = GraphicsBackend::available();

    app.window.set_title("Configurable window");

    app.ui.scaling = UiScaling::WithWindow { reference_height: 1080.0 };
    app.ui.scale = 1.0;

    app.window.set_resizable(false);
    if let Err(err) = app.window.set_size(1280, 720) {
        eprintln!("Couldn't set the size: {err}");
    }

    app.scene_manager.create_scene("main", move |_scene, app| ConfigurableWindow::new(app, launched_with, backends.clone()));
    app.scene_manager.open_scene("main");

    app.run();
}
