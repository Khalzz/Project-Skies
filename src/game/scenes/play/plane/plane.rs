use std::collections::{HashMap, HashSet};

use nalgebra::{UnitQuaternion, Vector3};

use crate::app::App;
use crate::engine::input::input;
use crate::engine::physics::water_contact::WaterContactState;
use crate::engine::rendering::render_pipeline::render_pass::AeroSample;
use crate::engine::rendering::camera::handler::SceneCameras;
use crate::engine::scene_manager::behavior::Behavior;
use crate::engine::scene_manager::node::Node;
use crate::engine::scene_manager::properties::{Model as ModelProperty, Transform3D};
use crate::engine::utils::lerps::lerp;
use crate::game::scenes::play::crash_effects::Crash;
use crate::transform::Transform;

use super::afterburner::Afterburner;
use super::fuel::Fuel;
use super::control_surfaces::{ControlInput, ControlSurface, SurfaceBase};
use super::controls::PlaneControls;
use super::fcs::Fcs;
use super::landing_gear::GearMeshes;
use super::messages::{AircraftEvent, AircraftState};
use super::effects::{self, EffectKind, EffectSpec};
use super::pilot::Pilot;
use crate::engine::particles::ParticleEmitters;
use crate::engine::game_nodes::game_object::ColliderType;
use crate::engine::physics::physics_handler::RenderMessage;
use super::autopilot::{Autopilot, Speed};
use crate::game::scenes::play::map::Difficulty;
use super::quick_list::QuickList;
use super::quick_menu::{GearStatus, QuickAction, QuickMenuStatus};

/// Damage one warhead dealt to one surface, waiting for the kill feed.
pub struct PartHit {
    pub shooter: String,
    pub weapon: String,
    /// The surface's label, as in the plane's data ("Left wing").
    pub part: String,
    /// HP that surface lost.
    pub dealt: f32,
    /// The surface's HP hit 0.
    pub destroyed: bool,
}

/// How long after an animation lets an aircraft go its pilot still feels
/// no G (s) - long enough for the physics to settle it from being placed
/// (or teleported) into flying at speed.
const CALM_GRACE_SECONDS: f32 = 2.5;

/// An aircraft's main-thread half - reads the pilot's input, sends it to the
/// physics half (`AircraftPhysics`), and animates the model from whatever
/// the physics half reports back (`AircraftState`). Owns no physical state
/// itself - see messages.rs for everything the two halves exchange.
pub struct Plane {
    pub controls: PlaneControls,
    /// Flown by the player: reads the stick/keys and drives the quick menu
    /// and the F7 overlay. A bot (false) does none of that - its controls
    /// stay wherever they were set (see `bot`, `with_throttle`).
    pub is_player: bool,
    /// A bot's pilot - flies its path (see `set_path`). Unused while it's
    /// the player's.
    autopilot: Option<Autopilot>,
    /// Where the throttle lever was when it last set the throttle (see
    /// read_pilot_input) - `None` without one connected.
    throttle_lever: Option<f32>,
    /// Where the aircraft is and how it's turned, as physics last reported
    /// (see `fixed_update`) - what the autopilot steers from.
    pose: Option<(Vector3<f32>, UnitQuaternion<f32>)>,
    pub input_locked: bool,
    /// Its autopilot flies it even when it's the player's - a timeline has
    /// it fly a path (see play::path_follow).
    pub scripted: bool,
    /// An animation's placing it (its body held still - see
    /// SceneContent::kinematic): nothing the physics says about it counts
    /// meanwhile - no crash, no G on the pilot, no controls.
    pub held: bool,
    /// An animation's playing on it: its pilot feels no G meanwhile.
    pub calm: bool,
    /// Who moved its controls last frame, and (read from the pilot's input)
    /// the stick it was easing toward - for the editor's Status.
    pub flown_by: &'static str,
    pub stick_target: f32,
    /// Seconds its pilot still feels no G after an animation let it go (see
    /// CALM_GRACE_SECONDS) - the jump from being placed to flying again
    /// isn't a G the pilot pulled.
    calm_left: f32,
    /// Hit the water - see `check_wreck`. Never cleared (a restart builds
    /// a new plane).
    pub wrecked: bool,
    /// Hit by a missile - wrecked on its next update (see `check_wreck`).
    pub shot_down: bool,
    /// Shot down and still in the air - its wings flying it down until it
    /// hits the water or the ground.
    going_down: bool,
    /// In the sea - its wreck no longer smokes.
    in_water: bool,
    /// How settled the wreck is: 0 still moving .. 1 at rest - eased, see
    /// `update_effects`. At rest it smokes less.
    wreck_rest: f32,
    /// How it just ended, for the scene to show (see play::crash_effects) -
    /// taken from here once it has.
    pub crash: Option<Crash>,
    /// The pilot's G tolerance - grey/black/red-out, passing out.
    pub pilot: Pilot,
    /// Wingtip vortex strength 0..1, eased - see update_effects.
    vortex_strength: f32,
    /// The node's emitters from the plane's data.ron, by name - what each
    /// one is, so update_effects knows how to drive it.
    effects: Vec<(String, EffectKind)>,
    /// Each wheel's ground contact last frame, by mesh - for spotting
    /// touchdowns (tyre smoke).
    wheels_grounded: HashMap<String, bool>,
    /// Seconds of tyre smoke left per wheel, by mesh.
    tyre_smoke: HashMap<String, f32>,
    pub fcs: Fcs,
    control_surfaces: Vec<ControlSurface>,
    afterburner: Afterburner,
    /// The engine's afterburner gate - military power, and where the
    /// autopilot holds the lever when the speed is right (1: no afterburner).
    military: f32,
    // The afterburner mesh's own scale from the model, captured the first
    // frame - its length is stretched by `afterburner.value` on top of it.
    afterburner_base_scale: Option<Vector3<f32>>,
    /// Internal tanks - see `fuel`.
    fuel: Fuel,
    /// Fuel left at which the aircraft-data box marks bingo (kg). None: no
    /// bingo set.
    pub bingo_kg: Option<f32>,
    /// Radar altitude (m, straight down) at which the HUD's low-altitude
    /// warning starts. See `map::DEFAULT_RADAR_WARN_M`.
    pub radar_warn_m: f32,
    gear_meshes: GearMeshes,
    quick_menu: QuickList,
    /// Fuselage and wing colliders, for a missile's fuze. The shapes the
    /// plane loaded with (and a data.ron reload).
    colliders: Vec<ColliderType>,
    /// A blast already sent, whose result a later physics state answers.
    /// Kept until that answer arrives: the physics thread can be several
    /// frames behind, and an empty frame must not drop the credit.
    inflight_blast: Option<(String, String)>,
    /// A blast sent this frame, not the one the current state answers.
    queued_blast: Option<(String, String)>,
    /// A pilot kill the scene hasn't shown in the feed yet: who fired,
    /// with what.
    pub kill_report: Option<(String, String)>,
    /// Part damage the feed hasn't shown, from the latest warhead: who
    /// fired, with what, which surface, how much HP it lost, and whether
    /// that emptied it.
    part_hits: Vec<PartHit>,
    /// Wing labels already reported destroyed.
    lost_wings: HashSet<String>,
    /// Last published HP per surface, so a new blast's drop can be measured.
    last_profile_hp: HashMap<String, f32>,
    /// `AircraftState::blasts_handled` already accounted for.
    reported_blasts: u32,
    /// Each lifting surface's full HP, so damage smoke scales off what's
    /// left (see `update_effects`). From the plane's data.ron.
    surface_max: HashMap<String, f32>,
}

