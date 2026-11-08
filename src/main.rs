use pankarta_software::prelude::*;
use pankarta_software::window::WindowMode;

use game::scenes::{follow_test, main_menu, play, sandbox};

pub mod game;

/// Mesh name of the ocean surface registered in `main` - see
/// game::ext::mesh_data::build_water_rings_mesh. Each altitude tier is its own
/// mesh (`ocean_surface_mesh(tier)`), swapped by the play scene as it climbs.
pub const OCEAN_SURFACE_MODEL: &str = "OceanSurface";

/// The ocean surface's altitude tiers (see game::ext::mesh_data::WaterTier), each
/// 6 rings, innermost first: the rings fine enough to physically move the water
/// (out to ~8 km below 4 km altitude; none above - from that high it's all
/// shading), then flat rings out past the horizon (the water's LOOK is
/// per-pixel everywhere, so where the moving rings end doesn't show). Each
/// ring's cell is a whole multiple of the one inside it, and its inner edge
/// lands on its own grid.
/// - near (below 1 km): 2 m cells to 2 km, 4 m to 4 km, 16 m to 8 km -
///   ~7.9M vertices;
/// - mid (1-4 km up): 8 m to 4 km, 16 m to 8 km - ~1.9M;
/// - high (4 km up and above): 32 m to 8 km, 64 m to 16 km - ~0.5M.
pub const OCEAN_TIERS: &[game::ext::mesh_data::WaterTier] = {
    use game::ext::mesh_data::{WaterRing, WaterTier};
    &[
        WaterTier { min_altitude: f32::NEG_INFINITY, rings: &[
            WaterRing { cell: 2.0, outer_half_extent: 2_000.0 },
            WaterRing { cell: 4.0, outer_half_extent: 4_000.0 },
            WaterRing { cell: 16.0, outer_half_extent: 8_192.0 },
            WaterRing { cell: 256.0, outer_half_extent: 32_768.0 },
            WaterRing { cell: 2_048.0, outer_half_extent: 131_072.0 },
            WaterRing { cell: 16_384.0, outer_half_extent: 2_097_152.0 },
        ] },
        WaterTier { min_altitude: 1_000.0, rings: &[
            WaterRing { cell: 8.0, outer_half_extent: 4_000.0 },
            WaterRing { cell: 16.0, outer_half_extent: 8_192.0 },
            WaterRing { cell: 512.0, outer_half_extent: 32_768.0 },
            WaterRing { cell: 4_096.0, outer_half_extent: 262_144.0 },
            WaterRing { cell: 16_384.0, outer_half_extent: 1_048_576.0 },
            WaterRing { cell: 32_768.0, outer_half_extent: 2_097_152.0 },
        ] },
        WaterTier { min_altitude: 4_000.0, rings: &[
            WaterRing { cell: 32.0, outer_half_extent: 8_192.0 },
            WaterRing { cell: 64.0, outer_half_extent: 16_384.0 },
            WaterRing { cell: 1_024.0, outer_half_extent: 65_536.0 },
            WaterRing { cell: 4_096.0, outer_half_extent: 262_144.0 },
            WaterRing { cell: 16_384.0, outer_half_extent: 1_048_576.0 },
            WaterRing { cell: 32_768.0, outer_half_extent: 2_097_152.0 },
        ] },
    ]
};

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
    // The image the flight HUD is drawn into, then composited over the world
    // by the G-vision shader (see game::frame_plans::flight).
    app.renderer.add_target("hud");
    // What every scene draws unless it sets up its own frame plan.
    app.renderer.default_frame_plan = game::frame_plans::standard();

    // Generated meshes scenes draw with `Mesh::custom(name)`: the ocean's
    // altitude tiers.
    game::ext::mesh_data::register_ocean(&mut app, OCEAN_SURFACE_MODEL, OCEAN_TIERS);

    // The sea every scene has - its level and wind are the world's
    // (particles splash on it and drift with it - see game::ocean).
    let ocean = game::ocean::Ocean::default();
    app.particles.set_sea_level(Some(ocean.sea_level()));
    app.particles.set_wind(ocean.wind());

    app.scene_manager.create_scene("splash", game::ext::splash_screen::Splash::new);
    app.scene_manager.create_scene("main_menu", |scene, app| main_menu::scene::GameLogic::new(scene, app, Environment::ProceduralSky { sun: true }));
    app.scene_manager.create_scene("playing", play::scene::GameLogic::new);
    app.scene_manager.create_scene("sandbox", |scene, app| sandbox::scene::GameLogic::new(scene, app, game::skybox()));
    // Follow-point camera testbed - island/water/sky + a static MQ-9 and the
    // free-fly camera. Uses the procedural sea sky like play.
    app.scene_manager.create_scene("follow_test", |scene, app| follow_test::scene::GameLogic::new(scene, app, Environment::ProceduralSky { sun: true }));

    app.scene_manager.open_scene("splash");
    app.run();
}
