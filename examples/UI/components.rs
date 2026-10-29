//! The engine's UI widgets - text, card, button, text input, scrollable, modal and toast - with
//! their base look and a few styles made from it. Just how they look: the
//! modal and the toasts can be opened to see them, but nothing here does
//! anything - see `ui_greetings` for widgets put to work.
//!
//! cargo run --example ui_components

use pankarta_software::app::App;
use pankarta_software::rendering::enviroment::environment::Environment;
use pankarta_software::rendering::ui::ui::Ui;
use pankarta_software::scene_manager::scene::{Scene, SceneBehaviour};
use pankarta_software::ui::color::UiColor;
use pankarta_software::ui::layer::Layer;
use pankarta_software::ui::ui_node::{Style, UiNode};
use pankarta_software::ui::ui_transform::{Anchor, BorderEdges, Orientation, Padding, PositionValue, SizeValue};
use pankarta_software::ui::widgets::{
    button, card, close_modal, dropdown, modal, open_modal, scrollable, show_toast, text, text_input, toggle, ButtonStyle, CardStyle,
    DropdownStyle, ModalStyle, ScrollableStyle, TextInputStyle, ToastStyle, ToggleStyle,
};
use pankarta_software::ui::{Align, TextStyle, UiScaling, Weight};

/// Where the line at the bottom saying what was clicked is - see `set_status`.
const STATUS: &str = "root/status";
const CONFIRM_MODAL: &str = "confirm";

// ── Text styles, defined once and used everywhere below ──

fn body() -> TextStyle {
    TextStyle::default().size(15.0)
}

fn heading() -> TextStyle {
    body().size(20.0).weight(Weight::BOLD)
}

fn muted() -> TextStyle {
    body().color(UiColor::Rgb(170, 170, 170))
}

// ── Button styles made from the engine's base one ──

/// No box at all - a white bar on its left lights up on hover. The hover look
/// is replaced, not changed, so the base's border color doesn't carry over.
fn ghost() -> ButtonStyle {
    ButtonStyle {
        background: UiColor::TRANSPARENT.into(),
        border_color: UiColor::TRANSPARENT.into(),
        border_width: 3.0,
        corner_radius: 0.0,
        padding: Padding::from((10.0, 2.0)),
        ..ButtonStyle::default()
    }
    .text(|text| text.align(Align::Left))
    .on_hover(|_| Style::default()
        .set_border_color(UiColor::WHITE)
        .set_border_edges(BorderEdges::LEFT)
        .set_background_color(UiColor::Rgba(0, 0, 0, 200))
        .set_text_color(UiColor::WHITE))
    .on_press(|_| Style::default()
        .set_border_color(UiColor::WHITE)
        .set_border_edges(BorderEdges::LEFT)
        .set_background_color(UiColor::Rgba(50, 50, 50, 200))
        .set_text_color(UiColor::WHITE))
}

/// A row of the scrollable list - full width, text on the left, faint.
fn list_row() -> ButtonStyle {
    ButtonStyle::default()
        .text(|_| body().align(Align::Left))
        .padding((10.0, 5.0))
        .background(UiColor::TRANSPARENT)
        .border(UiColor::TRANSPARENT, 0.0)
        .corner_radius(4.0)
}

// ── The screen ──

struct UiShowcase;

