//! `Missile` - a missile node's behavior: it flies itself.
//!
//! Every frame the game runs it steps its `MissileFlight` through the
//! airspace (see airspace::Airspace), moves its model there and its smoke
//! trail to its tail (laying smoke while the motor burns), and lights its
//! plume. When the flight ends - its fuze, the ground, the sea, out of time
//! - it sets its warhead off on every aircraft still flying (each one's
//! physics works out what the blast reaches), leaves an explosion, lets its
//! trail fade, and removes itself.
//!
//! It flies in `update`, after the scene has put up this frame's airspace,
//! so it sees the aircraft where they are now - and anything that follows
//! it from the scene's update (the camera) asks where it'll be once this
//! frame's flown (`camera_pose`).

use nalgebra::{UnitQuaternion, Vector3};

use pankarta_software::app::App;
use pankarta_software::prelude::{Behavior, SceneAccess};
use pankarta_software::scene_manager::node::properties::{Model, Transform3D};
use pankarta_software::scene_manager::node::Node;

use super::airspace::{Airspace, AIRSPACE};
use super::effects::{self, PLUME_MESH, SMOKE};
use super::flight::{MissileFlight, Outcome, Surroundings};
use crate::ext::emitters::ParticleEmitters;
use crate::ext::physics_behavior::PhysicsNodeExt;
use crate::prefabs::plane::messages::AircraftEvent;
use crate::prefabs::plane::plane::Plane;
use crate::scenes::play::weapon::weapon_spec::{Seeker, WeaponSpec};

/// How far apart a missile's trail points are (m), and how many it keeps -
/// for the editor to draw its path.
const TRAIL_STEP: f32 = 25.0;
const TRAIL_POINTS: usize = 400;

/// A missile in flight as the editor's telemetry shows it.
pub struct MissileTelemetry {
    pub weapon: String,
    pub shooter: String,
    pub position: Vector3<f32>,
    pub velocity: Vector3<f32>,
    /// Seconds since launch.
    pub time: f32,
    /// Its motor's still burning.
    pub burning: bool,
    /// The aircraft its seeker's on, by name - none: lost, or on the sun.
    pub target: Option<String>,
    /// Where it's been, oldest first.
    pub trail: Vec<Vector3<f32>>,
}

/// A missile in flight, as F10's map shows it.
pub struct MissileTrack {
    pub position: Vector3<f32>,
    pub velocity: Vector3<f32>,
    /// The aircraft that fired it, by name.
    pub shooter: String,
}

/// A warhead going off - where, how far it reaches, how much HP it takes
/// off a surface it touches, the shove where it touches a jet (N s), and
/// who fired what.
struct BlastEvent {
    at: Vector3<f32>,
    radius: f32,
    damage: f32,
    impulse: f32,
    shooter: String,
    weapon: String,
}

/// A missile node's behavior - see the module doc.
pub struct Missile {
    /// Which one fired it is, from the first - its node's id.
    number: u32,
    spec: WeaponSpec,
    flight: MissileFlight,
    /// The aircraft that fired it, by name - its fuze never goes off on it.
    shooter: String,
    /// Where it's been, a point every TRAIL_STEP meters or so (at most
    /// TRAIL_POINTS).
    trail: Vec<Vector3<f32>>,
    /// For the plume's flicker (s).
    clock: f32,
    /// Its flight's over - it's on its way out of the scene.
    ended: bool,
}

impl Missile {
    pub(super) fn new(number: u32, shooter: &str, spec: WeaponSpec, flight: MissileFlight) -> Self {
        let trail = vec![flight.position];
        Self { number, spec, flight, shooter: shooter.to_owned(), trail, clock: 0.0, ended: false }
    }

    /// Its node's id.
    pub fn id(&self) -> String {
        format!("missile:{}", self.number)
    }

    /// Its smoke trail's node's id.
    fn smoke_id(&self) -> String {
        format!("{}:smoke", self.id())
    }

