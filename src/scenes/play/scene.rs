use crate::prefabs::aircraft::aircraft::Aircraft;
use crate::prefabs::aircraft::avionics::hud::{self, HUD_LINES, HUD_TEXT};
use crate::prefabs::camera;
use crate::prefabs::camera::camera::Camera;
use crate::prefabs::ground_unit::{
    self,
    air_defense::{radar::Radar, sam::Sam, AirDefenseType},
    UnitType,
};
use crate::prefabs::ocean::{self, Ocean};
use crate::prefabs::windsock;
use crate::prefabs::world::aerial_perspective;
use crate::prefabs::world::atmosphere::Atmosphere;
use crate::scenes::play::battleground::battleground::{BattleUnit, Coalition};
use crate::scenes::play_old::flight_manager::flight_manager as FlightManager;

use nalgebra::Vector3;
use pankarta_software::prelude::*;

use super::battleground::battleground::Battleground;
use super::ui;

pub struct Play {
    ocean: Ocean,
    // F5: every unit's debug gizmos in the level (see set_level_debug)
    forces_debug: bool,
    flight_manager: FlightManager,
    pub battleground: Battleground,
}

impl Play {
    pub fn new(scene: &mut Scene, app: &mut App, environment: Environment) -> Self {
        app.renderer.set_environment(environment);
        app.renderer.frame_plan = crate::frame_plans::sandbox();
        app.renderer.set_shadows(SHADOWS);
        // The cockpit's head-up display (see avionics/hud.rs)
        app.add_line_set(HUD_LINES, false);
        app.add_text_set(HUD_TEXT, false);

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
            .spawn_node(
                app,
                Node::new("Isla de pascua")
                    .with_property(Transform3D {
                        position: Vector3::new(0.0, 0.0, -0.0),
                        ..Default::default()
                    })
                    .with_property(Model::file("models/Maps/isla de pascua/map_gltf/map.gltf"))
                    // Its own triangles as a fixed collider: read with the scene's other assets
                    .with_property(Colliders::new().add(
                        Collider::from_own_model(Meshes::all(), ModelShape::Trimesh).friction(0.8),
                    )),
            )
            .unwrap();

        // Every tree on the island (from OpenStreetMap), merged into one model in the map's own frame -
        // drawn only, so nothing snags on them
        scene
            .spawn_node(
                app,
                Node::new("Isla de pascua trees")
                    .with_property(Transform3D::default())
                    .with_property(Model::file("models/Maps/isla de pascua/trees_merged.glb")),
            )
            .unwrap();

        /*
        let futrono = Node::new("Futrono")
            .with_property(Transform3D {
                position: Vector3::new(5000.0, 0.0, 5000.0),
                ..Default::default()
            })
            .with_property(Model::file("models/Geo/futrono.glb"));
        scene.spawn_node(app, futrono).unwrap();
        */

        // The sea, kept centred under the camera by Ocean::update.
        scene
            .spawn_node(
                app,
                Node::new("world")
                    .with_property(Transform3D::default())
                    .with_property(ocean::surface())
                    .with_property(
                        Atmosphere::STANDARD
                            .with_sea_level(Ocean::default().sea_level())
                            .with_wind(0.0, 14.0),
                    ),
            )
            .unwrap();

        scene
            .spawn_node(
                app,
                Node::new("f16")
                    .with_property(Transform3D {
                        // Parked off the centreline, behind the jets
                        position: on_runway(150.0, -15.0, GEAR_HEIGHT),
                        rotation: runway_heading(),
                        ..Default::default()
                    })
                    .with_property(
                        Model::file("planes/f16/model/f16.glb").double_sided(
                            DoubleSidedMeshes::Meshes(vec!["Afterburner".to_owned()]),
                        ),
                    ),
            )
            .unwrap();

        // Top left: the frame rate and where the camera is - see ui.rs.
        scene.spawn_node(app, ui::node("Camera")).unwrap();

        scene
            .spawn_node(
                app,
                // It finds the ground under it by itself
                windsock::windsock("windsock", on_runway(150.0, -60.0, 1.0)),
            )
            .unwrap();

        scene
            .spawn_node(
                app,
                ground_unit::ground_unit(
                    "ground_unit",
                    // Beside the runway, dropped onto the ground from a little above it
                    on_runway(300.0, 120.0, 5.0),
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
                    on_runway(330.0, 120.0, 5.0),
                    UnitType::AirDefense(AirDefenseType::Radar(Radar::new(40_000.0, false))),
                ),
            )
            .unwrap();

        let mut battleground = Battleground::new();

        battleground
            .spawn(
                scene,
                app,
                BattleUnit::Aircraft {
                    unit: Aircraft::f16(),
                },
                Transform3D {
                    // Resting on its gear, so it starts still instead of falling onto the runway
                    position: on_runway(300.0, -12.0, GEAR_HEIGHT),
                    rotation: runway_heading(),
                    ..Default::default()
                },
                Coalition::Blue,
            )
            .unwrap();

        battleground
            .spawn(
                scene,
                app,
                BattleUnit::Aircraft {
                    unit: Aircraft::f16(),
                },
                Transform3D {
                    // Resting on its gear, so it starts still instead of falling onto the runway
                    position: on_runway(300.0, 12.0, GEAR_HEIGHT),
                    rotation: runway_heading(),
                    ..Default::default()
                },
                Coalition::Blue,
            )
            .unwrap();

        let player = battleground
            .spawn(
                scene,
                app,
                BattleUnit::Aircraft {
                    unit: Aircraft::f16(),
                },
                Transform3D {
                    // Resting on its gear, so it starts still instead of falling onto the runway
                    position: on_runway(320.0, 0.0, GEAR_HEIGHT),
                    rotation: runway_heading(),
                    ..Default::default()
                },
                Coalition::Blue,
            )
            .unwrap();
        // edit this, prioritize the battleground system where we define to what aliation each bot gooes, and
        // the flight manager only as a property to a bot to define his flight animation/path

        scene.spawn_node(app, Camera::new(&player).prefab(scene));
        battleground.set_player(scene, &player);

        let mut flight_manager = FlightManager::new();

        /*
            flight_manager
            .spawn_one(scene, app, "./assets/scenes/sandbox", "mq-9")
            .unwrap();

            flight_manager
            .spawn_one(scene, app, "./assets/scenes/sandbox", "viper")
            .unwrap();
        */

        Self {
            ocean: Ocean::default(),
            forces_debug: false,
            battleground,
            flight_manager,
        }
    }
}

