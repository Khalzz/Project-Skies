use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use glyphon::cosmic_text::Align;
use std::sync::mpsc::Sender;

use nalgebra::{Point3, UnitQuaternion, Vector3};

use crate::app::App;
use crate::engine::game_nodes::game_object::{Camera as GameObjectCamera, Cameras};
use crate::engine::particles::ParticleEmitters;
use crate::engine::physics::physics_handler::PhysicsCommand;
use crate::engine::physics::water_contact::WaterContact;
use crate::engine::rendering::ui::ui::Ui;
use crate::engine::scene_manager::node::Node;
use crate::engine::scene_manager::properties::{Model, Transform3D};
use crate::engine::ui::color::UiColor;
use crate::engine::ui::ui_transform::SizeValue;
use crate::resources::{DoubleSidedMeshes, ModelSource};
use crate::engine::scene_manager::scene::Scene;
use crate::game::game_settings::GAME_SETTINGS;
use crate::game::ui::label;
use crate::game::scenes::play::map::{AircraftPlacement, Coalition, MapSpec};
use crate::game::scenes::play::plane::aircraft_spec::AircraftSpec;
use crate::game::scenes::play::plane::{aircraft_physics::AircraftPhysics, effects, messages::{AircraftReload, AircraftState}, plane::Plane};

// After crashing into the water: how long the wreck floats, then how long it
// takes to flood and go under - see WaterContact::sinks.
const WRECK_FLOAT_SECONDS: f32 = 2.0;
const WRECK_SINK_SECONDS: f32 = 6.0;

/// The level's map.ron, or - when it's missing or broken (reported) - just
/// the player in an F-16 on the runway, so the level still opens.
fn load_map(scene_folder: &str) -> MapSpec {
    MapSpec::load(scene_folder).unwrap_or_else(|error| {
        eprintln!("{error} - spawning only the player's F-16");
        MapSpec {
            coalitions: vec![Coalition {
                name: "Blue".to_owned(),
                color: (90, 160, 255),
                aircraft: vec![AircraftPlacement {
                    name: "player".to_owned(),
                    plane: "f16".to_owned(),
                    player: true,
                    position: (0.0, 105.0, -3400.0),
                    rotation: (0.0, 0.0, 0.0),
                    speed: None,
                    throttle: 0.0,
                    path: Vec::new(),
                }],
            }],
        }
    })
}

/// The model file of every plane the level's map.ron puts in it - for the
/// play scene to load them under its loading screen (see
/// SceneManager::preload_models).
pub fn models(scene_folder: &str) -> Vec<ModelSource> {
    let map = load_map(scene_folder);
    let mut planes: Vec<&str> = map.aircraft().map(|(_, aircraft)| aircraft.plane.as_str()).collect();
    planes.sort_unstable();
    planes.dedup();
    planes.into_iter()
        .filter_map(|plane| AircraftSpec::load(plane).ok())
        .map(|spec| ModelSource::asset(&spec.model.model).double_sided(DoubleSidedMeshes::Meshes(spec.model.double_sided)))
        .collect()
}

/// How often (s) each plane's data.ron is checked for edits - see `update`.
const WATCH_INTERVAL_SECONDS: f32 = 0.5;

/// A spawned plane whose data.ron is watched for edits.
struct WatchedPlane {
    node_id: String,
    /// Its folder name under assets/planes/ - what AircraftSpec::load takes.
    name: String,
    data_path: PathBuf,
    /// The file's last-modified time when last read.
    modified: Option<SystemTime>,
}

fn modified_time(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|metadata| metadata.modified()).ok()
}

/// The UI element holding the name tag over the aircraft `node_id` - one
/// top-level element per aircraft (see `update_name_tags`).
fn name_tag_key(node_id: &str) -> String {
    format!("aircraft_name:{node_id}")
}

// A name tag's box (px) - the text is centered in it.
const NAME_TAG_WIDTH: f32 = 240.0;
const NAME_TAG_HEIGHT: f32 = 20.0;
/// How far above the aircraft's center its name tag sits, meters straight up
/// in the world - a fixed height, so up close the tag clears the plane and
/// far away the two close in on each other on screen.
const NAME_TAG_HEIGHT_METERS: f32 = 6.0;

/// What the flight manager keeps about each aircraft it spawned.
struct SpawnedAircraft {
    /// Its data.ron's `initial_velocity` - what a teleport without a map
    /// `speed` sets it going at (see `teleport`).
    data_initial_velocity: Vector3<f32>,
}