    /// Which one fired it is, from the first.
    pub fn number(&self) -> u32 {
        self.number
    }

    /// What it is.
    pub fn spec(&self) -> &WeaponSpec {
        &self.spec
    }

    /// Where its model goes - its position, turned along its flight (its
    /// model's own rotation on top).
    pub(super) fn pose(&self) -> (Vector3<f32>, UnitQuaternion<f32>) {
        (self.flight.position, self.flight.attitude * self.spec.model.rotation())
    }

    /// Where its smoke comes out - its tail.
    fn tail(&self) -> Vector3<f32> {
        self.flight.position - self.flight.heading() * self.spec.model.scale * 0.5
    }

    /// Where it'll be once it's flown this frame's `delta_time` and which
    /// way it's flying (+Z along its flight - without its model's own turn)
    /// - for the camera to ride along, placed from the scene's update,
    /// before the missile has flown (see the module doc). A frame's straight
    /// line is within centimeters of the curve it flies.
    pub fn camera_pose(&self, delta_time: f32) -> (Vector3<f32>, UnitQuaternion<f32>) {
        (self.flight.position + self.flight.velocity * delta_time, self.flight.attitude)
    }

    /// For the editor's telemetry.
    pub fn telemetry(&self) -> MissileTelemetry {
        MissileTelemetry {
            weapon: self.spec.id.clone(),
            shooter: self.shooter.clone(),
            position: self.flight.position,
            velocity: self.flight.velocity,
            time: self.flight.time,
            burning: self.flight.burning(&self.spec),
            target: self.flight.target_aircraft().map(str::to_owned),
            trail: self.trail.clone(),
        }
    }

    /// Where it is, how it's moving, who fired it.
    pub fn track(&self) -> MissileTrack {
        MissileTrack { position: self.flight.position, velocity: self.flight.velocity, shooter: self.shooter.clone() }
    }

    /// The aircraft it's homing on, by name, if it's a heat seeker - for
    /// the bots to react to.
    pub fn heat_seeker_target(&self) -> Option<&str> {
        (self.spec.guidance.seeker == Seeker::InfraRed).then(|| self.flight.target_aircraft()).flatten()
    }

    /// Flies `delta_time` through the airspace, and moves its model and
    /// smoke there - ending it if that's how the step ended.
    fn fly(&mut self, node: &mut Node, scene: &mut SceneAccess, app: &mut App, delta_time: f32) {
        let sea_level = app.particles.sea_level().unwrap_or(0.0);
        let outcome = {
            let airspace = scene.get_node(AIRSPACE).and_then(|node| node.get_property::<Airspace>());
            let surroundings = match airspace {
                Some(airspace) => Surroundings {
                    aircraft: &airspace.aircraft,
                    ground: Some(&airspace.ground),
                    sea_level,
                    sun: airspace.sun,
                    shooter: &self.shooter,
                },
                None => Surroundings { shooter: &self.shooter, ..Surroundings::open(&[], sea_level) },
            };
            self.flight.step(&self.spec, delta_time, &surroundings)
        };

        if self.trail.last().is_none_or(|last| (self.flight.position - last).magnitude() >= TRAIL_STEP) {
            self.trail.push(self.flight.position);
            if self.trail.len() > TRAIL_POINTS {
                self.trail.remove(0);
            }
        }
        let (position, rotation) = self.pose();
        if let Some(transform) = node.get_property_mut::<Transform3D>() {
            transform.position = position;
            transform.rotation = rotation;
        }
        let burning = self.flight.burning(&self.spec);
        let tail = self.tail();
        if let Some(smoke) = scene.get_node_mut(&self.smoke_id()) {
            if let Some(transform) = smoke.get_property_mut::<Transform3D>() {
                transform.position = tail;
                transform.rotation = rotation;
            }
            // Smoke only while the motor's burning.
            if let Some(emitter) = smoke.get_property_mut::<ParticleEmitters>().and_then(|emitters| emitters.get_mut(SMOKE)) {
                emitter.enabled = burning;
            }
        }

        let at = match outcome {
            Outcome::Flying => return,
            Outcome::Detonated { at, target } => {
                println!("{} detonated by {target}", self.spec.id);
                at
            }
            Outcome::HitGround { at } => {
                println!("{} hit the ground", self.spec.id);
                at
            }
            Outcome::HitWater { at } => {
                println!("{} went into the sea", self.spec.id);
                at
            }
            Outcome::SelfDestructed { at } => {
                println!("{} self-destructed", self.spec.id);
                at
            }
        };
        self.end(node, scene, at);
    }