/*
    Shadows for flying: from the cockpit out to 4 km (hills, buildings, trees and the other jets cast
    them from the air). 6 maps spread evenly from `near` to `distance`, each (4000 / 2)^(1/5) = 4.6x
    the one before, reaching ~2 m, 9 m, 42 m, 190 m, 870 m and 4 km. At 1024 texels a map that's
    millimetres in the cockpit (canopy frame, panel), ~0.2 m around the jet on the runway and ~10 m
    for the far hills - where a texel is that small on screen anyway. 1024 maps: 4 MB each, 24 MB for
    the six (and as much for the cached still casters).
*/
const SHADOWS: Shadows = Shadows {
    enabled: true,
    distance: 4_000.0,
    cascades: 6,
    near: 2.0,
    resolution: 1024,
    softness: 1.0, // edges fade over ~1.5 texels
    blur: 0.0,
    on_transparent: true,
};

/*
    Mataveri's runway on the Isla de Pascua model (its "Runway" mesh, the model at the world's origin):
    the centre of each end, from the mesh's ends. 45 m wide, 3301 m long, climbing 60 m from the
    west end (78.27 m) to the east one (138.56 m) - the real one's slope - with no slope across it.
    It follows the ground, sagging up to ~0.5 m under the straight line between its ends; the heights
    here are the line through its surface where the jets start (150-320 m from the west end), 0.5 m
    under the ends', so they're put right on it there.
*/
const RUNWAY_WEST: Vector3<f32> = Vector3::new(-9607.80, 77.77, 3315.40);
const RUNWAY_EAST: Vector3<f32> = Vector3::new(-6685.90, 138.06, 4851.20);
/*
    An F-16 resting on its gear: its origin this far over the ground. Its legs reach 2.2 m under it
    (landing_gear.rs) and settle under its weight: 11.9 t over a centre of mass 2.244 m aft, so 74% on
    the mains (43 kN each) and 26% on the nose (30 kN), compressing their gas springs
    k·x·(1 + 2(x/stroke)²) about 0.12 m (mains) and 0.125 m (nose). Spawned lower than this they start
    squeezed past their stroke into the bottom-out stop, which throws the jet up.
*/
const GEAR_HEIGHT: f32 = 2.08;

