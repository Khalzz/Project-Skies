//! Countermeasures - flares and chaff.
//!
//! Each aircraft that carries them has a `CountermeasureSet` (from its
//! data.ron's `countermeasures`): how many of each are left, which it's set
//! to release (flares, chaff or both), and a burst program in progress.
//! Pressed once it releases one of each selected; the program releases
//! BURST_COUNT of them, BURST_INTERVAL apart. They come out of the plane's
//! dispensers - each where it is on the plane, which way and how fast it
//! throws them, and what it carries - taking turns.
//!
//! What's released flies on its own (`Countermeasures`): a flare is thrown
//! clear of the jet, slows in the air and falls, burning hot for
//! FLARE_BURN_SECONDS - a heat source an IR missile can be pulled onto (see
//! `heat_tracks` and play::weapon::heat); chaff blooms into a cloud of foil
//! that hangs and drifts down - for radar missiles, once there are any.

use nalgebra::{UnitQuaternion, Vector3};
use serde::Deserialize;

use crate::app::App;
use crate::engine::particles::{Emitter, EmissionShape, Forces, ParticleEffect, ParticleEmitters};
use crate::engine::scene_manager::node::Node;
use crate::engine::scene_manager::properties::Transform3D;
use crate::engine::scene_manager::scene::Scene;

use super::missile::TargetTrack;

/// The program: how many releases, how far apart (s).
pub const BURST_COUNT: u32 = 6;
pub const BURST_INTERVAL: f32 = 0.25;

/// A flare's heat at full burn (see play::weapon::heat - a jet on full
/// afterburner is 12), how quickly it lights (s), and how long it burns (s).
pub const FLARE_HEAT: f32 = 40.0;
const FLARE_LIGHT_SECONDS: f32 = 0.15;
pub const FLARE_BURN_SECONDS: f32 = 4.5;
/// How quickly the air slows them (1/s): a flare is a heavy pellet, chaff
/// a cloud of foil that all but stops.
const FLARE_DRAG: f32 = 1.2;
const CHAFF_DRAG: f32 = 3.0;
const GRAVITY: Vector3<f32> = Vector3::new(0.0, -9.81, 0.0);
/// Chaff's foil barely falls.
const CHAFF_GRAVITY: f32 = 0.1;
/// How long chaff is in the air, and how long a spent flare's node is kept
/// for its smoke to fade (s).
const CHAFF_SECONDS: f32 = 4.0;
const SMOKE_SECONDS: f32 = 4.0;

/// `data.ron`'s `countermeasures: ( ... )` - how many it carries, and the
/// dispensers they come out of.
#[derive(Debug, Clone, Deserialize)]
pub struct CountermeasureSpec {
    #[serde(default)]
    pub flares: u32,
    #[serde(default)]
    pub chaff: u32,
    pub dispensers: Vec<DispenserSpec>,
}

/// One dispenser on the plane (in the jet's frame: +X left, +Y up, +Z
/// nose): where it is (m), which way it throws what it releases (any
/// length - it's made a direction), how fast (m/s, on top of the jet's own
/// speed), and what it carries (both when left out).
#[derive(Debug, Clone, Deserialize)]
pub struct DispenserSpec {
    pub position: Vector3<f32>,
    pub direction: Vector3<f32>,
    #[serde(default = "default_eject_speed")]
    pub speed: f32,
    #[serde(default = "default_carries")]
    pub carries: Vec<Kind>,
}

fn default_eject_speed() -> f32 {
    25.0
}

fn default_carries() -> Vec<Kind> {
    vec![Kind::Flare, Kind::Chaff]
}

/// What a press releases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Release {
    Flares,
    Chaff,
    Both,
}

impl Release {
    /// The next setting, round and round.
    pub fn next(self) -> Self {
        match self {
            Release::Flares => Release::Chaff,
            Release::Chaff => Release::Both,
            Release::Both => Release::Flares,
        }
    }

