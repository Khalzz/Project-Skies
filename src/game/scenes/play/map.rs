use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use nalgebra::{UnitQuaternion, Vector3};
use serde::{Deserialize, Serialize};

use crate::engine::scene_manager::properties::Space;

/// A level's starting setup - `<scene folder>/map.ron`: its coalitions, every
/// aircraft in each, where each one starts and whether the player flies it.
/// Read each time the level opens (a restart included), so edits to it apply
/// on the next restart - or right away from the flight editor (F10's editor view) (see
/// play::flight_editor), which can also write it back (`save`).
#[derive(Deserialize, Serialize, Clone, PartialEq, Default)]
pub struct MapSpec {
    pub coalitions: Vec<Coalition>,
    /// Every other node the level adds - cameras, mostly - made in the
    /// editor's hierarchy. Optional.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<NodeSpec>,
    /// The level's paths, by name - flown by aircraft, followed by anything
    /// (see `AircraftPlacement::follow`, `NodeSpec::follow`, `PathClip`).
    /// Optional.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<PathSpec>,
    /// Every node's animations, by the node's id - an aircraft, a map node,
    /// any node the level spawns. Optional.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub animations: BTreeMap<String, Vec<AnimationDef>>,
}

/// A named path - `map.ron`'s `paths: [ ... ]`.
#[derive(Deserialize, Serialize, Clone, PartialEq, Debug)]
pub struct PathSpec {
    /// Unique among the paths.
    pub name: String,
    /// Its points in order, meters in the world (x, altitude, z).
    #[serde(default)]
    pub points: Vec<(f32, f32, f32)>,
    /// Round and round, back to the first point - otherwise once. Optional,
    /// true when left out.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub looped: bool,
    /// Followed (not flown) as a smooth curve through its points instead of
    /// straight lines between them. Optional.
    #[serde(default, skip_serializing_if = "is_false")]
    pub smooth: bool,
}

/// One of a node's animations - `map.ron`'s `animations: { "<node>": [ ... ] }`.
/// Played on its own timeline: the paths its node follows when (`paths`),
/// what the camera shows when (`camera`), and one-shot `events`.
#[derive(Deserialize, Serialize, Clone, PartialEq, Debug)]
pub struct AnimationDef {
    /// Unique among its node's animations.
    pub name: String,
    /// How long it runs (s).
    #[serde(default = "default_length")]
    pub length: f32,
    /// Plays as soon as the level starts.
    #[serde(default, skip_serializing_if = "is_false")]
    pub on_start: bool,
    /// Starts over when it ends - otherwise it ends there.
    #[serde(default, skip_serializing_if = "is_false")]
    pub looped: bool,
    /// The player has no controls while it plays (and its node feels no G).
    /// Optional, true when left out.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub lock_input: bool,
    /// The HUD's hidden while it plays - it's there to watch. Optional,
    /// true when left out.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub hide_hud: bool,
    /// The paths its node follows, each from `start` to `end`. Optional.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<PathClip>,
    /// What the camera shows: each key a whole camera setup from its time
    /// on. Optional - none: the player's view stays.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub camera: Vec<CameraKey>,
    /// Done once, when it gets there. Optional.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<AnimEvent>,
}

fn default_length() -> f32 {
    10.0
}

impl AnimationDef {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), length: default_length(), on_start: false, looped: false, lock_input: true, hide_hud: true, paths: Vec::new(), camera: Vec::new(), events: Vec::new() }
    }

    /// Paths, camera keys and events in time order.
    pub fn sort(&mut self) {
        self.paths.sort_by(|a, b| a.start.total_cmp(&b.start));
        self.camera.sort_by(|a, b| a.time.total_cmp(&b.time));
        self.events.sort_by(|a, b| a.time.total_cmp(&b.time));
    }
}