/// An aircraft as it is right now in the scene - for the F6 flight editor.
#[derive(Clone, Copy)]
pub struct LiveAircraft {
    pub position: Vector3<f32>,
    pub rotation: UnitQuaternion<f32>,
    /// Indicated airspeed (kt) and altitude (m), from its instruments.
    pub speed_kt: f32,
    pub altitude: f32,
    pub wrecked: bool,
    /// The point of its path its autopilot is flying to (an index), for a
    /// bot with a path.
    pub next_path_point: Option<usize>,
}

/* This code defines the flight manager for the game, managing all planes in the game. */
pub struct flight_manager {
    watched: Vec<WatchedPlane>,
    since_last_check: f32,
    /// The node id of the aircraft the player flies - see `player_id`.
    player_id: String,
    /// The map as the scene has it right now - what spawn_map spawned, with
    /// every change `sync` has applied since.
    applied: MapSpec,
    /// Every aircraft in the scene, by node id.
    spawned: HashMap<String, SpawnedAircraft>,
}

impl flight_manager {
    pub fn new() -> Self {
        flight_manager { watched: Vec::new(), since_last_check: 0.0, player_id: "player".to_owned(), applied: MapSpec::default(), spawned: HashMap::new() }
    }

    /// The node id of the aircraft the player flies - its `name` in map.ron.
    pub fn player_id(&self) -> &str {
        &self.player_id
    }

    /// The map as the scene has it right now - see `sync`.
    pub fn map(&self) -> &MapSpec {
        &self.applied
    }

    /// The name of the coalition the aircraft `node_id` is in - `None` for a
    /// node that isn't an aircraft from the map.
    pub fn coalition_of(&self, node_id: &str) -> Option<&str> {
        self.applied.find(node_id).map(|(coalition, _)| coalition.name.as_str())
    }

    /// Live-updates planes from their data.ron: every WATCH_INTERVAL_SECONDS
    /// it checks each spawned plane's file, and when one was saved, re-reads
    /// it and sends the running jet its new aero (wings, surfaces, pilot
    /// seat) and mass/center of mass - no level restart. A file that doesn't
    /// load (half-saved, a typo) is reported and the jet keeps its previous
    /// values. Model, scale and colliders still need a restart. Call once a
    /// frame.
    pub fn update(&mut self, scene: &mut Scene, delta_time: f32) {
        self.since_last_check += delta_time;
        if self.since_last_check < WATCH_INTERVAL_SECONDS {
            return;
        }
        self.since_last_check = 0.0;

        for plane in &mut self.watched {
            let modified = modified_time(&plane.data_path);
            if modified == plane.modified {
                continue;
            }
            plane.modified = modified;

            let spec = match AircraftSpec::load(&plane.name) {
                Ok(spec) => spec,
                Err(error) => {
                    eprintln!("{error} - keeping the previous values");
                    continue;
                }
            };
            if let Some(spawned) = self.spawned.get_mut(&plane.node_id) {
                spawned.data_initial_velocity = spec.physics.rigidbody.initial_velocity;
            }
            let Some(node) = scene.content.nodes.get_mut(&plane.node_id) else { continue };
            // Its emitters, rebuilt where the file now puts them (the plane
            // switches them on again as needed next frame).
            if let Some(emitters) = node.get_property_mut::<ParticleEmitters>() {
                *emitters = effects::plane_emitters(&spec.effects, &spec.gear);
            }
            if let Some(plane) = node.get_behavior_mut::<Plane>() {
                plane.set_effects(&spec.effects);
            }
            node.push_physics_event(AircraftReload {
                aero: spec.aero,
                engine: spec.engine,
                gear: spec.gear,
                effects: spec.effects,
                mass: spec.physics.rigidbody.mass,
                center_of_mass: spec.physics.rigidbody.center_of_mass,
                colliders: spec.physics.colliders,
            });
            println!("Reloaded '{}' ({}) from {} - aero, engine, gear, effects and mass applied", plane.node_id, plane.name, plane.data_path.display());
        }
    }

    /// Spawns every aircraft in the level's map.ron (see MapSpec), in every
    /// coalition - the player's and the bots, each bot with a name tag over
    /// it in its coalition's color. Before the scene's physics starts.
    pub fn spawn_map(&mut self, scene: &mut Scene, app: &mut App, scene_folder: &str) {
        let map = load_map(scene_folder);
        self.player_id = map.player().name.clone();
        for (coalition, placement) in map.aircraft() {
            if let Err(error) = self.add_plane(scene, app, None, coalition, placement) {
                if placement.player {
                    panic!("{error} - the player's aircraft can't be left out");
                }
                eprintln!("{error} - left out of the level");
            }
        }
        self.applied = map;
    }

