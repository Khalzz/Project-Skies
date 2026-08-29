use std::time::Duration;

use app::App;
use game::scenes::{main_menu, play, sandbox};

use crate::engine::rendering::enviroment::environment::{Environment, SkyboxFaces};
use crate::engine::splash_screen::SplashScreenConfig;

fn default_skybox() -> Environment {
    Environment::Skybox(SkyboxFaces {
        px: "skybox/px.png".to_owned(),
        nx: "skybox/nx.png".to_owned(),
        py: "skybox/py.png".to_owned(),
        ny: "skybox/ny.png".to_owned(),
        pz: "skybox/pz.png".to_owned(),
        nz: "skybox/nz.png".to_owned(),
    })
}

mod app;
mod transform;
mod resources;
pub mod engine;
pub mod game;

// this tokio trait means that main WILL AND CAN be asyncronous (without tokio this is not achievable)
#[tokio::main]
async fn main() -> Result<(), String> {
    // Game Tooling
    match App::new("Pankarta Software", None, None).await {
        Ok(mut app) => {
            app.splash_screen = Some(
                SplashScreenConfig::new(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }, Duration::from_secs(3))
                    .with_text("A Pankarta Software Production")
            );

            

            // Preload every model the code-first entity system needs, once,
            // by name (see resources::register_model's own doc comment for
            // why this happens up front rather than the first time a Node
            // references it) - not tied to any particular scene.

            if let Err(e) = resources::register_model(&mut app, "F16", "F16/f16.gltf") {
                eprintln!("{e}");
            }
            if let Err(e) = resources::register_model(&mut app, "Water", "Water/water.gltf") {
                eprintln!("{e}");
            }
            if let Err(e) = resources::register_model(&mut app, "F14", "F14/f14.gltf") {
                eprintln!("{e}");
            }
            if let Err(e) = resources::register_model(&mut app, "Ground", "ground/ground.glb") {
                eprintln!("{e}");
            }

            app.scene_manager.create_loaded_scene(
                "playing",
                |device, queue, layout, config| play::scene::GameLogic::prepare(device, queue, layout, config, default_skybox()),
                play::scene::GameLogic::finish,
            );
            app.scene_manager.create_scene("main_menu", |scene, app| main_menu::scene::GameLogic::new(scene, app, default_skybox()));
            app.scene_manager.create_scene("sandbox", |scene, app| sandbox::scene::GameLogic::new(scene, app, default_skybox()));

            app.scene_manager.open_scene("main_menu");

            app.run();
        },
        Err(err) => eprintln!("Something went wrong in the definition of the app: {}", err),
    }

    Ok(())
}