/// From `start` to `end` (s into its animation) the animation's node follows
/// the map path `path`.
#[derive(Deserialize, Serialize, Clone, PartialEq, Debug)]
pub struct PathClip {
    pub start: f32,
    pub end: f32,
    pub path: String,
    /// Placed along it by time - the whole path from `start` to `end`,
    /// whatever the physics - or (an aircraft) flown along it.
    #[serde(default, skip_serializing_if = "FollowMode::is_positioned")]
    pub mode: FollowMode,
    /// Flown: the airspeed it holds, knots - left out, as fast as its
    /// pilot's skill flies (an Ace at full power).
    #[serde(default, skip_serializing_if = "Option::is_none", serialize_with = "plain_some")]
    pub speed_kt: Option<f32>,
    /// Flown: the pilot's skill it's flown with. Optional - an aircraft's own
    /// `difficulty` when left out.
    #[serde(default, skip_serializing_if = "Option::is_none", serialize_with = "plain_some")]
    pub skill: Option<Difficulty>,
    /// Positioned: nose first along the path. Optional, true when left out.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub orient: bool,
}

/// How a camera key's reached from the key before it.
#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Transition {
    /// A hard cut, at its time.
    #[default]
    Cut,
    /// A smooth move from the key before, all the way to its time.
    Smooth,
}

/// A whole camera setup, from `time` (s into its animation) on - see
/// `AnimationDef::camera`.
#[derive(Deserialize, Serialize, Clone, PartialEq, Debug)]
pub struct CameraKey {
    pub time: f32,
    #[serde(default, skip_serializing_if = "Transition::is_cut")]
    pub transition: Transition,
    /// Meters. With `follow` and `follow_rotation`, an offset in that node's
    /// frame. With `follow` and rotation off, an offset along the world's
    /// axes from it. A place in the world otherwise.
    pub position: (f32, f32, f32),
    /// Degrees about X, Y and Z - same space as `position`. Ignored with
    /// `look_at`.
    #[serde(default, skip_serializing_if = "is_zero_rotation")]
    pub rotation: (f32, f32, f32),
    /// Vertical field of view, degrees.
    pub fov: f32,
    /// A node it moves with. Optional.
    #[serde(default, skip_serializing_if = "Option::is_none", serialize_with = "plain_some")]
    pub follow: Option<String>,
    /// The offset turns with `follow` - a camera on top stays on top when
    /// that node rolls. Off: it follows the position only and keeps its own
    /// rotation. Optional, on when left out.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub follow_rotation: bool,
    /// A node it keeps looking at. Optional.
    #[serde(default, skip_serializing_if = "Option::is_none", serialize_with = "plain_some")]
    pub look_at: Option<String>,
    /// Drifts while it holds, instead of sitting locked.
    #[serde(default, skip_serializing_if = "is_false")]
    pub wave: bool,
    /// How far the wave travels (m).
    #[serde(default = "default_wave_amount", skip_serializing_if = "is_default_wave_amount")]
    pub wave_amount: f32,
    /// How fast the wave travels - 1 is a slow swing, higher is quicker.
    #[serde(default = "default_wave_speed", skip_serializing_if = "is_default_wave_speed")]
    pub wave_speed: f32,
    /// What the wave moves: the camera itself, or the point it looks at.
    #[serde(default, skip_serializing_if = "WaveOn::is_position")]
    pub wave_on: WaveOn,
}

/// What a camera key's wave moves.
#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum WaveOn {
    /// The camera drifts.
    #[default]
    Position,
    /// The point it looks at drifts, and the camera turns to follow.
    Objective,
}

impl WaveOn {
    fn is_position(&self) -> bool {
        *self == WaveOn::Position
    }
}

impl Transition {
    fn is_cut(&self) -> bool {
        *self == Transition::Cut
    }
}

/// Something done once, at `time` (s into its animation).
#[derive(Deserialize, Serialize, Clone, PartialEq, Debug)]
pub struct AnimEvent {
    pub time: f32,
    pub action: AnimAction,
}

