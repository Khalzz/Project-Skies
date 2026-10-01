use std::time::Duration;

use app::App;
use game::scenes::{follow_test, main_menu, play, sandbox};

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

/// Model name of the ocean surface mesh registered in `main` - see
/// resources::PrimitiveShape::WaterRings.
pub const OCEAN_SURFACE_MODEL: &str = "OceanSurface";

/// A tiny generated cube for nodes that have to be in the render world but
/// are never drawn (the "sun") - see `main`.
pub const MARKER_MODEL: &str = "Marker";

/// The ocean surface's altitude tiers (see resources::WaterTier), each 6
/// rings, innermost first (see resources::build_water_rings_mesh): the
/// rings fine enough to physically move the water (out to ~8 km below 4 km
/// altitude; none above - from that high it's all shading), then flat rings out
/// past the horizon (the water's LOOK is per-pixel everywhere, so where the
/// moving rings end doesn't show). Each ring's cell is a whole multiple of
/// the one inside it, and its inner edge lands on its own grid.
/// - near (below 1 km): 2 m cells to 2 km, 4 m to 4 km, 16 m to 8 km -
///   ~7.9M vertices;
/// - mid (1-4 km up): 8 m to 4 km, 16 m to 8 km - ~1.9M;
/// - high (4 km up and above): 32 m to 8 km, 64 m to 16 km - ~0.5M.
const OCEAN_TIERS: &[resources::WaterTier] = &[
    resources::WaterTier { min_altitude: f32::NEG_INFINITY, rings: &[
        resources::WaterRing { cell: 2.0, outer_half_extent: 2_000.0 },
        resources::WaterRing { cell: 4.0, outer_half_extent: 4_000.0 },
        resources::WaterRing { cell: 16.0, outer_half_extent: 8_192.0 },
        resources::WaterRing { cell: 256.0, outer_half_extent: 32_768.0 },
        resources::WaterRing { cell: 2_048.0, outer_half_extent: 131_072.0 },
        resources::WaterRing { cell: 16_384.0, outer_half_extent: 2_097_152.0 },
    ] },
    resources::WaterTier { min_altitude: 1_000.0, rings: &[
        resources::WaterRing { cell: 8.0, outer_half_extent: 4_000.0 },
        resources::WaterRing { cell: 16.0, outer_half_extent: 8_192.0 },
        resources::WaterRing { cell: 512.0, outer_half_extent: 32_768.0 },
        resources::WaterRing { cell: 4_096.0, outer_half_extent: 262_144.0 },
        resources::WaterRing { cell: 16_384.0, outer_half_extent: 1_048_576.0 },
        resources::WaterRing { cell: 32_768.0, outer_half_extent: 2_097_152.0 },
    ] },
    resources::WaterTier { min_altitude: 4_000.0, rings: &[
        resources::WaterRing { cell: 32.0, outer_half_extent: 8_192.0 },
        resources::WaterRing { cell: 64.0, outer_half_extent: 16_384.0 },
        resources::WaterRing { cell: 1_024.0, outer_half_extent: 65_536.0 },
        resources::WaterRing { cell: 4_096.0, outer_half_extent: 262_144.0 },
        resources::WaterRing { cell: 16_384.0, outer_half_extent: 1_048_576.0 },
        resources::WaterRing { cell: 32_768.0, outer_half_extent: 2_097_152.0 },
    ] },
];

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

            

            // Name the models scenes refer to - nothing is loaded here: each
            // one is read from disk the first time a scene spawns a node using
            // it, and dropped when that scene ends (see
            // resources::declare_model), so a level only holds what it uses.
            // Planes come from their own folders (assets/planes/<name>/).
            //
            // Backface culling is ON for every mesh by default -
            // `.double_sided(...)` opts meshes out of it: `DoubleSidedMeshes::
            // None` / `::All` / `::Meshes(vec!["node_name", ...])`. Mesh names
            // are glTF node names ("Foo#prim1" for a multi-primitive mesh's
            // extras).
            use resources::{DoubleSidedMeshes, ModelSource};
            // Same file the player's jet uses (assets/planes/f16/data.ron), so
            // both share one load.
            resources::declare_model(&mut app, "F16", ModelSource::asset("assets/planes/f16/model/f16.glb").double_sided(DoubleSidedMeshes::Meshes(vec!["Afterburner".to_owned()])));
            resources::declare_model(&mut app, "Water", ModelSource::res("Water/water.gltf"));
            resources::declare_model(&mut app, "F14", ModelSource::res("F14/f14.gltf"));
            resources::declare_model(&mut app, "Ground", ModelSource::res("ground/ground.glb"));
            resources::declare_model(&mut app, "Runway", ModelSource::res("Runway/Runway.glb").double_sided(DoubleSidedMeshes::All));
            resources::declare_model(&mut app, "MQ9", ModelSource::res("MQ-9/mq-9.glb"));
            // The ocean surface - one level-of-detail mesh of nested square
            // rings, in altitude tiers (see OCEAN_TIERS). Spawn it at scale 1 and keep it
            // recentered under the camera (see play::scene::update_water_plane);
            // drawn with the animated water shader (see App::water_shaded_models).
            if let Err(e) = resources::register_primitive_model(&mut app, OCEAN_SURFACE_MODEL, resources::PrimitiveShape::WaterRings { tiers: OCEAN_TIERS }) {
                eprintln!("{e}");
            }
            app.water_shaded_models.insert(OCEAN_SURFACE_MODEL.to_owned());
            // A stand-in model for nodes that need to exist in the render
            // world but are never drawn - e.g. the "sun" (only its position
            // is read, see App::run's lighting). Generated, so free to load.
            if let Err(e) = resources::register_primitive_model(&mut app, MARKER_MODEL, resources::PrimitiveShape::Cube) {
                eprintln!("{e}");
            }
            app.water.set_tier_altitudes(OCEAN_TIERS.iter().map(|tier| tier.min_altitude).collect());
            app.scene_manager.create_loaded_scene(
                "playing",
                // Play uses the procedural clear-day sea sky (see sky.wgsl)
                // instead of the cubemap - default_skybox() is still used by
                // main_menu / sandbox below.
                |device, queue, layout, config, reporter| play::scene::GameLogic::prepare(device, queue, layout, config, reporter, Environment::ProceduralSky),
                play::scene::GameLogic::finish,
            );
            // Its models load under its loading screen, not as it spawns them.
            app.scene_manager.preload_models("playing", play::scene::GameLogic::models);
            app.scene_manager.create_scene("main_menu", |scene, app| main_menu::scene::GameLogic::new(scene, app, default_skybox()));
            app.scene_manager.create_scene("sandbox", |scene, app| sandbox::scene::GameLogic::new(scene, app, default_skybox()));
            // Follow-point camera testbed - island/water/sky + a static MQ-9
            // and the free-fly camera. Uses the procedural sea sky like play.
            app.scene_manager.create_scene("follow_test", |scene, app| follow_test::scene::GameLogic::new(scene, app, Environment::ProceduralSky));

            app.scene_manager.open_scene("playing");

            app.run();
        },
        Err(err) => eprintln!("Something went wrong in the definition of the app: {}", err),
    }

    Ok(())
}