    /// Brings the scene in line with `edited` (the F6 flight editor's copy of
    /// the map), right away - only what changed:
    /// - an aircraft gone from it (or renamed) is removed; a new one (or the
    ///   new name) spawned; one whose `plane` changed is spawned again;
    /// - a changed `position`/`rotation`/`speed` teleports the aircraft there,
    ///   at that speed (see `teleport`);
    /// - `throttle`, `player` and its coalition's color apply in place.
    ///
    /// A map that doesn't `validate` (two aircraft with one name, no player
    /// while one is being switched...) changes nothing - its error comes
    /// back. Aircraft that couldn't be spawned are reported too.
    pub fn sync(&mut self, scene: &mut Scene, app: &mut App, physics_command_tx: Option<&Sender<PhysicsCommand>>, edited: &MapSpec) -> Result<(), String> {
        if *edited == self.applied {
            return Ok(());
        }
        edited.validate()?;
        let old = std::mem::replace(&mut self.applied, edited.clone());

        for (_, before) in old.aircraft() {
            let keep = edited.find(&before.name).is_some_and(|(_, after)| after.plane == before.plane);
            if !keep {
                self.remove_plane(scene, app, physics_command_tx, &before.name);
            }
        }

        let mut errors = Vec::new();
        for (coalition, after) in edited.aircraft() {
            let Some((old_coalition, before)) = old.find(&after.name).filter(|(_, before)| before.plane == after.plane) else {
                if let Err(error) = self.add_plane(scene, app, physics_command_tx, coalition, after) {
                    errors.push(error);
                }
                continue;
            };
            if before.position != after.position || before.rotation != after.rotation || before.speed != after.speed {
                self.teleport(scene, physics_command_tx, after);
            }
            if before.path != after.path || before.speed != after.speed {
                if let Some(plane) = scene.content.nodes.get_mut(&after.name).and_then(|node| node.get_behavior_mut::<Plane>()) {
                    plane.set_path(&after.path, cruise_speed(after));
                }
            }
            if before.throttle != after.throttle || before.player != after.player {
                if let Some(plane) = scene.content.nodes.get_mut(&after.name).and_then(|node| node.get_behavior_mut::<Plane>()) {
                    plane.controls.throttle = after.throttle.clamp(0.0, 1.0);
                    plane.is_player = after.player;
                }
            }
            if old_coalition.color != coalition.color {
                Self::color_name_tag(app, &after.name, coalition);
            }
        }
        self.player_id = edited.player().name.clone();

        if errors.is_empty() { Ok(()) } else { Err(errors.join("\n")) }
    }

    /// Spawns every aircraft again, where and as the map has it - fresh
    /// planes (a wreck flies again).
    pub fn respawn_all(&mut self, scene: &mut Scene, app: &mut App, physics_command_tx: Option<&Sender<PhysicsCommand>>) -> Result<(), String> {
        let names: Vec<String> = self.applied.aircraft().map(|(_, aircraft)| aircraft.name.clone()).collect();
        let errors: Vec<String> = names.iter()
            .filter_map(|name| self.respawn(scene, app, physics_command_tx, name).err())
            .collect();
        if errors.is_empty() { Ok(()) } else { Err(errors.join("\n")) }
    }

    /// Spawns the aircraft `name` again, where and as the map has it.
    pub fn respawn(&mut self, scene: &mut Scene, app: &mut App, physics_command_tx: Option<&Sender<PhysicsCommand>>, name: &str) -> Result<(), String> {
        let Some((coalition, placement)) = self.applied.find(name).map(|(coalition, placement)| (coalition.clone(), placement.clone())) else {
            return Err(format!("no aircraft named '{name}'"));
        };
        self.remove_plane(scene, app, physics_command_tx, name);
        self.add_plane(scene, app, physics_command_tx, &coalition, &placement)
    }

    /// Makes where the aircraft `name` is right now (position and attitude)
    /// its place in the map - without moving it. Returns the new place, for
    /// the editor's copy of the map.
    pub fn capture_current(&mut self, scene: &Scene, name: &str) -> Option<((f32, f32, f32), (f32, f32, f32))> {
        let transform = scene.content.renderizable_instances.get(name)?.instance.transform;
        let position = (transform.position.x, transform.position.y, transform.position.z);
        let rotation = AircraftPlacement::rotation_degrees(&transform.rotation);
        let placement = self.applied.find_mut(name)?;
        placement.position = position;
        placement.rotation = rotation;
        Some((position, rotation))
    }

