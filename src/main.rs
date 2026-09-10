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

            // Backface culling is ON for every mesh by default. The third arg
            // opts meshes out of it (two-sided): `DoubleSided::None` /
            // `::All` / `::Meshes(&["node_name", ...])`. Mesh names are glTF
            // node names ("Foo#prim1" for a multi-primitive mesh's extras).
            use resources::DoubleSided;
            if let Err(e) = resources::register_model(&mut app, "F16", "F16/f16.glb", DoubleSided::Meshes(&["Afterburner"])) {
                eprintln!("{e}");
            }
            if let Err(e) = resources::register_model(&mut app, "Water", "Water/water.gltf", DoubleSided::None) {
                eprintln!("{e}");
            }
            if let Err(e) = resources::register_model(&mut app, "F14", "F14/f14.gltf", DoubleSided::None) {
                eprintln!("{e}");
            }
            if let Err(e) = resources::register_model(&mut app, "Ground", "ground/ground.glb", DoubleSided::None) {
                eprintln!("{e}");
            }
            if let Err(e) = resources::register_model(&mut app, "Runway", "Runway/Runway.glb", DoubleSided::All) {
                eprintln!("{e}");
            }
            // Procedural test mesh for trying a water shader's vertex
            // displacement against - subdivided so there's actually geometry
            // for a shader to move, unlike the flat Water model above. Spawn
            // a node with Model { model_ref: "WaterPlane" } to try it.
            if let Err(e) = resources::register_primitive_model(&mut app, "WaterPlane", resources::PrimitiveShape::Plane { subdivisions: 1000 }) {
                eprintln!("{e}");
            }
            // Draw "WaterPlane" with the animated water shader instead of the
            // default one - see App::water_shaded_models's own doc comment.
            app.water_shaded_models.insert("WaterPlane".to_owned());
            // Separate, far-lower-detail model for play::scene::spawn_world's
            // "world_far" node - it's forced near-flat (see its own Y-scale
            // trick, read back in water.wgsl's vs_main as y_scale), so it
            // doesn't need "WaterPlane"'s dense subdivision count; kept as a
            // distinct model_ref (not just a second instance of "WaterPlane")
            // specifically so the F6 debug view (below) can hide it alone
            // without hiding "WaterPlane"'s own near/detailed instance too.
            // Just a flat backdrop - its waves are forced to ~zero amplitude
            // (the Y-scale trick, read back as y_scale in water.wgsl), and its
            // colour/fog are per-fragment, so it needs no real geometry. A
            // near-quad keeps it effectively free no matter how large it's
            // scaled in the scene.
            if let Err(e) = resources::register_primitive_model(&mut app, "WaterPlaneFar", resources::PrimitiveShape::Plane { subdivisions: 2 }) {
                eprintln!("{e}");
            }
            app.water_shaded_models.insert("WaterPlaneFar".to_owned());
            // F6 (see settings/input.ron's toggle_water_debug_view) hides
            // just this model_ref rather than isolating to water-only - lets
            // you inspect "world"'s own near/detailed water plane against the
            // rest of the scene without "world_far"'s much-larger, forced-
            // calm surface in the way.
            app.water_debug_hidden_models.insert("WaterPlaneFar".to_owned());
            // Stand-in for a real splash/spray model - see
            // play::scene::GameLogic::update_water_splash's own doc comment
            // for why the water shader itself can't show this (the mesh is
            // far too coarse at this scale for a small, localized effect).
            // Swap for a real model later by registering it under this same
            // "WaterSplash" name - nothing else needs to change.
            if let Err(e) = resources::register_primitive_model(&mut app, "WaterSplash", resources::PrimitiveShape::Cube) {
                eprintln!("{e}");
            }

            app.scene_manager.create_loaded_scene(
                "playing",
                // Play uses the procedural clear-day sea sky (see sky.wgsl)
                // instead of the cubemap - default_skybox() is still used by
                // main_menu / sandbox below.
                |device, queue, layout, config| play::scene::GameLogic::prepare(device, queue, layout, config, Environment::ProceduralSky),
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
