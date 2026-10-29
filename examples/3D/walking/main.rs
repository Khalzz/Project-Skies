use nalgebra::Vector3;
use pankarta_software::prelude::*;

pub mod player;
pub mod scene;

#[tokio::main]
async fn main() {
    let mut app = match App::new().await {
        Ok(app) => app,
        Err(err) => {
            eprintln!("Couldn't create the app: {err}");
            return;
        }
    };
    app.window.set_title("Walking");
    app.physics.set_gravity(Vector3::new(0.0, -20.0, 0.0));
    
    input::set_mouse_sensitivity(3.0, 3.0);

    app.renderer.set_ambient_light([0.8, 0.8, 0.8]);

    for (action, key) in [("forward", Key::W), ("back", Key::S), ("left", Key::A), ("right", Key::D), ("jump", Key::Space), ("run", Key::LShift)] {
        input::bind(action, InputSource::key(key));
    }

    app.scene_manager.create_scene("main", scene::WalkingScene::new);
    app.scene_manager.open_scene("main");
    app.run();
}
