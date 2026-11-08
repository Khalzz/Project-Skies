pub mod scenes;
pub mod selected_level;
pub mod game_settings;
pub mod head_tracking;
pub mod ui;
pub mod camera;
pub mod debug_overlay;
pub mod frame_plans;
pub mod ocean;
pub mod screen_effects;
/// What the game needs that the engine doesn't have - kept game-side (see
/// each module).
pub mod ext;
/// The models scenes use, by name.
pub mod models;

use pankarta_software::prelude::Environment;
use pankarta_software::rendering::enviroment::environment::SkyboxFaces;

/// The cubemap sky (assets/skybox) - the sandbox's.
pub fn skybox() -> Environment {
    Environment::Skybox(SkyboxFaces {
        px: "skybox/px.png".to_owned(),
        nx: "skybox/nx.png".to_owned(),
        py: "skybox/py.png".to_owned(),
        ny: "skybox/ny.png".to_owned(),
        pz: "skybox/pz.png".to_owned(),
        nz: "skybox/nz.png".to_owned(),
    })
}