impl UiShowcase {
    fn new(_scene: &mut Scene, app: &mut App) -> Self {
        // No camera set up here - the engine gives the scene its default one
        // (at the origin, unrotated). Behind the UI, the procedural sky - a
        // computed clear-day gradient, no images - without its sun.
        app.renderer.set_environment(Environment::Color(UiColor::Rgb(0, 89, 149)));

        // A card of buttons - the base style first, then styles made from it.
        let buttons = card(&CardStyle::default())
            .set_child("title", text(app, "Buttons", &heading()))
            .set_child("default", button(app, "Default", &ButtonStyle::default())
                .on_click(|app| set_status(app, "Default clicked")))
            .set_child("ghost", button(app, "Ghost", &ghost())
                .on_click(|app| set_status(app, "Ghost clicked")));

        // Inputs - their look; they work, but nothing reads them here.
        let inputs = card(&CardStyle::default())
            .set_child("title", text(app, "Inputs", &heading()))
            .set_child("name", text_input(app, "Your name", &TextInputStyle::default().width(200.0)))
            .set_child("quality", dropdown(app, &["Low", "Medium", "High", "Ultra"], 1, &DropdownStyle::default()))
            .set_child("switch", toggle(true, &ToggleStyle::default()));

        // The modal and the toasts, opened to see how they look.
        let overlays = card(&CardStyle::default())
            .set_child("title", text(app, "Overlays", &heading()))
            .set_child("open_modal", button(app, "Open modal...", &ButtonStyle::default().width(200.0))
                .on_click(open_confirm_modal))
            .set_child("toast", button(app, "Info toast", &ButtonStyle::default().width(200.0))
                .on_click(|app| show_toast(app, "Game saved", &ToastStyle::info())))
            .set_child("alert", button(app, "Alert toast", &ButtonStyle::default().width(200.0))
                .on_click(|app| show_toast(app, "Low fuel", &ToastStyle::alert())))
            .set_child("error", button(app, "Error toast", &ButtonStyle::default().width(200.0))
                .on_click(|app| show_toast(app, "Couldn't save the game", &ToastStyle::error())));

        // A scrollable list of 30 rows.
        let list_style = ScrollableStyle::default();
        let mut list = scrollable(&list_style);
        for i in 1..=30 {
            let name = format!("Item {i}");
            // Grow: as wide as the list's content area. Fit: as tall as its text.
            list = list.set_child(format!("row{i}"), button(app, &name, &list_row())
                .set_size(SizeValue::Grow, SizeValue::Fit)
                .on_click(move |app| set_status(app, &format!("{name} clicked"))));
        }
        let list_card = card(&CardStyle::default())
            .set_child("title", text(app, "Scrollable", &heading()))
            .set_child("list", list);

        let row = UiNode::container()
            .set_orientation(Orientation::Horizontal)
            .set_child_anchor(Anchor::Start, Anchor::Start)
            .set_gap(24.0)
            .set_child("buttons", buttons)
            .set_child("inputs", inputs)
            .set_child("overlays", overlays)
            .set_child("list", list_card);

        // A fixed width, so longer messages later still fit.
        let status = text(app, "Click something", &muted().align(Align::Center))
            .set_size(SizeValue::Pixels(600.0), SizeValue::Fit)
            .set_text_color(UiColor::Rgb(255, 255, 255));

        let root = UiNode::container()
            .set_position(PositionValue::Center(0.0), PositionValue::Center(0.0))
            .set_child_anchor(Anchor::Center, Anchor::Start)
            .set_gap(24.0)
            .set_child("row", row)
            .set_child("status", status);

        Layer::new(app).set_child("root", root).build(app);
        UiShowcase
    }
}

impl SceneBehaviour for UiShowcase {}

/// A modal asking to confirm - Cancel and Delete both close it.
fn open_confirm_modal(app: &mut App) {
    let content = UiNode::container()
        .set_gap(14.0)
        .set_child("title", text(app, "Delete save file?", &heading().size(22.0)))
        .set_child("message", text(app, "This can't be undone.", &muted()))
        .set_child("actions", UiNode::container()
            .set_orientation(Orientation::Horizontal)
            .set_gap(10.0)
            .set_child("cancel", button(app, "Cancel", &ButtonStyle::default())
                .on_click(|app| {
                    close_modal(app, CONFIRM_MODAL);
                    set_status(app, "Cancelled");
                })));

    open_modal(app, CONFIRM_MODAL, modal(content, &ModalStyle::default()));
}

/// Shows `text` on the line at the bottom.
fn set_status(app: &mut App, text: &str) {
    if let Some(label) = Ui::get_ui_node(&mut app.ui.renderizable_elements, STATUS).and_then(|node| node.as_label_mut()) {
        label.set_text(&mut app.ui.text.font_system, text, true);
    }
}

#[tokio::main]
async fn main() {
    let mut app = match App::new().await {
        Ok(app) => app,
        Err(err) => {
            eprintln!("Couldn't create the app: {err}");
            return;
        }
    };

    app.window.set_title("UI components");
    // Designed for this 1280x720 window - a bigger or smaller one (or
    // fullscreen) scales the UI to match.
    app.ui.scaling = UiScaling::WithWindow { reference_height: 720.0 };
    if let Err(err) = app.window.set_size(1280, 720) {
        eprintln!("Couldn't set the size: {err}");
    }

    app.scene_manager.create_scene("ui", UiShowcase::new);
    app.scene_manager.open_scene("ui");

    app.run();
}