/// Tyre smoke: from this ground speed at touchdown (m/s, ~50 kt) - slower
/// and the tyre spins up without smoking - full by TYRE_SMOKE_FULL_SPEED,
/// lasting TYRE_SMOKE_SECONDS.
const TYRE_SMOKE_MIN_SPEED: f32 = 25.0;
const TYRE_SMOKE_FULL_SPEED: f32 = 75.0;
const TYRE_SMOKE_SECONDS: f32 = 0.3;
/// A wreck slower than this (m/s) has come to rest - it smokes at
/// WRECK_REST_SMOKE of the rate it did moving. Eased in and out at
/// WRECK_REST_RATE (per second).
const WRECK_REST_SPEED: f32 = 2.0;
const WRECK_REST_SMOKE: f32 = 0.35;
const WRECK_REST_RATE: f32 = 1.5;

impl Plane {
    // Cap on App::aero_debug_trail/App::wing_lift_trail (see those fields'
    // own doc comments) - oldest sample drops once this is exceeded, so the
    // F7 overlay reads as a "recent history" trail rather than growing
    // forever while it's on.
    const AERO_DEBUG_TRAIL_CAPACITY: usize = 500;
    // How much history App::pitch_rate_trail keeps (s).
    const PITCH_RATE_TRAIL_SECONDS: f32 = 10.0;

    /// Wrecked in the sea - see `in_water`.
    pub fn in_water(&self) -> bool {
        self.in_water
    }

    /// Shot down and still in the air - see `going_down`.
    pub fn going_down(&self) -> bool {
        self.going_down
    }

    pub fn new() -> Self {
        Self {
            controls: PlaneControls::new(),
            is_player: true,
            autopilot: None,
            throttle_lever: None,
            pose: None,
            input_locked: false,
            scripted: false,
            held: false,
            calm: false,
            flown_by: "",
            stick_target: 0.0,
            calm_left: 0.0,
            wrecked: false,
            shot_down: false,
            going_down: false,
            in_water: false,
            wreck_rest: 0.0,
            crash: None,
            pilot: Pilot::new(),
            vortex_strength: 0.0,
            effects: Vec::new(),
            wheels_grounded: HashMap::new(),
            tyre_smoke: HashMap::new(),
            fcs: Fcs::new(),
            control_surfaces: Self::default_control_surfaces(),
            afterburner: Afterburner::new(),
            military: 1.0,
            afterburner_base_scale: None,
            fuel: Fuel::default(),
            bingo_kg: None,
            radar_warn_m: crate::game::scenes::play::map::DEFAULT_RADAR_WARN_M,
            gear_meshes: GearMeshes::new(),
            quick_menu: QuickList::new(),
            colliders: Vec::new(),
            inflight_blast: None,
            queued_blast: None,
            kill_report: None,
            part_hits: Vec::new(),
            lost_wings: HashSet::new(),
            last_profile_hp: HashMap::new(),
            reported_blasts: 0,
            surface_max: HashMap::new(),
        }
    }

