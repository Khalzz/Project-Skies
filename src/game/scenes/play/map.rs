use std::collections::HashSet;
use std::path::{Path, PathBuf};

use nalgebra::{UnitQuaternion, Vector3};
use serde::{Deserialize, Serialize};

/// A level's starting setup - `<scene folder>/map.ron`: its coalitions, every
/// aircraft in each, where each one starts and whether the player flies it.
/// Read each time the level opens (a restart included), so edits to it apply
/// on the next restart - or right away from the F6 flight editor (see
/// play::flight_editor), which can also write it back (`save`).
#[derive(Deserialize, Serialize, Clone, PartialEq, Default)]
pub struct MapSpec {
    pub coalitions: Vec<Coalition>,
}

/// A side in the level - `map.ron`'s `coalitions: [ ... ]`. Every aircraft
/// belongs to exactly one, by being listed in it.
#[derive(Deserialize, Serialize, Clone, PartialEq)]
pub struct Coalition {
    /// Its name ("Blue", "Allies"...) - unique within the map.
    pub name: String,
    /// Its color, RGB 0..255 - its aircraft's name tags.
    pub color: (u8, u8, u8),
    #[serde(default)]
    pub aircraft: Vec<AircraftPlacement>,
}

/// One aircraft in a coalition's `aircraft: [ ... ]`.
#[derive(Deserialize, Serialize, Clone, PartialEq)]
pub struct AircraftPlacement {
    /// Its name - the node's id (what level_planning.ron's tracks target) and
    /// the tag shown over it. Unique within the map.
    pub name: String,
    /// Its folder under assets/planes/ ("f16", "mq-9"...).
    pub plane: String,
    /// Flown by the player (stick, throttle, HUD, camera) - exactly one
    /// aircraft in the map has this. The rest are bots.
    #[serde(default, skip_serializing_if = "is_false")]
    pub player: bool,
    /// Where it starts, meters in the world.
    pub position: (f32, f32, f32),
    /// Its starting attitude, degrees about X (pitch), Y (heading) and Z
    /// (roll). Optional - level, facing +Z when left out.
    #[serde(default, skip_serializing_if = "is_zero_rotation")]
    pub rotation: (f32, f32, f32),
    /// Starting airspeed along its nose, m/s. Optional - when left out, the
    /// plane's own data.ron `initial_velocity` is used.
    #[serde(default, skip_serializing_if = "Option::is_none", serialize_with = "plain_some")]
    pub speed: Option<f32>,
    /// Starting throttle, 0..1. Optional - 0 (idle) when left out.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub throttle: f32,
    /// Points it's meant to fly through, in order - meters in the world.
    /// Optional. Nothing follows it yet - it's set up and shown in the F2
    /// flight editor.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<(f32, f32, f32)>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_zero(value: &f32) -> bool {
    *value == 0.0
}

fn is_zero_rotation(value: &(f32, f32, f32)) -> bool {
    *value == (0.0, 0.0, 0.0)
}

/// An optional field written plainly (`speed: 120.0`), the way the file is
/// read - without RON's `Some( ... )` around it.
fn plain_some<S: serde::Serializer>(value: &Option<f32>, serializer: S) -> Result<S::Ok, S::Error> {
    match value {
        Some(value) => serializer.serialize_f32(*value),
        None => serializer.serialize_none(),
    }
}

/// What `save` writes above the data - the format, for whoever edits the file
/// by hand.
const FILE_HEADER: &str = "\
// The level's starting setup - its coalitions and every aircraft in each (see
// play::map::MapSpec). Read each time the level opens, so an edit here shows
// up on the next restart (pause menu > Restart) - or edit it live in game with
// the F6 flight editor, which saves back to this file.
//
// Each coalition:
//   name      - its name (\"Blue\", \"Allies\"...). Unique.
//   color     - RGB 0..255 - its aircraft's name tags.
//   aircraft  - the aircraft in it (below).
//
// Each aircraft:
//   name      - its node id (what level_planning.ron's tracks target) and the
//               tag shown over it (bots only). Unique across the whole map.
//   plane     - its folder under assets/planes/ (\"f16\", \"mq-9\"...).
//   player    - true for the one the player flies (exactly one in the whole
//               map), false or left out for a bot.
//   position  - meters in the world.
//   rotation  - optional, degrees about X (pitch), Y (heading), Z (roll).
//   speed     - optional, starting airspeed along the nose (m/s) - the plane's
//               data.ron initial_velocity when left out.
//   throttle  - optional, starting throttle 0..1 (idle when left out).
//   path      - optional, points (x, y, z) in the world it's meant to fly
//               through, in order. Nothing follows it yet.
";

impl AircraftPlacement {
    pub fn position(&self) -> Vector3<f32> {
        Vector3::new(self.position.0, self.position.1, self.position.2)
    }

    pub fn rotation(&self) -> UnitQuaternion<f32> {
        let (x, y, z) = self.rotation;
        UnitQuaternion::from_euler_angles(x.to_radians(), y.to_radians(), z.to_radians())
    }

    /// The inverse of `rotation` - `rotation` field values (degrees) for an
    /// attitude.
    pub fn rotation_degrees(rotation: &UnitQuaternion<f32>) -> (f32, f32, f32) {
        let (x, y, z) = rotation.euler_angles();
        (x.to_degrees(), y.to_degrees(), z.to_degrees())
    }
}