#[derive(Deserialize, Serialize, Clone, PartialEq, Debug)]
pub enum AnimAction {
    /// Plays the audio in this folder (its audio.ogg).
    PlayAudio(String),
    /// Shows a subtitle line for `seconds`.
    Subtitle { text: String, seconds: f32 },
    /// Starts node `node`'s animation `animation` from its start.
    StartAnimation { node: String, animation: String },
    /// Stops it.
    StopAnimation { node: String, animation: String },
}

impl AnimAction {
    pub fn describe(&self) -> String {
        match self {
            AnimAction::PlayAudio(folder) => format!("Audio: {folder}"),
            AnimAction::Subtitle { text, .. } => format!("Subtitle: {text}"),
            AnimAction::StartAnimation { node, animation } => format!("Start {node} › {animation}"),
            AnimAction::StopAnimation { node, animation } => format!("Stop {node} › {animation}"),
        }
    }
}

/// How a path is followed.
#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum FollowMode {
    /// Placed along it by time, physics held off - exact timing.
    #[default]
    #[serde(alias = "Animated")]
    Positioned,
    /// Flown along it by its autopilot, for real - only an aircraft.
    Flown,
}

impl FollowMode {
    fn is_positioned(&self) -> bool {
        *self == FollowMode::Positioned
    }

    pub fn label(self) -> &'static str {
        match self {
            FollowMode::Positioned => "Positioned",
            FollowMode::Flown => "Flown",
        }
    }
}

/// How fast a path's followed.
#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Debug, Default)]
pub enum Pace {
    /// At this speed, knots.
    Speed(f32),
    /// Timed: over the whole path in this many seconds from the follow's
    /// start (once round, for a looped one) - "start here at its start, be
    /// at the end this long after", placed by time whatever the physics.
    Duration(f32),
    /// As fast as the pilot's skill flies (see play::pilot_skill) - an Ace
    /// at full power.
    #[default]
    BySkill,
}

impl Pace {
    pub fn label(self) -> &'static str {
        match self {
            Pace::Speed(_) => "Speed",
            Pace::Duration(_) => "Timed",
            Pace::BySkill => "By skill",
        }
    }
}

/// A node's own path, followed (animated) from the level's start - see
/// `NodeSpec::follow`.
#[derive(Deserialize, Serialize, Clone, PartialEq, Debug)]
pub struct DefaultFollow {
    pub path: String,
    #[serde(default)]
    pub pace: Pace,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub orient: bool,
}

/// A node the level adds besides its aircraft - `map.ron`'s `nodes: [ ... ]`.
#[derive(Deserialize, Serialize, Clone, PartialEq, Debug)]
pub struct NodeSpec {
    /// Its node id - unique across the map, aircraft names included.
    pub id: String,
    /// The node it's a child of (an aircraft, another of these, or any node
    /// the level spawns). Optional - at the top when left out.
    #[serde(default, skip_serializing_if = "Option::is_none", serialize_with = "plain_some")]
    pub parent: Option<String>,
    /// With a parent: `Local` (an offset from the parent's origin, moving
    /// with it - the default) or `Global` (a place in the world,
    /// disconnected from it).
    #[serde(default, skip_serializing_if = "is_local")]
    pub space: Space,
    /// A `Local` child turns with its parent - false: it only follows its
    /// position. Optional - true when left out.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub follow_rotation: bool,
    /// Meters - from the parent's origin for a `Local` child, in the world
    /// otherwise.
    pub position: (f32, f32, f32),
    /// Degrees about X, Y and Z - the same space as `position`. Optional.
    #[serde(default, skip_serializing_if = "is_zero_rotation")]
    pub rotation: (f32, f32, f32),
    /// Makes it a camera. Optional.
    #[serde(default, skip_serializing_if = "Option::is_none", serialize_with = "plain_some")]
    pub camera: Option<CameraSpec>,
    /// Keeps it turned to face this node. Optional.
    #[serde(default, skip_serializing_if = "Option::is_none", serialize_with = "plain_some")]
    pub look_at: Option<String>,
    /// A path it follows from the level's start (animated), whenever no
    /// sequence has it following another. Optional.
    #[serde(default, skip_serializing_if = "Option::is_none", serialize_with = "plain_some")]
    pub follow: Option<DefaultFollow>,
}