    /// Full HP of each lifting surface, for the damage smoke and fire.
    /// Spawn, and a data.ron reload.
    pub fn set_surface_max(&mut self, wings: &[super::aero_spec::WingSpec]) {
        self.surface_max = wings.iter().map(|wing| (wing.label.clone(), wing.hp.max(1.0))).collect();
        // So the first warhead can be measured even before a physics state
        // has published the undamaged HP.
        if self.last_profile_hp.is_empty() {
            self.last_profile_hp.clone_from(&self.surface_max);
        }
    }

    /// The colliders a missile's fuze tests - see `set_colliders`.
    pub fn fuze_colliders(&self) -> &[ColliderType] {
        &self.colliders
    }

    /// Replaces the fuze colliders (spawn, or a data.ron reload).
    pub fn set_colliders(&mut self, colliders: Vec<ColliderType>) {
        self.colliders = colliders;
    }

    /// A warhead from `shooter` with `weapon` was sent to this aircraft.
    /// The first one waiting on the physics state is the one a pilot kill
    /// is credited to.
    pub fn note_blast(&mut self, shooter: &str, weapon: &str) {
        self.queued_blast = Some((shooter.to_owned(), weapon.to_owned()));
    }

    /// The pilot kill the feed hasn't shown, if there is one.
    pub fn take_kill_report(&mut self) -> Option<(String, String)> {
        self.kill_report.take()
    }

    /// Part damage the feed hasn't shown, from warheads since last time.
    pub fn take_part_hits(&mut self) -> Vec<PartHit> {
        std::mem::take(&mut self.part_hits)
    }

    /// A published blast result: kill the pilot when the seat was inside
    /// it, record how much HP each surface lost to that warhead, and log
    /// each wing whose HP just hit 0. Then the blast sent this frame waits
    /// for the next state.
    fn read_blast_result(&mut self, node: &Node) {
        if let Some(state) = node.physics_state::<AircraftState>() {
            if state.pilot_hit && !self.pilot.is_dead() {
                self.pilot.kill();
                // The aircraft is a kill the moment the pilot dies, even
                // while the airframe is still in the air.
                self.kill_report = Some(
                    self.inflight_blast.clone()
                        .or_else(|| self.queued_blast.clone())
                        .unwrap_or_else(|| ("Unknown".to_owned(), "blast".to_owned())),
                );
            }
            // Only a new warhead. A ground strike or an F10 toggle changes
            // HP without advancing this, and those are not a hit to credit.
            if state.blasts_handled > self.reported_blasts {
                self.reported_blasts = state.blasts_handled;
                if let Some((shooter, weapon)) = self.inflight_blast.clone().or_else(|| self.queued_blast.clone()) {
                    for (label, hp) in &state.profiles {
                        let Some(before) = self.last_profile_hp.get(label).copied() else { continue };
                        let dealt = before - *hp;
                        if dealt > 0.5 {
                            self.part_hits.push(PartHit {
                                shooter: shooter.clone(),
                                weapon: weapon.clone(),
                                part: label.clone(),
                                dealt,
                                destroyed: *hp <= 0.0,
                            });
                        }
                    }
                }
            }
            for (label, hp) in &state.profiles {
                if *hp <= 0.0 && self.lost_wings.insert(label.clone()) {
                    println!("{}: {label} destroyed", node.id);
                }
                self.last_profile_hp.insert(label.clone(), *hp);
            }
        }
        if let Some(blast) = self.queued_blast.take() {
            self.inflight_blast = Some(blast);
        }
    }

    /// Not flown by the player - see `is_player`.
    pub fn bot(mut self) -> Self {
        self.is_player = false;
        self
    }

    /// Gives a bot a path to fly (see autopilot) at `speed`, flown as well
    /// as a `skill` pilot flies, from its first point - round and round when
    /// `looped`, once otherwise - or, empty, none (it flies on as it is).
    pub fn set_path(&mut self, path: &[(f32, f32, f32)], speed: Speed, skill: Difficulty, looped: bool) {
        self.autopilot = (!path.is_empty()).then(|| Autopilot::new(path, speed, skill, looped).with_military(self.military));
    }

    /// The point of its path a bot is flying to - an index into its path.
    pub fn next_path_point(&self) -> Option<usize> {
        self.autopilot.as_ref().and_then(Autopilot::next_point)
    }

    /// The path its autopilot's flying - its points, which it's flying to,
    /// and whether round and round - while it's flying one.
    pub fn route(&self) -> Option<(Vec<Vector3<f32>>, Option<usize>, bool)> {
        if self.is_player && !self.scripted {
            return None;
        }
        let (points, next, looped) = self.autopilot.as_ref()?.route();
        (!points.is_empty()).then(|| (points.to_vec(), next, looped))
    }

    /// Whether its pilot feels the G it pulls - not while an animation has
    /// it (placed, or playing with it), nor for a moment after (see
    /// CALM_GRACE_SECONDS).
    pub fn feels_g(&self) -> bool {
        !self.held && !self.calm && self.calm_left <= 0.0
    }

    /// Where the point its autopilot's flying to is, while it's flying one
    /// (a bot, or anything an animation has flying a path).
    pub fn waypoint(&self) -> Option<Vector3<f32>> {
        if self.is_player && !self.scripted {
            return None;
        }
        self.autopilot.as_ref().and_then(Autopilot::waypoint)
    }

