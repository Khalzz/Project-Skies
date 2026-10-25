//! The smallest engine project: a window with one empty scene.
//!
//! cargo run --example basic_window

use pankarta_software::app::App;
use pankarta_software::ui::layer::Layer;
use pankarta_software::rendering::ui::ui::Ui;
use pankarta_software::ui::ui_transform::SizeValue;
use pankarta_software::ui::widgets::text;
use pankarta_software::ui::TextStyle;
use pankarta_software::scene_manager::scene::SceneBehaviour;

/// A scene with nothing in it but a frame rate counter.
#[derive(Default)]
struct EmptyScene {
    // Frames and seconds since the counter last changed - it shows their
    // average, twice a second, instead of a number too jumpy to read.
    frames: u32,
    elapsed: f32,
}

impl SceneBehaviour for EmptyScene {
    fn on_spawn(&mut self, _scene: &mut pankarta_software::prelude::Scene, app: &mut App) {
        // Wide enough for any frame rate - it's sized to its first text otherwise.
        let framerate_label = text(app, "600 fps", &TextStyle::default())
            .set_padding((10.0, 10.0))
            .set_size(SizeValue::Pixels(200.0), SizeValue::Fit);

        Layer::new(app).set_child("framerate", framerate_label).build(app);
    }

    fn update(&mut self, _scene: &mut pankarta_software::prelude::Scene, app: &mut App, _ctx: &mut pankarta_software::prelude::FrameContext) {
        self.frames += 1;
        self.elapsed += app.time.unscaled_delta_time;
        if self.elapsed < 0.5 {
            return;
        }
        let fps = self.frames as f32 / self.elapsed;
        self.frames = 0;
        self.elapsed = 0.0;

        // The label is UI, not a node - found in app.ui by the name the
        // Layer gave it.
        if let Some(label) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "framerate").and_then(|node| node.as_label_mut()) {
            label.set_text(&mut app.ui.text.font_system, &format!("{fps:.0} fps"), true);
        }
    }
}

#[tokio::main]
async fn main() {
    // 800x600, windowed, resizable, "Pankarta Software" - hidden until run().
    let mut app = match App::new().await {
        Ok(app) => app,
        Err(err) => {
            eprintln!("Couldn't create the app: {err}");
            return;
        }
    };

    app.scene_manager.create_scene("main", |_scene, _app| EmptyScene::default());
    app.scene_manager.open_scene("main");

    app.run();
}