    /// For the HUD.
    pub fn label(self) -> &'static str {
        match self {
            Release::Flares => "FLR",
            Release::Chaff => "CHF",
            Release::Both => "FLR+CHF",
        }
    }

    /// A press lets flares go.
    pub fn releases_flares(self) -> bool {
        matches!(self, Release::Flares | Release::Both)
    }

    /// A press lets chaff go.
    pub fn releases_chaff(self) -> bool {
        matches!(self, Release::Chaff | Release::Both)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
pub enum Kind {
    Flare,
    Chaff,
}

/// One flare or chaff coming out: what, and the dispenser it comes out of.
pub type Released = (Kind, DispenserSpec);

/// An aircraft's countermeasures - see the module doc.
#[derive(Clone, Debug)]
pub struct CountermeasureSet {
    pub spec: CountermeasureSpec,
    pub flares: u32,
    pub chaff: u32,
    pub release: Release,
    /// Releases the program still has to make, and until the next (s).
    program_left: u32,
    program_timer: f32,
    /// The dispenser that's next, for each kind - they take turns.
    next_flare: usize,
    next_chaff: usize,
}

impl CountermeasureSet {
    /// Full, set to flares.
    pub fn new(spec: CountermeasureSpec) -> Self {
        Self { flares: spec.flares, chaff: spec.chaff, spec, release: Release::Flares, program_left: 0, program_timer: 0.0, next_flare: 0, next_chaff: 0 }
    }

    /// One release: one of each kind selected that's left (and that a
    /// dispenser carries) - taken off what's left - and where each comes
    /// out.
    pub fn release_one(&mut self) -> Vec<Released> {
        let mut released = Vec::new();
        if matches!(self.release, Release::Flares | Release::Both) && self.flares > 0 {
            if let Some(dispenser) = self.next_dispenser(Kind::Flare) {
                self.flares -= 1;
                released.push((Kind::Flare, dispenser));
            }
        }
        if matches!(self.release, Release::Chaff | Release::Both) && self.chaff > 0 {
            if let Some(dispenser) = self.next_dispenser(Kind::Chaff) {
                self.chaff -= 1;
                released.push((Kind::Chaff, dispenser));
            }
        }
        released
    }

    /// The next dispenser round that carries `kind`.
    fn next_dispenser(&mut self, kind: Kind) -> Option<DispenserSpec> {
        let dispensers = &self.spec.dispensers;
        let next = match kind {
            Kind::Flare => &mut self.next_flare,
            Kind::Chaff => &mut self.next_chaff,
        };
        for offset in 0..dispensers.len() {
            let index = (*next + offset) % dispensers.len();
            if dispensers[index].carries.contains(&kind) {
                *next = index + 1;
                return Some(dispensers[index].clone());
            }
        }
        None
    }

    /// Starts the program: the first release now (returned), the rest
    /// BURST_INTERVAL apart from `update`.
    pub fn start_program(&mut self) -> Vec<Released> {
        self.program_left = BURST_COUNT - 1;
        self.program_timer = BURST_INTERVAL;
        self.release_one()
    }

    /// The program's releases due this frame.
    pub fn update(&mut self, delta_time: f32) -> Vec<Released> {
        if self.program_left == 0 {
            return Vec::new();
        }
        self.program_timer -= delta_time;
        if self.program_timer > 0.0 {
            return Vec::new();
        }
        self.program_left -= 1;
        self.program_timer += BURST_INTERVAL;
        self.release_one()
    }
}

/// A flare or chaff in the air.
struct Decoy {
    kind: Kind,
    node_id: String,
    position: Vector3<f32>,
    velocity: Vector3<f32>,
    age: f32,
}

impl Decoy {
    /// How hot it is now - a flare lighting up, burning, dying down; chaff
    /// none.
    fn heat(&self) -> f32 {
        match self.kind {
            Kind::Chaff => 0.0,
            Kind::Flare => {
                let lit = (self.age / FLARE_LIGHT_SECONDS).min(1.0);
                let left = (1.0 - self.age / FLARE_BURN_SECONDS).max(0.0);
                FLARE_HEAT * lit * left
            }
        }
    }

    fn lifetime(&self) -> f32 {
        match self.kind {
            Kind::Flare => FLARE_BURN_SECONDS,
            Kind::Chaff => CHAFF_SECONDS,
        }
    }
}

/// Every flare and chaff in the air - see the module doc.
#[derive(Default)]
pub struct Countermeasures {
    flying: Vec<Decoy>,
    /// Spent ones' nodes, kept while their smoke fades - and how long left.
    leftovers: Vec<(String, f32)>,
    released: u32,
}

impl Countermeasures {
    /// Releases `kind` out of `dispenser`, on a jet at `jet_position` turned
    /// `rotation` and moving at `velocity` - thrown out the way the
    /// dispenser points, at its speed.
    pub fn release(&mut self, scene: &mut Scene, app: &mut App, (kind, dispenser): Released, jet_position: Vector3<f32>, rotation: UnitQuaternion<f32>, velocity: Vector3<f32>) {
        self.released += 1;
        let node_id = match kind {
            Kind::Flare => format!("flare:{}", self.released),
            Kind::Chaff => format!("chaff:{}", self.released),
        };
        let position = jet_position + rotation * dispenser.position;
        let direction = dispenser.direction.try_normalize(1e-6).unwrap_or_else(|| -Vector3::y());
        let eject = rotation * direction * dispenser.speed;
        let emitters = match kind {
            Kind::Flare => flare_emitters(),
            Kind::Chaff => chaff_emitters(),
        };
        let spawned = scene.spawn_node(app,
            Node::new(&node_id)
                .add_property(Transform3D { position, ..Transform3D::default() })
                .add_property(emitters),
        );
        if spawned.is_ok() {
            self.flying.push(Decoy { kind, node_id, position, velocity: velocity + eject, age: 0.0 });
        }
    }

    /// Flies them all `delta_time` seconds - the spent ones left to fade
    /// and removed. Once a frame the game runs.
    pub fn update(&mut self, scene: &mut Scene, app: &mut App, delta_time: f32) {
        for decoy in &mut self.flying {
            let (drag, gravity) = match decoy.kind {
                Kind::Flare => (FLARE_DRAG, GRAVITY),
                Kind::Chaff => (CHAFF_DRAG, GRAVITY * CHAFF_GRAVITY),
            };
            decoy.velocity += (gravity - decoy.velocity * drag) * delta_time;
            decoy.position += decoy.velocity * delta_time;
            decoy.age += delta_time;
            if let Some(transform) = scene.content.nodes.get_mut(&decoy.node_id).and_then(|node| node.get_property_mut::<Transform3D>()) {
                transform.position = decoy.position;
            }
        }

        // Spent: stops burning, its smoke left to fade.
        let (spent, flying): (Vec<Decoy>, Vec<Decoy>) = std::mem::take(&mut self.flying).into_iter().partition(|decoy| decoy.age >= decoy.lifetime());
        self.flying = flying;
        for decoy in spent {
            if let Some(emitters) = scene.content.nodes.get_mut(&decoy.node_id).and_then(|node| node.get_property_mut::<ParticleEmitters>()) {
                for (_, emitter) in emitters.iter_mut() {
                    emitter.enabled = false;
                }
            }
            self.leftovers.push((decoy.node_id, SMOKE_SECONDS));
        }

        for (node_id, seconds_left) in &mut self.leftovers {
            *seconds_left -= delta_time;
            if *seconds_left <= 0.0 {
                scene.despawn_node(app, node_id, None);
            }
        }
        self.leftovers.retain(|(_, seconds_left)| *seconds_left > 0.0);
    }

    /// The flares burning - heat sources, for the missiles (and the heat
    /// view) to see alongside the aircraft.
    pub fn heat_tracks(&self) -> Vec<TargetTrack> {
        self.flying.iter()
            .filter(|decoy| decoy.kind == Kind::Flare)
            .map(|decoy| TargetTrack { name: decoy.node_id.clone(), position: decoy.position, velocity: decoy.velocity, heat: decoy.heat(), decoy: true, rotation: UnitQuaternion::identity(), colliders: Vec::new(), reach: 0.0 })
            .filter(|track| track.heat > 0.05)
            .collect()
    }
}

/// A flare: a searing white-orange core, sparking, and a trail of white
/// smoke left hanging behind it.
fn flare_emitters() -> ParticleEmitters {
    let core = ParticleEffect::new()
        .rate(90.0)
        .lifetime(0.1, 0.25)
        .shape(EmissionShape::Sphere { radius: 0.3 })
        .world_direction(Vector3::y())
        .cone(180.0)
        .speed(1.0, 4.0)
        .drag(4.0)
        .size_over_life(1.8, 0.4)
        .color_over_life([1.0, 0.95, 0.8, 1.0], [1.0, 0.5, 0.1, 0.0])
        .brightness(12.0)
        .additive()
        .max_particles(40);
    let smoke = ParticleEffect::new()
        .rate(0.0)
        .per_meter(0.6)
        .lifetime(SMOKE_SECONDS * 0.6, SMOKE_SECONDS)
        .shape(EmissionShape::Sphere { radius: 0.2 })
        .world_direction(Vector3::y())
        .cone(180.0)
        .speed(0.2, 1.0)
        .forces(Forces { gravity: -0.02, drag: 1.0, wind: 0.6, turbulence: 0.3 })
        .faceted(6)
        .size_over_life(0.6, 5.0)
        .color_over_life([0.95, 0.95, 0.95, 0.6], [0.9, 0.9, 0.9, 0.0])
        .lit()
        .max_particles(400);
    ParticleEmitters::new()
        .add("core", Emitter::particles(core))
        .add("smoke", Emitter::particles(smoke))
}

/// Chaff: a burst of glinting foil blooming out into a cloud and drifting.
fn chaff_emitters() -> ParticleEmitters {
    let foil = ParticleEffect::new()
        .rate(0.0)
        .burst(90, None)
        .lifetime(CHAFF_SECONDS * 0.6, CHAFF_SECONDS)
        .shape(EmissionShape::Sphere { radius: 0.5 })
        .world_direction(Vector3::y())
        .cone(180.0)
        .speed(2.0, 8.0)
        .forces(Forces { gravity: 0.05, drag: 2.5, wind: 1.0, turbulence: 1.0 })
        .spin(-180.0, 180.0)
        .size_over_life(0.25, 0.25)
        .color_over_life([0.85, 0.87, 0.9, 0.9], [0.75, 0.77, 0.8, 0.0])
        .lit()
        .max_particles(90);
    ParticleEmitters::new().add("foil", Emitter::particles(foil))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dispenser(x: f32, carries: Vec<Kind>) -> DispenserSpec {
        DispenserSpec { position: Vector3::new(x, 0.0, 0.0), direction: -Vector3::y(), speed: 25.0, carries }
    }

    fn spec() -> CountermeasureSpec {
        CountermeasureSpec { flares: 3, chaff: 2, dispensers: vec![dispenser(1.0, default_carries()), dispenser(-1.0, default_carries())] }
    }

    fn kinds(released: Vec<Released>) -> Vec<Kind> {
        released.into_iter().map(|(kind, _)| kind).collect()
    }

    #[test]
    fn a_release_takes_what_its_set_to() {
        let mut set = CountermeasureSet::new(spec());
        assert_eq!(kinds(set.release_one()), vec![Kind::Flare]);
        set.release = Release::Both;
        assert_eq!(kinds(set.release_one()), vec![Kind::Flare, Kind::Chaff]);
        assert_eq!((set.flares, set.chaff), (1, 1));
        set.release = Release::Chaff;
        assert_eq!(kinds(set.release_one()), vec![Kind::Chaff]);
        assert_eq!(kinds(set.release_one()), Vec::<Kind>::new(), "none left");
    }

    #[test]
    fn the_dispensers_take_turns_with_what_they_carry() {
        let mut set = CountermeasureSet::new(spec());
        let sides: Vec<f32> = (0..3).flat_map(|_| set.release_one()).map(|(_, dispenser)| dispenser.position.x).collect();
        assert_eq!(sides, vec![1.0, -1.0, 1.0]);
        // Only the right one carries chaff.
        let mut set = CountermeasureSet::new(CountermeasureSpec { dispensers: vec![dispenser(1.0, vec![Kind::Flare]), dispenser(-1.0, vec![Kind::Chaff])], ..spec() });
        set.release = Release::Chaff;
        assert!(set.release_one().iter().all(|(_, dispenser)| dispenser.position.x == -1.0));
    }

    #[test]
    fn the_program_releases_a_burst_over_time() {
        let mut set = CountermeasureSet::new(CountermeasureSpec { flares: 60, ..spec() });
        let mut released = set.start_program().len();
        for _ in 0..(3.0 * 60.0) as usize {
            released += set.update(1.0 / 60.0).len();
        }
        assert_eq!(released, BURST_COUNT as usize);
        assert_eq!(set.flares, 60 - BURST_COUNT);
    }

    #[test]
    fn a_flare_lights_burns_and_dies() {
        let flare = |age| Decoy { kind: Kind::Flare, node_id: String::new(), position: Vector3::zeros(), velocity: Vector3::zeros(), age }.heat();
        assert!(flare(0.0) < 1.0);
        assert!(flare(0.3) > 30.0);
        assert!(flare(FLARE_BURN_SECONDS) <= 0.0);
    }
}