    /// Starts with the tanks `spec` describes, full.
    pub fn with_fuel(mut self, spec: &super::fuel::FuelSpec) -> Self {
        self.fuel = Fuel::from_spec(spec);
        self
    }

    /// A fuel spec edited while flying.
    pub fn set_fuel_spec(&mut self, spec: &super::fuel::FuelSpec) {
        self.fuel.apply_spec(spec);
    }

    /// Puts `kg` in the tanks, up to their maximum. Replaces what's there
    /// (a loadout's fuel setting), unlike `add_fuel`.
    pub fn set_fuel_kg(&mut self, kg: f32) {
        self.fuel.set_kg(kg);
    }

    /// Puts `kg` in the tanks, up to their maximum.
    pub fn add_fuel(&mut self, kg: f32) {
        self.fuel.add(kg);
    }

    /// Fuel left, and the tanks' maximum (kg).
    pub fn fuel(&self) -> (f32, f32) {
        (self.fuel.kg, self.fuel.capacity_kg)
    }

    /// Starts with the throttle at `throttle` (0..1).
    pub fn with_throttle(mut self, throttle: f32) -> Self {
        self.controls.throttle = throttle.clamp(0.0, 1.0);
        self
    }

    /// Military power is `gate` - the engine's afterburner gate (1 when it
    /// has none). The autopilot holds the lever there when the speed is right.
    pub fn with_military(mut self, gate: f32) -> Self {
        self.military = gate.clamp(0.01, 1.0);
        self
    }

    /// The wheels show `down` until physics reports the gear.
    pub fn arm_gear(&mut self, gear: &super::gear_spec::GearSpec, down: bool) {
        self.gear_meshes.arm(gear, down);
    }

    /// Where the wheels go with the gear up, from `gear` (data.ron edited
    /// mid-flight).
    pub fn set_gear_look(&mut self, gear: &super::gear_spec::GearSpec) {
        self.gear_meshes.set_gear(gear);
    }

    /// What each of the node's emitters is - the plane's data.ron effects.
    pub fn with_effects(mut self, effects: &[EffectSpec]) -> Self {
        self.set_effects(effects);
        self
    }

    /// See with_effects - also when the data.ron is edited mid-flight.
    pub fn set_effects(&mut self, effects: &[EffectSpec]) {
        self.effects = effects.iter().map(|spec| (spec.name.clone(), spec.effect)).collect();
    }

    fn default_control_surfaces() -> Vec<ControlSurface> {
        let rudder_base = UnitQuaternion::from_axis_angle(&Vector3::x_axis(), -28.4493_f32.to_radians());
        vec![
            ControlSurface::new("opaque", "left_elevator", *Vector3::x_axis(), 0.15, 7.0, SurfaceBase::None, ControlInput::Elevator),
            ControlSurface::new("opaque", "right_elevator", *Vector3::x_axis(), 0.15, 7.0, SurfaceBase::None, ControlInput::Elevator),
            ControlSurface::new("opaque", "left_aleron", *Vector3::x_axis(), -0.5, 7.0, SurfaceBase::Captured(None), ControlInput::Aileron),
            ControlSurface::new("opaque", "right_aleron", *Vector3::x_axis(), 0.5, 7.0, SurfaceBase::Captured(None), ControlInput::Aileron),
            ControlSurface::new("opaque", "rudder_0", *Vector3::y_axis(), 0.5, 7.0, SurfaceBase::Fixed(rudder_base), ControlInput::Rudder),
            ControlSurface::new("opaque", "rudder_1", *Vector3::y_axis(), 0.5, 7.0, SurfaceBase::Fixed(rudder_base), ControlInput::Rudder),
        ]
    }

