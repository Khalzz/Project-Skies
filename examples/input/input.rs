use pankarta_software::app::App;
use pankarta_software::scene_manager::scene::SceneBehaviour;
use pankarta_software::ui::layer::Layer;
use pankarta_software::ui::ui_transform::PositionValue;
use pankarta_software::ui::widgets::text;
use pankarta_software::ui::TextStyle;

struct EmptyScene;

impl SceneBehaviour for EmptyScene {}

#[tokio::main]
async fn main() {
    let mut app = match App::new().await {
        Ok(app) => app,
        Err(err) => {
            eprintln!("Couldn't create the app: {err}");
            return;
        }
    };

    app.window.set_title("Input Example");

    app.scene_manager.create_scene("main", |_scene, app| {
        let label = text(app, "Input", &TextStyle::default())
            .set_position(PositionValue::Center(0.0), PositionValue::Center(0.0));
        Layer::new(app).set_child("label", label).build(app);
        EmptyScene
    });
    app.scene_manager.open_scene("main");

    app.run();
}
