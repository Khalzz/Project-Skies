use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use nalgebra::{UnitQuaternion, Vector3};

use crate::app::App;
use crate::engine::game_nodes::game_object::{Camera as GameObjectCamera, Cameras};
use crate::engine::physics::water_contact::WaterContact;
use crate::engine::scene_manager::node::Node;
use crate::engine::scene_manager::properties::{Model, Transform3D};
use crate::resources::DoubleSidedMeshes;
use crate::engine::scene_manager::scene::Scene;
use crate::game::scenes::play::plane::aircraft_spec::AircraftSpec;
use crate::game::scenes::play::plane::{aircraft_physics::AircraftPhysics, effects, messages::AircraftReload, plane::Plane};

// After crashing into the water: how long the wreck floats, then how long it
// takes to flood and go under - see WaterContact::sinks.
const WRECK_FLOAT_SECONDS: f32 = 2.0;
const WRECK_SINK_SECONDS: f32 = 6.0;

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

/* This code defines the flight manager for the game, managing all planes in the game. */
pub struct flight_manager {
    watched: Vec<WatchedPlane>,
    since_last_check: f32,
}

impl flight_manager {
    pub fn new() -> Self {
        flight_manager { watched: Vec::new(), since_last_check: 0.0 }
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
            let Some(node) = scene.content.nodes.get_mut(&plane.node_id) else { continue };
            node.push_physics_event(AircraftReload {
                aero: spec.aero,
                engine: spec.engine,
                mass: spec.physics.rigidbody.mass,
                center_of_mass: spec.physics.rigidbody.center_of_mass,
                colliders: spec.physics.colliders,
            });
            println!("Reloaded '{}' from {} - aero, engine and mass applied", plane.name, plane.data_path.display());
        }
    }

    pub fn add_plane(&mut self, scene: &mut Scene, app: &mut App) {
      //let aircraft_spec = AircraftSpec::load("mq-9").unwrap();
      let aircraft_spec = AircraftSpec::load("f16").unwrap();

      scene.spawn_node(app,
          Node::new("player")
              .add_behavior(Plane::new())
              .add_property(Transform3D {
                  position: Vector3::new(0.0, 100.0, -3400.0),
                  rotation: UnitQuaternion::identity(),
                  scale: Vector3::new(1.0, 1.0, 1.0),
              })
              .add_property(Model::from_file(&aircraft_spec.model.model)
                  .double_sided(DoubleSidedMeshes::Meshes(aircraft_spec.model.double_sided.clone()))
                  .scaled(aircraft_spec.model.scale))
              .add_property(effects::plane_emitters())
              .add_property(aircraft_spec.physics.clone())
              .add_physics_behavior(AircraftPhysics::new(&aircraft_spec.aero, &aircraft_spec.engine))
              .add_physics_behavior(WaterContact::new(app.water.ocean.sea_level()).sinks(WRECK_FLOAT_SECONDS, WRECK_SINK_SECONDS))
      ).expect("play should only spawn 'player' once");
      if let Some(player) = scene.content.renderizable_instances.get_mut("player") {
          let mut cameras: Cameras = HashMap::new();
          cameras.insert("cockpit".to_owned(), GameObjectCamera { position: Vector3::new(0.0, 1.029, 6.485), fov: 70.0 });
          cameras.insert("cinematic".to_owned(), GameObjectCamera { position: Vector3::new(0.0, 1000.0, 900.0), fov: 60.0 });
          cameras.insert("frontal".to_owned(), GameObjectCamera { position: Vector3::new(0.0, 2.895, 16.84), fov: 40.0 });
          player.instance.metadata.cameras = Some(cameras);
      }

      // Its data.ron is live from here on - see update.
      let data_path = AircraftSpec::data_path(&aircraft_spec.name);
      self.watched.push(WatchedPlane {
          node_id: "player".to_owned(),
          name: aircraft_spec.name.clone(),
          modified: modified_time(&data_path),
          data_path,
      });
    }
}