impl MapSpec {
    /// `<scene folder>/map.ron`.
    pub fn path(scene_folder: &str) -> PathBuf {
        Path::new(scene_folder).join("map.ron")
    }

    /// Loads and checks `<scene folder>/map.ron` - see `validate`.
    pub fn load(scene_folder: &str) -> Result<MapSpec, String> {
        let path = Self::path(scene_folder);
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("map: couldn't read {}: {error}", path.display()))?;
        let ron_options = ron::Options::default().with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME);
        let map: MapSpec = ron_options.from_str(&text)
            .map_err(|error| format!("map: {} isn't valid: {error}", path.display()))?;
        map.validate().map_err(|error| format!("map: {} - {error}", path.display()))?;
        Ok(map)
    }

    /// Writes the map to `<scene folder>/map.ron`, replacing it - with the
    /// format explained above it (FILE_HEADER). Comments added to the file by
    /// hand don't survive.
    pub fn save(&self, scene_folder: &str) -> Result<(), String> {
        self.validate()?;
        let path = Self::path(scene_folder);
        let config = ron::ser::PrettyConfig::new().indentor("  ");
        let body = ron::ser::to_string_pretty(self, config).map_err(|error| format!("map: couldn't write it out: {error}"))?;
        std::fs::write(&path, format!("{FILE_HEADER}{body}\n"))
            .map_err(|error| format!("map: couldn't write {}: {error}", path.display()))
    }

    /// Checks it can be spawned: coalition names unique and not empty,
    /// aircraft names unique across the map, not empty and without a `/`,
    /// exactly one player.
    pub fn validate(&self) -> Result<(), String> {
        let mut coalition_names = HashSet::new();
        for coalition in &self.coalitions {
            if coalition.name.is_empty() || !coalition_names.insert(coalition.name.as_str()) {
                return Err(format!("coalition name '{}' is empty or used twice", coalition.name));
            }
        }

        let mut names = HashSet::new();
        for (_, aircraft) in self.aircraft() {
            // `/` separates UI paths - the name tag couldn't be found.
            if aircraft.name.is_empty() || aircraft.name.contains('/') {
                return Err(format!("aircraft name '{}' can't be empty or have a '/' in it", aircraft.name));
            }
            if !names.insert(aircraft.name.as_str()) {
                return Err(format!("two aircraft are named '{}'", aircraft.name));
            }
        }
        let players = self.aircraft().filter(|(_, aircraft)| aircraft.player).count();
        if players != 1 {
            return Err(format!("{players} aircraft have `player: true`, there should be exactly one"));
        }
        Ok(())
    }

    /// Every aircraft in the map, with the coalition it's in - in file order.
    pub fn aircraft(&self) -> impl Iterator<Item = (&Coalition, &AircraftPlacement)> {
        self.coalitions.iter().flat_map(|coalition| coalition.aircraft.iter().map(move |aircraft| (coalition, aircraft)))
    }

    /// The aircraft named `name`, with its coalition.
    pub fn find(&self, name: &str) -> Option<(&Coalition, &AircraftPlacement)> {
        self.aircraft().find(|(_, aircraft)| aircraft.name == name)
    }

    /// The aircraft named `name`, to change it.
    pub fn find_mut(&mut self, name: &str) -> Option<&mut AircraftPlacement> {
        self.coalitions.iter_mut().flat_map(|coalition| coalition.aircraft.iter_mut()).find(|aircraft| aircraft.name == name)
    }

    /// The aircraft the player flies - `validate` makes sure there's one.
    pub fn player(&self) -> &AircraftPlacement {
        self.aircraft().map(|(_, aircraft)| aircraft).find(|aircraft| aircraft.player).expect("MapSpec::validate checks there's a player")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_test_chamber_map_loads() {
        let map = MapSpec::load("assets/scenes/test_chamber").expect("assets/scenes/test_chamber/map.ron should load");
        for (_, aircraft) in map.aircraft() {
            crate::game::scenes::play::plane::aircraft_spec::AircraftSpec::load(&aircraft.plane)
                .unwrap_or_else(|error| panic!("map aircraft '{}': {error}", aircraft.name));
        }
    }

    #[test]
    fn a_saved_map_reads_back_the_same() {
        let mut map = MapSpec::load("assets/scenes/test_chamber").unwrap();
        // Every optional field set, so each one goes through the round trip.
        let aircraft = &mut map.coalitions[0].aircraft[0];
        aircraft.rotation = (1.0, 90.0, -5.0);
        aircraft.speed = Some(120.0);
        aircraft.throttle = 0.5;
        aircraft.path = vec![(0.0, 500.0, 1000.0), (1000.0, 600.0, 2000.0)];

        let folder = std::env::temp_dir().join("project_skies_map_round_trip");
        std::fs::create_dir_all(&folder).unwrap();
        let folder = folder.to_string_lossy().into_owned();
        map.save(&folder).unwrap();
        let read_back = MapSpec::load(&folder).unwrap();
        assert!(read_back == map, "{}", std::fs::read_to_string(MapSpec::path(&folder)).unwrap());
    }
}
