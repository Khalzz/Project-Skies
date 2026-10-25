//! A text input put to work: type a name, press Greet, and a toast says
//! hello - reading what's typed with `UiNode::input_value`.
//!
//! cargo run --example ui_greetings

use pankarta_software::app::App;
use pankarta_software::rendering::enviroment::environment::Environment;
use pankarta_software::rendering::ui::ui::Ui;
use pankarta_software::scene_manager::scene::{Scene, SceneBehaviour};
use pankarta_software::ui::color::UiColor;
use pankarta_software::ui::layer::Layer;
use pankarta_software::ui::ui_transform::PositionValue;
use pankarta_software::ui::widgets::{button, card, show_toast, text, text_input, ButtonStyle, CardStyle, TextInputStyle, ToastStyle};
use pankarta_software::ui::{TextStyle, UiScaling, Weight};

/// Where the name input is - the card's id, then the input's.
const NAME_INPUT: &str = "greeter/name";

struct Greetings;

impl Greetings {
    fn new(scene: &mut Scene, app: &mut App) -> Self {
        scene.settings.environment = Some(Environment::Color(wgpu::Color { r: 0.0, g: 0.1, b: 0.3, a: 1.0 }));

        let heading = TextStyle::default().size(20.0).weight(Weight::BOLD);
        let hint = TextStyle::default().size(14.0).color(UiColor::Rgb(170, 170, 170));

        let greeter = card(&CardStyle::default().gap(10.0))
            .set_position(PositionValue::Center(0.0), PositionValue::Center(0.0))
            .set_child("title", text(app, "Greetings", &heading))
            .set_child("hint", text(app, "Type your name and press Greet.", &hint))
            .set_child("name", text_input(app, "Your name", &TextInputStyle::default().width(240.0).max_length(24)))
            .set_child("greet", button(app, "Greet", &ButtonStyle::default().width(240.0))
                .on_click(greet));

        Layer::new(app).set_child("greeter", greeter).build(app);
        Greetings
    }
}

impl SceneBehaviour for Greetings {}

/// Says hello to whoever's typed in the name input - or asks for a name.
fn greet(app: &mut App) {
    let name = Ui::get_ui_node(&mut app.ui.renderizable_elements, NAME_INPUT)
        .and_then(|node| node.input_value())
        .unwrap_or_default()
        .trim()
        .to_owned();
    if name.is_empty() {
        show_toast(app, "Type a name first", &ToastStyle::alert());
    } else {
        show_toast(app, &format!("Hello, {name}!"), &ToastStyle::info());
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

    app.window.set_title("UI greetings");
    // Designed for this 1280x720 window - a bigger or smaller one (or
    // fullscreen) scales the UI to match.
    app.ui.scaling = UiScaling::WithWindow { reference_height: 720.0 };
    if let Err(err) = app.window.set_size(1280, 720) {
        eprintln!("Couldn't set the size: {err}");
    }

    app.scene_manager.create_scene("greetings", Greetings::new);
    app.scene_manager.open_scene("greetings");

    app.run();
}