// Along the runway (from the west end, toward the east), across it (+ to the right looking east) and
// above its surface, meters
fn on_runway(along: f32, across: f32, above: f32) -> Vector3<f32> {
    let forward = (RUNWAY_EAST - RUNWAY_WEST).normalize();
    let right = forward.cross(&Vector3::y()).normalize();
    RUNWAY_WEST + forward * along + right * across + Vector3::y() * above
}

// Nose (-Z) down the runway toward the east end, pitched up with its slope
fn runway_heading() -> UnitQuaternion<f32> {
    let forward = (RUNWAY_EAST - RUNWAY_WEST).normalize();
    let up = (Vector3::y() - forward * forward.y).normalize();
    UnitQuaternion::face_towards(&-forward, &up)
}

impl SceneBehaviour for Play {
    fn update(&mut self, scene: &mut Scene, app: &mut App, _ctx: &mut FrameContext) {
        // Its data.ron's edits, live; and every aircraft's name over it.
        self.flight_manager.update(scene, app.time.delta_time);
        self.flight_manager.update_name_tags(scene, app, false);

        // F5: every unit's debug gizmos in the level
        if app.input.is_action_just_pressed("toggle_forces_debug") {
            self.forces_debug = !self.forces_debug;
            set_level_debug(scene, self.forces_debug);
        }
        // F6: every collider in the level (the physics world's, so everything with a body)
        if app.input.is_action_just_pressed("toggle_colliders_debug") {
            let physics = &mut app.renderer.debug.physics;
            physics.colliders = !physics.colliders;
        }
    }

    fn late_update(&mut self, scene: &mut Scene, app: &mut App, _ctx: &mut FrameContext) {
        // The planes' effects (engine smoke, contrails...) onto the
        // engine's emitters, once every behavior has driven them.
        crate::ext::emitters::sync(scene, app);
    }

    fn after_cameras(&mut self, scene: &mut Scene, app: &mut App) {
        // The sea's waves, around where the camera is drawn from.
        self.ocean
            .update(scene, "world", app.time.delta_time, &app.origin);
        aerial_perspective::set_view(scene, app);
        draw_hud(scene, app);
    }
}

// Every unit in the level draws its debug gizmos, or none does: aircraft (forces, aero), ground units, radars
// (beam, contacts), the windsock (the wind)
fn set_level_debug(scene: &mut Scene, debug: bool) {
    for node in scene.content.nodes.iter_mut() {
        if let Some(aircraft) = node.get_behavior_mut::<Aircraft>() {
            aircraft.debug = debug;
        }
        if let Some(unit) = node.get_behavior_mut::<ground_unit::GroundUnit>() {
            unit.debug = debug;
        }
        if let Some(radar) = node.get_behavior_mut::<Radar>() {
            radar.debug = debug;
        }
        if let Some(windsock) = node.get_behavior_mut::<windsock::Windsock>() {
            windsock.debug = debug;
        }
    }
}

// The HUD of the jet the camera's in, once the camera's placed this frame; none outside the cockpit
fn draw_hud(scene: &Scene, app: &mut App) {
    hud::clear(app);
    let Some(camera) = scene.get_node("Camera") else {
        return;
    };
    let (Some(lens), Some(eye)) = (
        camera.get_property::<CameraLens>(),
        camera.get_property::<Transform3D>(),
    ) else {
        return;
    };
    let Some(view) = camera.get_behavior::<Camera>() else {
        return;
    };
    if !view.in_cockpit() {
        return;
    }
    let Some(jet) = view.following().and_then(|id| scene.get_node(id)) else {
        return;
    };
    let (Some(transform), Some(aircraft)) = (
        jet.get_property::<Transform3D>(),
        jet.get_behavior::<Aircraft>(),
    ) else {
        return;
    };
    // The INS attitude as it's drawn this frame, so the horizon line stays on the horizon between steps
    let mut sensors = aircraft.sensors;
    sensors.attitude = transform.rotation;
    // Where the eye is in the jet: the glass is seen from there
    let eye_in_jet = transform.rotation.inverse() * (eye.position - transform.position);
    let hud = &aircraft.avionics.hud;
    let area = hud.optics.visible(eye_in_jet);
    let picture = hud.symbology(&sensors, &area);
    let view = hud::View {
        eye: eye.position,
        rotation: eye.rotation,
        fovy: lens.fovy,
    };
    hud::draw(app, &picture, &area, &view, transform.rotation);
}