/// A camera node's lens - see `NodeSpec::camera`.
#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Debug)]
pub struct CameraSpec {
    /// Vertical field of view, degrees.
    pub fov: f32,
    /// Its near plane, meters. Optional.
    #[serde(default = "default_near")]
    pub near: f32,
}

fn default_near() -> f32 {
    0.1
}

fn is_local(space: &Space) -> bool {
    *space == Space::Local
}

impl NodeSpec {
    /// Whether it's placed from its parent (a `Local` child) - its
    /// position and rotation are then an offset from it.
    pub fn is_local(&self) -> bool {
        self.parent.is_some() && self.space == Space::Local
    }

    pub fn position(&self) -> Vector3<f32> {
        Vector3::new(self.position.0, self.position.1, self.position.2)
    }

    pub fn rotation(&self) -> UnitQuaternion<f32> {
        let (x, y, z) = self.rotation;
        UnitQuaternion::from_euler_angles(x.to_radians(), y.to_radians(), z.to_radians())
    }
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
    /// Optional. A bot flies it (see plane::autopilot). Its own path - see
    /// `follow` for flying one of the map's paths instead.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<(f32, f32, f32)>,
    /// The map path it flies (one of `MapSpec::paths`, by name) instead of
    /// its own `path`. Optional.
    #[serde(default, skip_serializing_if = "Option::is_none", serialize_with = "plain_some")]
    pub follow: Option<String>,
    /// What it carries: a weapon on each of its plane's pylons, by pylon
    /// name - `{ "1 Left wingtip": "missiles/aim-9x" }`. Optional: nothing
    /// when left out. The pylons themselves are the plane's (its data.ron's
    /// `pylons`); this is how this aircraft is armed for the mission.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub loadout: BTreeMap<String, String>,
    /// How many flares and chaff it's loaded with. Optional: as many as its
    /// plane's data.ron `countermeasures` says when left out.
    #[serde(default, skip_serializing_if = "Option::is_none", serialize_with = "plain_some")]
    pub flares: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none", serialize_with = "plain_some")]
    pub chaff: Option<u32>,
    /// Fuel in the tanks at spawn (kg). Optional: full when left out.
    #[serde(default, skip_serializing_if = "Option::is_none", serialize_with = "plain_some")]
    pub fuel_kg: Option<f32>,
    /// Bingo fuel (kg). Optional: none when left out. The aircraft-data box
    /// marks the fuel line once what's left is at or under this.
    #[serde(default, skip_serializing_if = "Option::is_none", serialize_with = "plain_some")]
    pub bingo_kg: Option<f32>,
    /// Radar altitude (m, straight down) that starts the HUD's low-altitude
    /// warning. Optional: `DEFAULT_RADAR_WARN_M` when left out.
    #[serde(default, skip_serializing_if = "Option::is_none", serialize_with = "plain_some")]
    pub radar_warn_m: Option<f32>,
    /// How good a pilot a bot is - how quickly it reacts (see
    /// play::ai_defense). Optional: Medium when left out. Nothing for the
    /// player's.
    #[serde(default, skip_serializing_if = "is_default_difficulty")]
    pub difficulty: Difficulty,
    /// How it spawns. Ground: sitting, gear down. Air: flying, gear up.
    /// Optional - ground when left out.
    #[serde(default, skip_serializing_if = "Start::is_ground")]
    pub start: Start,
}

