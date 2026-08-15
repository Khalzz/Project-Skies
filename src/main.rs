use std::time::Duration;

use app::App;
use game::{main_menu, play, plane_selection};

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

            app.scene_manager.create_loaded_scene("playing", "./assets/scenes/test_chamber", default_skybox(), play::scene::GameLogic::finish);
            app.scene_manager.create_scene("selecting_plane", plane_selection::scene::GameLogic::new);
            app.scene_manager.create_loaded_scene("main_menu", "./assets/scenes/main_menu", default_skybox(), main_menu::scene::GameLogic::finish);

            app.scene_manager.open_scene("main_menu");

            app.run();
        },
        Err(err) => eprintln!("Something went wrong in the definition of the app: {}", err),
    }

    Ok(())
}