    /// Its warhead goes off at `at`: onto every aircraft still flying, an
    /// explosion there, its smoke left to fade - and it's gone.
    fn end(&mut self, node: &Node, scene: &mut SceneAccess, at: Vector3<f32>) {
        self.ended = true;
        let blast = BlastEvent {
            at,
            radius: self.spec.warhead.lethal_radius,
            damage: self.spec.warhead.damage,
            impulse: self.spec.warhead.impulse,
            shooter: self.shooter.clone(),
            weapon: self.spec.id.clone(),
        };
        deliver(scene, &blast);
        if let Some(emitter) = scene
            .get_node_mut(&self.smoke_id())
            .and_then(|node| node.get_property_mut::<ParticleEmitters>())
            .and_then(|emitters| emitters.get_mut(SMOKE))
        {
            emitter.enabled = false;
        }
        scene.spawn(effects::explosion(&format!("{}:explosion", node.id), at));
        scene.despawn(node.id.clone());
    }

    /// The plume: lit and flickering while the motor burns, hidden after.
    fn light_plume(&self, node: &mut Node) {
        let burning = self.flight.burning(&self.spec);
        // A quick, uneven flicker - two waves out of step.
        let flicker = 1.0 + 0.08 * (self.clock * 47.0).sin() + 0.05 * (self.clock * 83.0).sin();
        let Some(plume) = node.get_property_mut::<Model>().and_then(|model| model.mesh_mut(PLUME_MESH)) else { return };
        plume.visible = burning;
        if burning {
            plume.transform.scale = plume.original_transform().scale * flicker;
        }
    }
}

/// `blast` onto every aircraft still flying. Each one's physics spends wing
/// HP and notices a seat inside the sphere; a wing hit doesn't shoot the
/// jet down.
fn deliver(scene: &mut SceneAccess, blast: &BlastEvent) {
    let flying: Vec<String> = scene
        .nodes()
        .filter(|node| node.get_behavior::<Plane>().is_some_and(|plane| !plane.wrecked))
        .map(|node| node.id.clone())
        .collect();
    for id in flying {
        let Some(node) = scene.get_node_mut(&id) else { continue };
        if let Some(plane) = node.get_behavior_mut::<Plane>() {
            plane.note_blast(&blast.shooter, &blast.weapon);
        }
        node.push_physics_event(AircraftEvent::Blast {
            at: blast.at,
            radius: blast.radius,
            damage: blast.damage,
            impulse: blast.impulse,
        });
    }
}

impl Behavior for Missile {
    /// Its smoke trail, at its tail.
    fn on_spawn(&mut self, node: &mut Node, scene: &mut SceneAccess, _app: &mut App) {
        let (_, rotation) = self.pose();
        scene.spawn(effects::smoke_trail(&self.smoke_id(), &node.id, self.tail(), rotation));
        println!(
            "Fox 2 - {} away{}",
            self.spec.id,
            self.flight.target_aircraft().map(|target| format!(" at {target}")).unwrap_or_default()
        );
    }

    fn update(&mut self, node: &mut Node, scene: &mut SceneAccess, app: &mut App) {
        if self.ended || app.time.is_paused() {
            return;
        }
        let delta_time = app.time.delta_time;
        self.clock += delta_time;
        self.fly(node, scene, app, delta_time);
        self.light_plume(node);
    }
}
