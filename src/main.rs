use pankarta_software::prelude::*;
use pankarta_software::window::WindowMode;

use game::scenes::{follow_test, main_menu, ocean_lab, play, sandbox};

pub mod game;

/// Mesh name of the ocean surface registered in `main` - see
/// game::ext::mesh_data::build_water_rings_mesh. Each altitude tier is its own
/// mesh (`ocean_surface_mesh(tier)`), swapped by the play scene as it climbs.
pub const OCEAN_SURFACE_MODEL: &str = "OceanSurface";

/// The ocean surface's mesh (see game::ext::mesh_data::WaterTier) - one
/// layout for every altitude, innermost ring first: a detailed area around
/// the camera, then four rings outward, each much coarser than the last,
/// the outermost flat out past the horizon.
///
/// A ring only moves the waves at least ~3 of its cells long (see ocean.wgsl's
/// GEOMETRY_MIN_CELLS), and the water's LOOK is per-pixel everywhere - so
/// the mesh only has to carry the swell's shape near the camera, where it
/// shows against the sky. Each ring's cell is a whole multiple of the one
/// inside it, and its edges land on its own and the next ring's grid
/// (checked by build_water_rings_mesh); ocean.wgsl's ring morph joins them
/// without cracks however big the jump. The ocean lab
/// (game::scenes::ocean_lab) draws it next to the old layout.
/// - main: 4 m cells, 128 m across - moves every wave 12 m and longer;
/// - 16 m cells out to 512 m - the longest waves (the default sea's are
///   10-80 m), faded out by its edge;
/// - 128 m cells to 4 km, 2 km cells to 65 km - flat for the default sea,
///   longer waves for strong winds;
/// - 32 km cells to 1,048 km - flat, out past the horizon (from 20 km up
///   it's ~500 km away).
///
/// Ring sizes: the morph's slide is worked out from the next-grid point a
/// vertex slides onto, so a ring's inner corners stay unmorphed (meeting
/// the ring inside) only while its half-extent is at least ~2x the next
/// ring's cell - each ring here is.
///
/// ~35k triangles (was ~1.07M near the water: 2 m cells, 256 across every
/// ring, doubling out to 32 km, with separate tiers by altitude).
pub const OCEAN_TIERS: &[game::ext::mesh_data::WaterTier] = {
    use game::ext::mesh_data::{WaterRing, WaterTier};
    &[
        WaterTier { min_altitude: f32::NEG_INFINITY, rings: &[
            WaterRing { cell: 4.0, outer_half_extent: 64.0 },
            WaterRing { cell: 16.0, outer_half_extent: 512.0 },
            WaterRing { cell: 128.0, outer_half_extent: 4_096.0 },
            WaterRing { cell: 2_048.0, outer_half_extent: 65_536.0 },
            WaterRing { cell: 32_768.0, outer_half_extent: 1_048_576.0 },
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
    app.scene_manager.create_scene("playing", play::Play::new);
    app.scene_manager.create_scene("sandbox", |scene, app| sandbox::scene::GameLogic::new(scene, app, game::skybox()));
    // Follow-point camera testbed - island/water/sky + a static MQ-9 and the
    // free-fly camera. Uses the procedural sea sky like play.
    app.scene_manager.create_scene("follow_test", |scene, app| follow_test::scene::GameLogic::new(scene, app, Environment::ProceduralSky { sun: true }));
    // The sea's test bench - every setting that changes what it costs to
    // draw, toggled live next to the frame time (see its module doc). Open
    // it by setting game::ext::splash_screen's NEXT_SCENE to "ocean_lab".
    app.scene_manager.create_scene("ocean_lab", ocean_lab::scene::GameLogic::new);

    app.scene_manager.open_scene("splash");
    app.run();
}