/// Radar altitude (m) that starts the low-altitude warning when a placement
/// doesn't set its own.
pub const DEFAULT_RADAR_WARN_M: f32 = 150.0;

/// How an aircraft is put into the level - see `AircraftPlacement::start`.
#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Start {
    /// Sitting still, landing gear down.
    #[default]
    Ground,
    /// Already flying, landing gear up.
    Air,
}

impl Start {
    fn is_ground(&self) -> bool {
        *self == Start::Ground
    }

    /// The gear is down and locked at spawn.
    pub fn gear_down(self) -> bool {
        self == Start::Ground
    }

    pub fn label(self) -> &'static str {
        match self {
            Start::Ground => "Ground",
            Start::Air => "Air",
        }
    }
}

/// A bot's skill - see `AircraftPlacement::difficulty`.
#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Difficulty {
    Beginner,
    #[default]
    Medium,
    Ace,
}

impl Difficulty {
    pub const ALL: [Difficulty; 3] = [Difficulty::Beginner, Difficulty::Medium, Difficulty::Ace];

    pub fn label(self) -> &'static str {
        match self {
            Difficulty::Beginner => "Beginner",
            Difficulty::Medium => "Medium",
            Difficulty::Ace => "Ace",
        }
    }
}

fn is_default_difficulty(value: &Difficulty) -> bool {
    *value == Difficulty::default()
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn default_wave_amount() -> f32 {
    5.0
}

fn is_default_wave_amount(value: &f32) -> bool {
    *value == default_wave_amount()
}

fn default_wave_speed() -> f32 {
    1.0
}

fn is_default_wave_speed(value: &f32) -> bool {
    *value == default_wave_speed()
}

fn default_true() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

fn is_zero(value: &f32) -> bool {
    *value == 0.0
}

fn is_zero_rotation(value: &(f32, f32, f32)) -> bool {
    *value == (0.0, 0.0, 0.0)
}

/// An optional field written plainly (`speed: 120.0`), the way the file is
/// read - without RON's `Some( ... )` around it.
fn plain_some<T: Serialize, S: serde::Serializer>(value: &Option<T>, serializer: S) -> Result<S::Ok, S::Error> {
    match value {
        Some(value) => value.serialize(serializer),
        None => serializer.serialize_none(),
    }
}

/// What `save` writes above the data - the format, for whoever edits the file
/// by hand.
const FILE_HEADER: &str = "\
// The level's starting setup - its coalitions and every aircraft in each (see
// play::map::MapSpec). Read each time the level opens, so an edit here shows
// up on the next restart (pause menu > Restart) - or edit it live in game with
// the flight editor (F10's editor view), which saves back to this file.
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
//               through, in order - a bot flies them.
//   follow    - optional, the name of one of the map's paths it flies
//               instead of its own `path`.
//   loadout   - optional, what it carries: { \"<pylon name>\": \"<weapon>\" } -
//               the pylons are its plane's (data.ron `pylons`), the weapons
//               folders under assets/weapons (\"missiles/aim-9x\").
//   flares, chaff - optional, how many it's loaded with - as many as its
//               plane's data.ron `countermeasures` says when left out.
//   difficulty - optional, a bot's skill: Beginner, Medium (when left out) or
//               Ace - how quickly it reacts to a missile with flares.
//   start     - optional, Ground (when left out: sitting, gear down) or Air
//               (flying, gear up).
//
// nodes - optional, every other node the level adds (cameras, mostly):
//   id        - its node id. Unique across the map, aircraft names included.
//   parent    - optional, the node it's a child of.
//   space     - optional, with a parent: Local (an offset from the parent's
//               origin, moving with it - the default) or Global (a place in
//               the world, disconnected from it).
//   follow_rotation - optional, false: a Local child only follows its
//               parent's position, not its turning.
//   position  - meters (from the parent for Local, in the world otherwise).
//   rotation  - optional, degrees about X, Y, Z.
//   camera    - optional, makes it a camera: (fov: degrees, near: meters).
//   look_at   - optional, a node it keeps facing.
//   follow    - optional, a path it follows from the start:
//               (path: \"name\", pace: Speed(knots) | Duration(seconds) | BySkill).
//
// paths - optional, named paths: (name, points: [(x, y, z), ...], looped -
// optional, round and round (the default) or once, smooth - optional, a
// curve through the points when followed).
//
// animations - optional, every node's animations by node id (edit them in
// the editor: the node's Animations in Properties, then the timeline), each:
// name, length (s), on_start, looped, lock_input, hide_hud, paths - (start,
// end, path: \"name\", mode: Positioned | Flown, speed_kt, skill, orient) -
// camera - keys (time, transition: Cut | Smooth, position, rotation, fov,
// follow, follow_rotation - optional, on: the offset turns with what it
// follows, look_at) - and events (time, action: PlayAudio | Subtitle |
// StartAnimation | StopAnimation).
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
        for node in &self.nodes {
            if node.id.is_empty() || node.id.contains('/') {
                return Err(format!("node id '{}' can't be empty or have a '/' in it", node.id));
            }
            if !names.insert(node.id.as_str()) {
                return Err(format!("the name '{}' is used twice (nodes and aircraft share names)", node.id));
            }
            if node.parent.as_deref() == Some(node.id.as_str()) {
                return Err(format!("node '{}' can't be its own parent", node.id));
            }
        }
        let mut path_names = HashSet::new();
        for path in &self.paths {
            if path.name.is_empty() || !path_names.insert(path.name.as_str()) {
                return Err(format!("path name '{}' is empty or used twice", path.name));
            }
        }
        for (_, aircraft) in self.aircraft() {
            if let Some(follow) = aircraft.follow.as_deref().filter(|follow| !path_names.contains(follow)) {
                return Err(format!("aircraft '{}' follows the path '{follow}', which isn't in the map", aircraft.name));
            }
        }
        for (node, animations) in &self.animations {
            let mut animation_names = HashSet::new();
            for animation in animations {
                if animation.name.is_empty() || !animation_names.insert(animation.name.as_str()) {
                    return Err(format!("'{node}' has an animation named '{}' - empty or used twice", animation.name));
                }
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

    /// The path named `name`.
    pub fn named_path(&self, name: &str) -> Option<&PathSpec> {
        self.paths.iter().find(|path| path.name == name)
    }

    pub fn named_path_mut(&mut self, name: &str) -> Option<&mut PathSpec> {
        self.paths.iter_mut().find(|path| path.name == name)
    }

    /// Node `node`'s animation `name`.
    pub fn animation(&self, node: &str, name: &str) -> Option<&AnimationDef> {
        self.animations.get(node)?.iter().find(|animation| animation.name == name)
    }

    pub fn animation_mut(&mut self, node: &str, name: &str) -> Option<&mut AnimationDef> {
        self.animations.get_mut(node)?.iter_mut().find(|animation| animation.name == name)
    }

    /// The points `placement` flies by default, and whether round and round
    /// - the map path it follows, or its own path.
    pub fn flight_plan(&self, placement: &AircraftPlacement) -> (Vec<(f32, f32, f32)>, bool) {
        match placement.follow.as_deref().and_then(|name| self.named_path(name)) {
            Some(path) => (path.points.clone(), path.looped),
            None => (placement.path.clone(), true),
        }
    }

    /// Changes the points the aircraft `name` flies by default - the map
    /// path it follows (every aircraft on it changes too), or its own.
    pub fn set_flight_plan(&mut self, name: &str, points: Vec<(f32, f32, f32)>) {
        let follow = self.find(name).and_then(|(_, placement)| placement.follow.clone());
        if let Some(path) = follow.as_deref().and_then(|follow| self.named_path_mut(follow)) {
            path.points = points;
            return;
        }
        if let Some(placement) = self.find_mut(name) {
            placement.path = points;
        }
    }

    /// The node named `id` (not an aircraft).
    pub fn node(&self, id: &str) -> Option<&NodeSpec> {
        self.nodes.iter().find(|node| node.id == id)
    }

    pub fn node_mut(&mut self, id: &str) -> Option<&mut NodeSpec> {
        self.nodes.iter_mut().find(|node| node.id == id)
    }

    /// Whether `name` is taken - by an aircraft or a node.
    pub fn name_taken(&self, name: &str) -> bool {
        self.find(name).is_some() || self.node(name).is_some()
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
        aircraft.loadout = BTreeMap::from([
            ("1 Left wingtip".to_owned(), "missiles/aim-9x".to_owned()),
            ("9 Right wingtip".to_owned(), "missiles/aim-9x".to_owned()),
        ]);
        // A node and a timeline too.
        let player = aircraft.name.clone();
        map.nodes.push(NodeSpec {
            id: "chase_cam".to_owned(),
            parent: Some(player.clone()),
            space: Space::Global,
            follow_rotation: false,
            position: (1.0, 2.0, 3.0),
            rotation: (0.0, 45.0, 0.0),
            camera: Some(CameraSpec { fov: 55.0, near: 0.2 }),
            look_at: Some(player.clone()),
            follow: Some(DefaultFollow { path: "patrol".to_owned(), pace: Pace::Speed(80.0), orient: false }),
        });
        map.paths.push(PathSpec { name: "patrol".to_owned(), points: vec![(0.0, 900.0, 0.0), (5000.0, 1100.0, 3000.0)], looped: false, smooth: true });
        map.coalitions[0].aircraft[0].follow = Some("patrol".to_owned());
        let mut intro = AnimationDef::new("intro");
        intro.length = 12.0;
        intro.on_start = true;
        intro.looped = true;
        intro.hide_hud = false;
        intro.paths.push(PathClip { start: 0.5, end: 9.0, path: "patrol".to_owned(), mode: FollowMode::Flown, speed_kt: Some(350.0), skill: Some(Difficulty::Ace), orient: true });
        intro.paths.push(PathClip { start: 9.0, end: 11.0, path: "patrol".to_owned(), mode: FollowMode::Positioned, speed_kt: None, skill: None, orient: false });
        intro.camera.push(CameraKey { time: 0.0, transition: Transition::Cut, position: (0.0, 4.0, -25.0), rotation: (0.0, 0.0, 0.0), fov: 60.0, follow: Some(player.clone()), follow_rotation: true, look_at: None, wave: false, wave_amount: 5.0, wave_speed: 1.0, wave_on: WaveOn::Position });
        intro.camera.push(CameraKey { time: 4.0, transition: Transition::Smooth, position: (100.0, 1200.0, 50.0), rotation: (5.0, 90.0, 0.0), fov: 35.0, follow: None, follow_rotation: true, look_at: Some(player.clone()), wave: false, wave_amount: 5.0, wave_speed: 1.0, wave_on: WaveOn::Position });
        intro.events.push(AnimEvent { time: 2.0, action: AnimAction::PlayAudio("./assets/audio/ReasonToFight".to_owned()) });
        intro.events.push(AnimEvent { time: 3.0, action: AnimAction::StartAnimation { node: "chase_cam".to_owned(), animation: "then".to_owned() } });
        map.animations.insert(player.clone(), vec![intro]);
        map.animations.insert("chase_cam".to_owned(), vec![AnimationDef::new("then")]);

        let folder = std::env::temp_dir().join("project_skies_map_round_trip");
        std::fs::create_dir_all(&folder).unwrap();
        let folder = folder.to_string_lossy().into_owned();
        map.save(&folder).unwrap();
        let read_back = MapSpec::load(&folder).unwrap();
        assert!(read_back == map, "{}", std::fs::read_to_string(MapSpec::path(&folder)).unwrap());
    }
}
