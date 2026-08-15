use std::time::Duration;

use app::App;
use game::{main_menu, play, plane_selection};

use crate::engine::splash_screen::SplashScreenConfig;

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

            app.scene_manager.create_scene("playing", play::scene::GameLogic::new);
            app.scene_manager.create_scene("selecting_plane", plane_selection::scene::GameLogic::new);
            app.scene_manager.create_scene("main_menu", main_menu::scene::GameLogic::new);

            app.scene_manager.open_scene("main_menu");

            app.run();
        },
        Err(err) => eprintln!("Something went wrong in the definition of the app: {}", err),
    }

    Ok(())
}