    /// Stick/throttle/trim into `controls`, toggles into `fcs`, and one-shot
    /// gear commands straight to the physics half as events.
    fn read_pilot_input(&mut self, node: &mut Node, delta_time: f32) {
        const STICK_INPUT_LERP_SPEED: f32 = 6.0;
        let target_elevator = input::get_axis("pitch_down", "pitch_up");
        let target_aileron = input::get_axis("roll_left", "roll_right");
        let target_rudder = input::get_axis("rudder_left", "rudder_right");

        let target_elevator = if self.fcs.pitch_autotrim {
            target_elevator
        } else {
            target_elevator + self.controls.trim.pitch
        };

        let target_aileron = target_aileron + self.controls.trim.roll;

        self.stick_target = target_elevator;
        self.controls.elevator = lerp(self.controls.elevator, target_elevator, delta_time * STICK_INPUT_LERP_SPEED);
        self.controls.aileron = lerp(self.controls.aileron, target_aileron, delta_time * STICK_INPUT_LERP_SPEED);
        self.controls.rudder = lerp(self.controls.rudder, target_rudder, delta_time * STICK_INPUT_LERP_SPEED);

        const THROTTLE_RATE: f32 = 0.5;
        self.controls.throttle = (self.controls.throttle + input::get_axis("throttle_down", "throttle_up") * THROTTLE_RATE * delta_time).clamp(0.0, 1.0);
        // A throttle lever (a HOTAS throttle - "throttle_axis") sets the
        // throttle to where it is whenever it moves; the keys/triggers above
        // still nudge it while it doesn't.
        const LEVER_MOVED: f32 = 0.002;
        if let Some(lever) = input::absolute_value("throttle_axis") {
            if self.throttle_lever.is_none_or(|last| (last - lever).abs() > LEVER_MOVED) {
                self.controls.throttle = lever;
                self.throttle_lever = Some(lever);
            }
        } else {
            self.throttle_lever = None;
        }
        self.controls.trim.update(delta_time);

        if input::is_action_just_pressed("toggle_fbw_pitch") {
            self.apply_quick_action(node, QuickAction::ToggleFlyByWire);
        }

        self.controls.wheel_brake = input::action_strength("wheel_brake");
        if input::is_action_just_pressed("toggle_parking_brake") {
            self.apply_quick_action(node, QuickAction::ToggleParkingBrake);
        }

        if input::is_action_just_pressed("toggle_landing_gear") {
            self.apply_quick_action(node, QuickAction::ToggleGear);
        }
        if input::is_action_just_pressed("landing_gear_up") {
            node.push_physics_event(AircraftEvent::GearUp);
        }
        if input::is_action_just_pressed("landing_gear_down") {
            node.push_physics_event(AircraftEvent::GearDown);
        }
        if input::is_action_just_pressed("toggle_air_brake") {
            node.push_physics_event(AircraftEvent::ToggleSpeedBrake);
        }
        if input::is_action_just_pressed("air_brake_extend") {
            node.push_physics_event(AircraftEvent::SpeedBrakeExtend);
        }
        if input::is_action_just_pressed("air_brake_retract") {
            node.push_physics_event(AircraftEvent::SpeedBrakeRetract);
        }
    }

    /// Runs one of the plane's toggles - shared by the keyboard keys (see
    /// `read_pilot_input`) and the D-pad quick menu, so both always do the
    /// same thing.
    fn apply_quick_action(&mut self, node: &mut Node, action: QuickAction) {
        match action {
            QuickAction::ToggleGear => node.push_physics_event(AircraftEvent::ToggleGear),
            QuickAction::ToggleSpeedBrake => node.push_physics_event(AircraftEvent::ToggleSpeedBrake),
            QuickAction::ToggleParkingBrake => {
                self.controls.parking_brake = !self.controls.parking_brake;
                println!("Parking brake: {}", if self.controls.parking_brake { "SET" } else { "RELEASED" });
            }
            QuickAction::ToggleFlyByWire => {
                self.fcs.pitch_autotrim = !self.fcs.pitch_autotrim;
                println!(
                    "Pitch fly-by-wire (g-command): {}",
                    if self.fcs.pitch_autotrim { "ENGAGED - stick commands g" } else { "OFF - manual + trim" }
                );
            }
        }
    }

    /// The contextual aircraft list stays hidden. Gear, brakes and
    /// fly-by-wire stay on their own keys in `read_pilot_input`.
    fn update_quick_menu(&mut self, _node: &mut Node, app: &mut App) {
        let status = QuickMenuStatus {
            gear: GearStatus::Up,
            speed_brake: GearStatus::Up,
            parking_brake: false,
            fly_by_wire: false,
        };
        self.quick_menu.refresh_ui(app, &status, false);
    }

    /// Any part of the plane touching the water (see the node's
    /// `WaterContact`), the fuselage hitting the ground, a pilot killed by
    /// the impact, or `shot_down`, wrecks it: the physics half stops flying
    /// it, and the controls go neutral with the throttle closed and stay
    /// that way. A pilot killed by a blast stays in the air on the profiles
    /// that survived, until the fuselage hits the ground or the jet hits
    /// the water. A wing hitting the ground tears that wing off; it does
    /// not wreck the jet.
    fn check_wreck(&mut self, node: &mut Node) {
        let water_impact = node.physics_state::<WaterContactState>().and_then(|state| state.impact);
        // Going down after a missile hit: still flying until the water - or
        // the ground, hit hard enough to kill the pilot.
        if self.wrecked && self.going_down {
            if let Some(impact) = water_impact {
                println!("Went into the water at {:.0} m/s", impact.speed);
                self.end(node, Crash::Water);
            } else if self.pilot.is_dead() {
                println!("Went into the ground");
                self.end(node, Crash::Ground);
            }
            return;
        }
        if self.wrecked {
            return;
        }
        let event = if let Some(impact) = water_impact {
            println!("Crashed into the water at {:.0} m/s (collider {})", impact.speed, impact.collider);
            self.in_water = true;
            self.crash = Some(Crash::Water);
            AircraftEvent::Wreck
        } else if self.shot_down {
            println!("Shot down");
            self.going_down = true;
            AircraftEvent::ShotDown
        } else if node.physics_state::<AircraftState>().is_some_and(|state| state.fuselage_struck) {
            println!("Went into the ground");
            self.crash = Some(Crash::Ground);
            if !self.pilot.is_dead() {
                self.pilot.killed_in_crash();
            }
            AircraftEvent::Wreck
        } else if self.pilot.is_dead() && !self.pilot.killed_by_blast() {
            self.crash = Some(Crash::Ground);
            AircraftEvent::Wreck
        } else if self.pilot.killed_by_blast() && node.physics_state::<AircraftState>().is_some_and(|state| Pilot::impact_is_fatal(state.flight_data.impact_g)) {
            println!("Went into the ground");
            self.crash = Some(Crash::Ground);
            AircraftEvent::Wreck
        } else {
            return;
        };
        self.wrecked = true;
        self.controls = PlaneControls { parking_brake: self.controls.parking_brake, ..PlaneControls::new() };
        self.controls.throttle = 0.0;
        node.push_physics_event(event);
    }

