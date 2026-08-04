use std::collections::HashMap;

use app::App;
use game::{play, plane_selection};
use engine::scene_manager::scene::{GameState, SceneManager, ScenePool};

mod app;
mod transform;
mod resources;

// Generic, game-agnostic infrastructure - reusable across different games.
// Each directory declares its own children in its own file (see src/engine.rs,
// src/engine/rendering.rs, etc.) instead of one tree here.
pub mod engine;

// This game's own content - scenes, gameplay logic, flight model. Everything
// here is specific to this game, not reusable engine machinery. Each directory
// declares its own children in its own file (see src/game.rs, src/game/play.rs,
// etc.) instead of one tree here, so adding a file means editing the file next
// to it, not this one.
pub mod game;

// this tokio trait means that main WILL AND CAN be asyncronous (without tokio this is not achievable)
#[tokio::main]
async fn main() -> Result<(), String> {
    // Game Tooling
    match App::new("Pankarta Software", None, None).await {
        Ok(mut app) => {
            let mut scenes: ScenePool = HashMap::new();
            scenes.insert(GameState::Playing, Box::new(play::play::GameLogic::new(&mut app)));
            scenes.insert(GameState::SelectingPlane, Box::new(plane_selection::plane_selection::GameLogic::new(&mut app)));

            app.scene_manager = SceneManager::new(scenes, GameState::Playing);
            app.run();
        },
        Err(err) => eprintln!("Something went wrong in the definition of the app: {}", err),
    }

    Ok(())
}
