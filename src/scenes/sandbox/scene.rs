use crate::prefabs::camera::FreeCamera;
use crate::prefabs::ground_unit::{
    self,
    air_defense::{
        radar::{Radar, RADAR_LINES},
        sam::Sam,
        AirDefenseType,
    },
    UnitType,
};
use crate::prefabs::island;
use crate::prefabs::ocean::{self, Ocean};
use crate::scenes::play::flight_manager::flight_manager as FlightManager;

use nalgebra::Vector3;
use pankarta_software::prelude::*;

use super::ui;

pub struct Sandbox {
    ocean: Ocean,
    /// The aircraft in the sandbox - spawned, their name tags kept over
    /// them - see play::flight_manager.
    flight_manager: FlightManager,
}

impl Sandbox {
    pub fn new(scene: &mut Scene, app: &mut App, environment: Environment) -> Self {
        app.renderer.set_environment(environment);
        // The radars' contact lines (see air_defense/radar.rs) - registered, and drawn.
        app.add_line_set(RADAR_LINES, false);
        app.renderer.frame_plan = crate::frame_plans::sandbox();

        // The sun: the light the world's lit by. The sky's sun disc and glow
        // are drawn where it shines from - turn this and they move with it.
        scene
            .spawn_node(
                app,
                Node::new("sun")
                    .with_property(Transform3D::facing(Vector3::new(-0.45, -0.64, -0.62)))
                    .with_property(Light::directional([0.7, 0.7, 0.8])),
            )
            .unwrap();

        scene
            .spawn_node(app, island::island("island", Some(island::trimesh())))
            .unwrap();

        // The sea, kept centred under the camera by Ocean::update.
        scene
            .spawn_node(
                app,
                Node::new("world")
                    .with_property(Transform3D::default())
                    .with_property(ocean::surface()),
            )
            .unwrap();

        scene
            .spawn_node(
                app,
                Node::new("f16")
                    .with_property(Transform3D {
                        position: Vector3::new(500.0, 130.0, 900.0),
                        ..Default::default()
                    })
                    .with_property(
                        Model::file("planes/f16/model/f16.glb").double_sided(
                            DoubleSidedMeshes::Meshes(vec!["Afterburner".to_owned()]),
                        ),
                    ),
            )
            .unwrap();

        scene
            .spawn_node(
                app,
                Node::new("camera")
                    .with_property(Transform3D {
                        position: Vector3::new(0.0, 110.0, 0.0),
                        ..Transform3D::facing(Vector3::new(-0.4, -0.35, 1.0))
                    })
                    .with_property(CameraLens::new(60.0))
                    .with_behavior(FreeCamera::new()),
            )
            .unwrap();

        // Top left: the frame rate and where the camera is - see ui.rs.
        scene.spawn_node(app, ui::node("camera")).unwrap();

        scene
            .spawn_node(
                app,
                ground_unit::ground_unit(
                    "ground_unit",
                    Vector3::new(0.0, 110.0, 0.0),
                    UnitType::AirDefense(AirDefenseType::Sam(Sam::new(true, 10_000.0))),
                ),
            )
            .unwrap();

        // An MPQ-64 style search radar next to it - see air_defense/radar.rs.
        scene
            .spawn_node(
                app,
                ground_unit::ground_unit(
                    "radar",
                    Vector3::new(12.0, 110.0, 0.0),
                    UnitType::AirDefense(AirDefenseType::Radar(Radar::new(40_000.0, true))),
                ),
            )
            .unwrap();

        // edit this, prioritize the battleground system where we define to what aliation each bot gooes, and
        // the flight manager only as a property to a bot to define his flight animation/path

        let mut flight_manager = FlightManager::new();
        flight_manager
            .spawn_one(scene, app, "./assets/scenes/sandbox", "mq-9")
            .unwrap();
        flight_manager
            .spawn_one(scene, app, "./assets/scenes/sandbox", "viper")
            .unwrap();

        Self {
            ocean: Ocean::default(),
            flight_manager,
        }
    }
}

impl SceneBehaviour for Sandbox {
    fn update(&mut self, scene: &mut Scene, app: &mut App, _ctx: &mut FrameContext) {
        // The radars refill their contact lines every frame - after this, in their updates.
        if let Some(lines) = app.renderer.render_sets.get_lines_mut(RADAR_LINES) {
            lines.dynamic.clear();
        }
        // Its data.ron's edits, live; and every aircraft's name over it.
        self.flight_manager.update(scene, app.time.delta_time);
        self.flight_manager.update_name_tags(scene, app, false);
        // F5 / F9: the physics debug view - colliders, and every sensor's
        // area (cyan while empty, green with something in it), the truck's
        // IR sensor among them.
        if app.input.is_action_just_pressed("toggle_debug") {
            app.renderer.debug.physics = !app.renderer.debug.physics;
        }
    }

    fn late_update(&mut self, scene: &mut Scene, app: &mut App, _ctx: &mut FrameContext) {
        // The planes' effects (engine smoke, contrails...) onto the
        // engine's emitters, once every behavior has driven them.
        crate::ext::emitters::sync(scene, app);
    }

    fn after_cameras(&mut self, scene: &mut Scene, app: &mut App) {
        // The sea's waves, around where the camera is drawn from.
        self.ocean.update(scene, "world", app.time.delta_time);
    }
}