    /// Burns fuel with the throttle. Dry tanks cut the engine. The physics
    /// body hears about each whole kilogram, and weighs that much less than
    /// a full jet.
    fn update_fuel(&mut self, node: &mut Node, app: &App, delta_time: f32) {
        if !self.fuel.has_tanks() {
            return;
        }
        if !app.is_paused && !self.wrecked && self.fuel.kg > 0.0 {
            let burned = self.fuel.burn_kg_s(self.controls.throttle, self.military) * delta_time;
            self.fuel.kg = (self.fuel.kg - burned).max(0.0);
        }
        if self.fuel.kg <= 0.0 {
            self.controls.throttle = 0.0;
            if !self.fuel.starved {
                self.fuel.starved = true;
                node.push_physics_event(AircraftEvent::EngineOut);
            }
        }
        // A shot is flying the jet. The tally above still runs for the HUD;
        // the body keeps the weight it spawned at until the shot lets go.
        // A fuel report rewrites the mass properties, and doing that on the
        // opening frames is what wrecked the intro.
        if self.held || self.calm || self.scripted {
            return;
        }
        if let Some(kg) = self.fuel.take_report() {
            node.push_physics_event(AircraftEvent::Fuel { kg, capacity: self.fuel.capacity_kg });
        }
    }

    /// A plane going down after a missile hit reaching the end: `crash`.
    fn end(&mut self, node: &mut Node, crash: Crash) {
        self.going_down = false;
        self.in_water = crash == Crash::Water;
        self.crash = Some(crash);
        node.push_physics_event(AircraftEvent::Wreck);
    }

    /// Drives the node's particle emitters (see plane::effects): wingtip
    /// vortices streaming off under G, smoke off the wreck, and a puff of
    /// tyre smoke off each wheel that touches down fast.
    fn update_effects(&mut self, node: &mut Node, delta_time: f32) {
        // Condensation shows from ~3.5 G, full by ~7 (either sign); the raw G
        // is spiky, so the strength eases toward it.
        let state = node.physics_state::<AircraftState>();
        let g = state.map(|state| state.flight_data.g_meter.abs()).unwrap_or(1.0);
        let t = ((g - 3.5) / 3.5).clamp(0.0, 1.0);
        let target = if self.wrecked { 0.0 } else { t * t * (3.0 - 2.0 * t) };
        self.vortex_strength = lerp(self.vortex_strength, target, (delta_time * 5.0).min(1.0));

        // Touchdowns: a wheel that wasn't on the ground last frame and is
        // now, fast enough - the faster, the thicker the smoke.
        let speed = state.map(|state| Vector3::new(state.flight_data.velocity.x, 0.0, state.flight_data.velocity.z).magnitude()).unwrap_or(0.0);
        let wheels = state.map(|state| state.wheels.clone()).unwrap_or_default();
        for (mesh, data) in &wheels {
            let was_grounded = self.wheels_grounded.insert(mesh.clone(), data.grounded).unwrap_or(true);
            if data.grounded && !was_grounded && speed > TYRE_SMOKE_MIN_SPEED {
                self.tyre_smoke.insert(mesh.clone(), TYRE_SMOKE_SECONDS);
            }
        }
        let smoke_strength = ((speed - TYRE_SMOKE_MIN_SPEED) / (TYRE_SMOKE_FULL_SPEED - TYRE_SMOKE_MIN_SPEED)).clamp(0.0, 1.0);

        // A wreck come to rest smokes less - eased in over a couple of
        // seconds, so it doesn't flick on and off as it settles.
        let moving = state.map(|state| state.flight_data.velocity.magnitude()).unwrap_or(0.0);
        let resting = self.wrecked && moving < WRECK_REST_SPEED;
        self.wreck_rest = lerp(self.wreck_rest, if resting { 1.0 } else { 0.0 }, (delta_time * WRECK_REST_RATE).min(1.0));
        let burning = self.wrecked && !self.in_water;
        let profiles = state.map(|state| state.profiles.clone()).unwrap_or_default();

        let Some(emitters) = node.get_property_mut::<ParticleEmitters>() else { return };
        for (name, kind) in &self.effects {
            let Some(emitter) = emitters.get_mut(name) else { continue };
            match kind {
                EffectKind::WingtipVortex => {
                    emitter.intensity = self.vortex_strength;
                    emitter.enabled = self.vortex_strength > 0.02;
                }
                EffectKind::WreckSmoke => {
                    emitter.enabled = burning;
                    emitter.intensity = lerp(1.0, WRECK_REST_SMOKE, self.wreck_rest);
                    // In flames from the moment it's done for - falling
                    // and on the ground.
                    if let Some(fire) = emitters.get_mut(&effects::wreck_fire_name(name)) {
                        fire.intensity = 1.0;
                        fire.enabled = burning;
                    }
                }
            }
        }
        for (mesh, left) in self.tyre_smoke.iter_mut() {
            let Some(emitter) = emitters.get_mut(&effects::tyre_smoke_name(mesh)) else { continue };
            // On the wheel's contact point, wherever the suspension has it.
            if let Some(data) = wheels.get(mesh) {
                emitter.offset = data.local_position;
            }
            emitter.enabled = *left > 0.0;
            emitter.intensity = smoke_strength.max(0.2);
            *left = (*left - delta_time).max(0.0);
        }

        // A hit surface smokes from the first HP it loses. Fire joins once
        // about half of it is gone, and both are full when the surface is.
        // A wreck has its own plume, so these stop.
        let aloft = !self.wrecked && !self.in_water;
        for (label, hp) in &profiles {
            let max = self.surface_max.get(label).copied().unwrap_or(1.0).max(1.0);
            let damage = (1.0 - hp / max).clamp(0.0, 1.0);
            let smoking = aloft && damage > 0.02;
            let fire = ((damage - 0.5) / 0.5).clamp(0.0, 1.0);
            if let Some(smoke) = emitters.get_mut(&effects::damage_smoke_name(label)) {
                smoke.enabled = smoking;
                smoke.intensity = damage;
            }
            if let Some(flame) = emitters.get_mut(&effects::damage_fire_name(label)) {
                flame.enabled = smoking && fire > 0.0;
                flame.intensity = fire;
            }
        }
    }
    /// The pilot feels the plane's G - frozen while paused, resting during
    /// cinematics (physics is paused, the G is stale), and after a wreck
    /// only the crash itself (see Pilot::recover).
    fn update_pilot(&mut self, node: &Node, app: &App, delta_time: f32) {
        if app.is_paused {
            return;
        }
        let flight_data = node.physics_state::<AircraftState>().map(|state| &state.flight_data);
        match flight_data {
            Some(flight_data) if !self.input_locked && !self.calm && self.calm_left <= 0.0 => {
                if self.wrecked {
                    self.pilot.recover(flight_data.impact_g, delta_time);
                } else {
                    self.pilot.update(flight_data.g_meter, flight_data.impact_g, delta_time);
                }
            }
            _ => self.pilot.rest(delta_time),
        }
    }