    /// The aircraft `name` as it is right now - `None` if it isn't in the
    /// scene.
    pub fn live(&self, scene: &Scene, name: &str) -> Option<LiveAircraft> {
        let transform = scene.content.renderizable_instances.get(name)?.instance.transform;
        let node = scene.content.nodes.get(name);
        let flight_data = node.and_then(|node| node.physics_state::<AircraftState>()).map(|state| &state.flight_data);
        Some(LiveAircraft {
            position: transform.position,
            rotation: transform.rotation,
            speed_kt: flight_data.map(|data| data.speedometer).unwrap_or(0.0),
            altitude: flight_data.map(|data| data.altimeter).unwrap_or(transform.position.y),
            wrecked: node.and_then(|node| node.get_behavior::<Plane>()).is_some_and(|plane| plane.wrecked),
            next_path_point: node.and_then(|node| node.get_behavior::<Plane>())
                .filter(|plane| !plane.is_player)
                .and_then(|plane| plane.next_path_point()),
        })
    }

    /// Spawns one aircraft from the map, under its `name`, in `coalition` -
    /// sending its body to the physics thread when one is running (see
    /// Scene::spawn_node_live).
    fn add_plane(&mut self, scene: &mut Scene, app: &mut App, physics_command_tx: Option<&Sender<PhysicsCommand>>, coalition: &Coalition, placement: &AircraftPlacement) -> Result<(), String> {
        let aircraft_spec = AircraftSpec::load(&placement.plane)
            .map_err(|error| format!("map aircraft '{}': {error}", placement.name))?;

        let rotation = placement.rotation();
        let mut physics = aircraft_spec.physics.clone();
        let data_initial_velocity = physics.rigidbody.initial_velocity;
        if let Some(speed) = placement.speed {
            physics.rigidbody.initial_velocity = rotation * Vector3::new(0.0, 0.0, speed);
        }
        let mut plane = Plane::new().with_effects(&aircraft_spec.effects).with_throttle(placement.throttle);
        plane.set_path(&placement.path, cruise_speed(placement));
        let plane = if placement.player { plane } else { plane.bot() };

        scene.spawn_node_live(app,
            Node::new(&placement.name)
                .add_behavior(plane)
                .add_property(Transform3D {
                    position: placement.position(),
                    rotation,
                    scale: Vector3::new(1.0, 1.0, 1.0),
                })
                .add_property(Model::from_file(&aircraft_spec.model.model)
                    .double_sided(DoubleSidedMeshes::Meshes(aircraft_spec.model.double_sided.clone()))
                    .scaled(aircraft_spec.model.scale))
                .add_property(effects::plane_emitters(&aircraft_spec.effects, &aircraft_spec.gear))
                .add_property(physics)
                .add_physics_behavior(AircraftPhysics::new(&aircraft_spec.aero, &aircraft_spec.engine, &aircraft_spec.gear, &aircraft_spec.effects))
                .add_physics_behavior(WaterContact::new(app.water.ocean.sea_level()).sinks(WRECK_FLOAT_SECONDS, WRECK_SINK_SECONDS)),
            physics_command_tx,
        ).map_err(|error| format!("map aircraft '{}': {error}", placement.name))?;
        if let Some(instance) = scene.content.renderizable_instances.get_mut(&placement.name) {
            let mut cameras: Cameras = HashMap::new();
            cameras.insert("cockpit".to_owned(), GameObjectCamera { position: aircraft_spec.aero.pilot_position, fov: 70.0 });
            cameras.insert("cinematic".to_owned(), GameObjectCamera { position: Vector3::new(0.0, 1000.0, 900.0), fov: 60.0 });
            cameras.insert("frontal".to_owned(), GameObjectCamera { position: Vector3::new(0.0, 2.895, 16.84), fov: 40.0 });
            instance.instance.metadata.cameras = Some(cameras);
        }

        // Its name over it - placed every frame by update_name_tags (which
        // leaves the player's hidden).
        let mut tag = label(app, &placement.name)
            .set_size(SizeValue::Pixels(NAME_TAG_WIDTH), SizeValue::Pixels(NAME_TAG_HEIGHT))
            .set_font_size(&mut app.ui.text.font_system, 14.0)
            .set_text_color(coalition_color(coalition))
            .set_align(Align::Center)
            .active(false);
        tag.resolve(app.window_manager.size.width as f32, app.window_manager.size.height as f32);
        app.ui.add_to_ui(name_tag_key(&placement.name), tag);

        self.spawned.insert(placement.name.clone(), SpawnedAircraft { data_initial_velocity });

        // Its data.ron is live from here on - see update.
        let data_path = AircraftSpec::data_path(&aircraft_spec.name);
        self.watched.push(WatchedPlane {
            node_id: placement.name.clone(),
            name: aircraft_spec.name.clone(),
            modified: modified_time(&data_path),
            data_path,
        });
        Ok(())
    }

