use pankarta_software::prelude::*;
use pankarta_software::rendering::enviroment::environment::SkyboxFaces;
use pankarta_software::window::WindowMode;

use scenes::{follow_test, main_menu, play, sandbox};

pub mod scenes;
pub mod selected_level;
pub mod game_settings;
pub mod head_tracking;
pub mod ui;
pub mod camera;
pub mod debug_overlay;
pub mod frame_plans;
pub mod prefabs;
pub mod screen_effects;
/// What the game needs that the engine doesn't have - kept game-side (see
/// each module).
pub mod ext;

/// `path` (as the game's data files write it: "assets/planes/f16/...",
/// relative to where the game runs) inside the assets folder - which is
/// what the engine loads files relative to.
pub fn asset_relative(path: &str) -> String {
    let path = path.replace('\\', "/");
    let path = path.trim_start_matches("./");
    path.strip_prefix("assets/").unwrap_or(path).to_owned()
}

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

#[tokio::main]
async fn main() {
    let mut app = match App::new().await {
        Ok(app) => app,
        Err(error) => {
            eprintln!("Something went wrong in the definition of the app: {error}");
            return;
        }
    };
    app.window.set_title("Pankarta Software");
    if let Err(error) = app.window.set_window_mode(WindowMode::Fullscreen) {
        eprintln!("{error}");
    }
    // The game's UI is laid out in screen points, as its own engine did -
    // not scaled to a 1080p design height (the engine's default).
    app.ui.scaling = pankarta_software::ui::UiScaling::Fixed;

    // The game's default controls, shipped with it - then the player's own
    // rebinds (only what they changed), kept next to them.
    if let Err(error) = app.input.load_bindings_from_str(include_str!("../settings/input.ron")) {
        eprintln!("the default controls couldn't be read: {error}");
    }
    app.input.set_user_bindings_path(Some("settings/bindings.ron"));

    // The game's own shaders, by name - assets/shaders/ocean.wgsl is "ocean"
    // (see the engine's shader_library). Saving one while the game runs
    // rebuilds it.
    app.renderer.shaders.add_folder("assets/shaders");

    // No floating origin: the engine's (on by default) moves the whole world
    // back toward (0, 0, 0) once the camera is 4 km out, but the game keeps
    // world positions of its own that it doesn't move with it - the map's
    // paths (what autopilots and animations fly), its camera keys, spawn
    // points. With it on, the start animation's camera (~9 km out) shifted
    // the world every frame, and the plane flew at points that weren't
    // where the path is any more. The maps are a few tens of km across,
    // where f32 positions are still mm-precise.
    app.origin.set_enabled(false);
    // The image the flight HUD is drawn into, then composited over the world
    // by the G-vision shader (see frame_plans::flight).
    app.renderer.add_target("hud");
    // What every scene draws unless it sets up its own frame plan.
    app.renderer.default_frame_plan = frame_plans::standard();

    // The sea every scene has: its mesh, and its level and wind as the
    // world's (particles splash on it and drift with it).
    prefabs::ocean::register(&mut app);

    app.scene_manager.create_scene("splash", ext::splash_screen::Splash::new);
    app.scene_manager.create_scene("main_menu", |scene, app| main_menu::scene::GameLogic::new(scene, app, Environment::ProceduralSky { sun: true }));
    app.scene_manager.create_scene("playing", play::Play::new);
    app.scene_manager.create_scene("sandbox", |scene, app| sandbox::scene::GameLogic::new(scene, app, skybox()));
    // Follow-point camera testbed - island/water/sky + a static MQ-9 and the
    // free-fly camera. Uses the procedural sea sky like play.
    app.scene_manager.create_scene("follow_test", |scene, app| follow_test::scene::GameLogic::new(scene, app, Environment::ProceduralSky { sun: true }));

    app.scene_manager.open_scene("splash");
    app.run();
}