    /// F7 overlay's rolling history - see `App::aero_debug_trail`/
    /// `App::wing_lift_trail`/`App::pitch_rate_trail`.
    fn record_aero_debug_trail(app: &mut App, state: &AircraftState, controls: &PlaneControls, delta_time: f32) {
        let flight_data = &state.flight_data;
        app.aero_debug_trail.push_back(AeroSample {
            speed_kt: flight_data.speedometer,
            mach: flight_data.mach,
            aoa_deg: flight_data.aoa_y,
            altitude_m: flight_data.altimeter,
            roll_rate: flight_data.roll_rate,
            // FlightData::pitch_rate is about the body's +X (the left wing),
            // so its + is nose DOWN - flipped to nose-up +.
            pitch_rate: -flight_data.pitch_rate,
            yaw_rate: flight_data.yaw_rate,
            climb_rate: flight_data.velocity.y,
            aileron: controls.aileron,
            // elevator + is stick forward (see Plane::update) - flipped to
            // pull +.
            pitch_stick: -controls.elevator,
            rudder: controls.rudder,
            up_alignment: flight_data.up_alignment,
        });
        if app.aero_debug_trail.len() > Self::AERO_DEBUG_TRAIL_CAPACITY {
            app.aero_debug_trail.pop_front();
        }

        let main_wing_lift_y = state.wing_lift_forces.get("Left wing").map(|f| f.y).unwrap_or(0.0);
        let elevator_wing_lift_y = state.wing_lift_forces.get("Right elevator wing").map(|f| f.y).unwrap_or(0.0);
        let next_index = app.wing_lift_trail.back().map(|(i, _, _)| i + 1.0).unwrap_or(0.0);
        app.wing_lift_trail.push_back((next_index, main_wing_lift_y, elevator_wing_lift_y));
        if app.wing_lift_trail.len() > Self::AERO_DEBUG_TRAIL_CAPACITY {
            app.wing_lift_trail.pop_front();
        }

        let now = app.pitch_rate_trail.back().map(|(t, _)| t + delta_time).unwrap_or(0.0);
        app.pitch_rate_trail.push_back((now, flight_data.pitch_rate));
        while app.pitch_rate_trail.front().is_some_and(|(t, _)| now - t > Self::PITCH_RATE_TRAIL_SECONDS) {
            app.pitch_rate_trail.pop_front();
        }
    }
}

impl Behavior for Plane {
    /// Keeps where physics says the aircraft is - for the autopilot.
    fn fixed_update(&mut self, _node: &mut Node, _cameras: &mut SceneCameras, _app: &mut App, _dt: f32, physics_message: Option<&RenderMessage>) {
        if let Some(message) = physics_message {
            self.pose = Some((message.translation, UnitQuaternion::new_normalize(message.rotation)));
        }
    }