    /// Takes the aircraft `name` out of the scene - node, body, name tag.
    fn remove_plane(&mut self, scene: &mut Scene, app: &mut App, physics_command_tx: Option<&Sender<PhysicsCommand>>, name: &str) {
        scene.despawn_node(app, name, physics_command_tx);
        app.ui.renderizable_elements.remove(&name_tag_key(name));
        app.ui.has_changed = true;
        self.watched.retain(|plane| plane.node_id != name);
        self.spawned.remove(name);
    }

    /// Puts the aircraft at its place in the map, at its map `speed` along
    /// the nose (or its data.ron `initial_velocity` without one), no spin.
    fn teleport(&self, scene: &mut Scene, physics_command_tx: Option<&Sender<PhysicsCommand>>, placement: &AircraftPlacement) {
        let position = placement.position();
        let rotation = placement.rotation();
        let linvel = match placement.speed {
            Some(speed) => rotation * Vector3::new(0.0, 0.0, speed),
            None => self.spawned.get(&placement.name).map(|spawned| spawned.data_initial_velocity).unwrap_or_else(Vector3::zeros),
        };
        if let Some(tx) = physics_command_tx {
            let _ = tx.send(PhysicsCommand::SetTransform { name: placement.name.clone(), translation: position, rotation: rotation.into_inner(), linvel });
        }
        // Shown there right away, not a frame later when physics reports.
        if let Some(instance) = scene.content.renderizable_instances.get_mut(&placement.name) {
            instance.instance.transform.position = position;
            instance.instance.transform.rotation = rotation;
        }
        if let Some(transform) = scene.content.nodes.get_mut(&placement.name).and_then(|node| node.get_property_mut::<Transform3D>()) {
            transform.position = position;
            transform.rotation = rotation;
        }
    }

    fn color_name_tag(app: &mut App, name: &str, coalition: &Coalition) {
        if let Some(tag) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &name_tag_key(name)) {
            let color = coalition_color(coalition);
            tag.update_style(|style| style.set_text_color(color));
            app.ui.has_changed = true;
        }
    }

    /// Keeps each bot's name tag NAME_TAG_HEIGHT_METERS over it on screen -
    /// hidden for the player's own aircraft, while it's off screen or behind
    /// the camera, while `hidden` (the flight HUD is), or when the "Show
    /// aircraft names" setting is off. The caller marks the UI changed.
    pub fn update_name_tags(&self, scene: &Scene, app: &mut App, hidden: bool) {
        let show = !hidden && GAME_SETTINGS.lock().unwrap().show_aircraft_names;
        let (width, height) = (app.window_manager.size.width, app.window_manager.size.height);
        for node_id in self.spawned.keys() {
            let screen_position = if show && *node_id != self.player_id {
                scene.content.renderizable_instances.get(node_id)
                    .map(|instance| Point3::from(instance.instance.transform.position + Vector3::new(0.0, NAME_TAG_HEIGHT_METERS, 0.0)))
                    .and_then(|anchor| app.camera_resources.world_to_screen(scene.cameras.active(), anchor, width, height))
            } else {
                None
            };
            let Some(tag) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &name_tag_key(node_id)) else { continue };
            let Some(screen_position) = screen_position else {
                tag.set_active(false);
                continue;
            };
            tag.set_active(true);
            tag.transform.x = screen_position.x as f32 - tag.transform.width / 2.0;
            // Its bottom edge on the anchor point.
            tag.transform.y = screen_position.y as f32 - tag.transform.height;
            tag.transform.rect.left = tag.transform.x;
            tag.transform.rect.top = tag.transform.y;
            tag.transform.rect.right = tag.transform.x + tag.transform.width;
            tag.transform.rect.bottom = tag.transform.y + tag.transform.height;
        }
    }
}

/// The airspeed a bot holds flying its path (m/s) - its map `speed`, or
/// DEFAULT_CRUISE_SPEED without one.
fn cruise_speed(placement: &AircraftPlacement) -> f32 {
    placement.speed.filter(|speed| *speed > 1.0).unwrap_or(DEFAULT_CRUISE_SPEED)
}

/// See `cruise_speed`.
const DEFAULT_CRUISE_SPEED: f32 = 150.0;

fn coalition_color(coalition: &Coalition) -> UiColor {
    UiColor::Rgb(coalition.color.0, coalition.color.1, coalition.color.2)
}