    fn update(&mut self, node: &mut Node, _cameras: &mut SceneCameras, app: &mut App, delta_time: f32) {
        if self.held || self.calm {
            self.calm_left = CALM_GRACE_SECONDS;
        } else if !app.is_paused {
            self.calm_left = (self.calm_left - delta_time).max(0.0);
        }
        // Placed by an animation: the physics' view of it (held still, at
        // no speed) means nothing - no G for the pilot, no crash, no flying
        // it - until it's let go.
        if self.held {
            self.flown_by = "placed by an animation";
            if !app.is_paused {
                self.update_effects(node, delta_time);
            }
            return;
        }
        // 1. Input → physics half.
        self.update_pilot(node, app, delta_time);
        self.read_blast_result(node);
        self.check_wreck(node);
        if !app.is_paused {
            self.update_effects(node, delta_time);
        }
        self.flown_by = if self.pilot.is_dead() && !self.wrecked {
            "nobody (pilot killed)"
        } else if !self.is_player || self.scripted {
            "autopilot"
        } else if self.wrecked {
            "nobody (wrecked)"
        } else if self.pilot.is_unconscious() {
            "nobody (pilot unconscious)"
        } else if self.input_locked {
            "nobody (input locked)"
        } else {
            "the pilot"
        };
        if self.pilot.is_dead() && !self.wrecked {
            // Hands off, throttle closed. The profiles the blast left keep
            // flying; a centered stick does not hold the jet up.
            self.controls.elevator = 0.0;
            self.controls.aileron = 0.0;
            self.controls.rudder = 0.0;
            self.controls.throttle = 0.0;
            self.controls.fly_by_wire_pitch_autotrim = false;
        } else if !self.is_player || self.scripted {
            // A bot (or one a timeline's flying): its autopilot flies its
            // path; without one its controls are left as they are.
            let flight_data = node.physics_state::<AircraftState>().map(|state| state.flight_data.clone());
            if let (Some(autopilot), Some((position, rotation)), Some(flight_data)) = (&mut self.autopilot, self.pose, flight_data) {
                if !self.wrecked && !app.is_paused {
                    autopilot.fly(&mut self.controls, position, rotation, &flight_data, delta_time);
                }
            }
        } else if !self.input_locked && !self.wrecked && !self.pilot.is_unconscious() {
            self.read_pilot_input(node, delta_time);
        } else if self.pilot.is_unconscious() {
            // Passed out: stick and pedals straight back to center, held
            // there until the pilot comes to (the throttle stays where it
            // was). Center means the same as a hands-off stick in
            // read_pilot_input - trim kept: without fly-by-wire the jet
            // needs its pitch trim to hold ~1 G, and without it a centered
            // stick noses it over into heavy negative G.
            self.controls.elevator = if self.fcs.pitch_autotrim { 0.0 } else { self.controls.trim.pitch };
            self.controls.aileron = self.controls.trim.roll;
            self.controls.rudder = 0.0;
        }
        if self.is_player {
            self.update_quick_menu(node, app);
        }
        self.update_fuel(node, app, delta_time);
        if !(self.pilot.is_dead() && !self.wrecked) {
            self.controls.fly_by_wire_pitch_autotrim = self.fcs.pitch_autotrim;
        }
        node.set_physics_input(self.controls.clone());

        // 2. Physics half → model. `None` until the physics thread has
        // reported back for the first time.
        let state = node.physics_state::<AircraftState>();
        let node_scale = node.get_property::<Transform3D>().map(|transform| transform.scale).unwrap_or(Vector3::new(1.0, 1.0, 1.0));
        let Some(model_property) = node.get_property::<ModelProperty>() else { return };
        // What maps model space to the world - node x model scale.
        let scale = model_property.render_scale(node_scale);
        // The key its loaded copy is kept under - see render_bridge.
        let model_ref = model_property.source.as_ref().map(|source| source.key())
            .or_else(|| app.model_sources.get(&model_property.model_ref).map(|source| source.key()))
            .unwrap_or_else(|| model_property.model_ref.clone());
        let model_ref = &model_ref;
        let Some(model_instance) = app.game_models.get_mut(model_ref) else { return };
        let model = &mut model_instance.model;

        let engine_dead = self.wrecked || self.fuel.starved;
        if engine_dead {
            self.afterburner.value = 0.0;
        } else {
            self.afterburner.update(state.map(|state| state.afterburner).unwrap_or(0.0), delta_time);
        }
        if let Some(afterburner_mesh) = model.mesh_lists.get_mut("transparent").and_then(|meshes| meshes.get_mut("Afterburner")) {
            let base = *self.afterburner_base_scale.get_or_insert(afterburner_mesh.transform.scale);
            let new_transform = Transform::new(afterburner_mesh.transform.position, afterburner_mesh.transform.rotation, Vector3::new(base.x, base.y, base.z * self.afterburner.value));
            afterburner_mesh.change_transform(&app.renderer.queue, new_transform);
        }

        let simulated_elevator = state.map(|state| state.elevator_control_input);
        for surface in &mut self.control_surfaces {
            surface.apply(model, &self.controls, simulated_elevator, delta_time, &app.renderer.queue);
        }

        if self.gear_meshes.armed_deploy().is_some() {
            self.gear_meshes.show_armed(model, scale, &app.renderer.queue);
        }
        if let Some(state) = state {
            self.gear_meshes.place(model, &state.wheels, state.gear_deploy, scale, &app.renderer.queue);
            self.gear_meshes.disarm();

            if self.is_player && app.debug_overlay_visible() {
                Self::record_aero_debug_trail(app, state, &self.controls, delta_time);
            }
        }
    }
}
