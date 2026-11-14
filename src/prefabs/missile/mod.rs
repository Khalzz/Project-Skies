//! A missile - a node that flies itself once it's fired (`missile`): its
//! `Missile` behavior steps its flight every frame, moves its model and
//! lights its plume, lays its smoke trail, and when it ends sets its
//! warhead off on the aircraft round it, leaves an explosion and its trail
//! to fade, and removes itself.
//!
//! - `flight`: the flight itself, as plain numbers - motor, drag, homing,
//!   the seeker and the fuze (tested on its own);
//! - `behavior`: the node's `Missile` behavior, and what the rest of the game
//!   reads off it (the camera, the editor, the map, the bots);
//! - `airspace`: what's round every missile this frame - the aircraft and
//!   flares, the ground, the sun - put up once a frame by the scene for all
//!   of them to read;
//! - `effects`: its smoke, its explosion, and the leftovers' own countdown.

pub mod airspace;
mod behavior;
mod effects;
pub mod flight;

pub use airspace::Airspace;
pub use behavior::{Missile, MissileTelemetry, MissileTrack};
pub use effects::PLUME_MESH;
pub use flight::TargetTrack;

use std::sync::atomic::{AtomicU32, Ordering};

use nalgebra::{UnitQuaternion, Vector3};

use pankarta_software::prelude::DoubleSidedMeshes;
use pankarta_software::scene_manager::node::properties::{Model, Transform3D};
use pankarta_software::scene_manager::node::Node;
use pankarta_software::scene_manager::scene::Scene;

use crate::scenes::play::weapon::weapon_spec::WeaponSpec;
use flight::MissileFlight;

/// Every missile fired so far - each one's number, for its node's id.
static LAUNCHED: AtomicU32 = AtomicU32::new(0);

/// `spec` fired off the aircraft `shooter`, from `position`, moving with it
/// (`velocity`), pointing along its nose (`rotation`) - homing on `target`
/// if it has one. Its node ("missile:<n>"), ready to spawn: it flies itself
/// from then on.
pub fn missile(
    shooter: &str,
    spec: WeaponSpec,
    position: Vector3<f32>,
    rotation: UnitQuaternion<f32>,
    velocity: Vector3<f32>,
    target: Option<String>,
) -> Node {
    let number = LAUNCHED.fetch_add(1, Ordering::Relaxed) + 1;
    let flight = MissileFlight::launch(position, velocity, rotation, target);
    let missile = Missile::new(number, shooter, spec, flight);
    let (position, rotation) = missile.pose();
    let spec = missile.spec();
    let model = Model::file(crate::asset_relative(&spec.model.model))
        .double_sided(DoubleSidedMeshes::Meshes(spec.model.double_sided.clone()))
        .transform(Transform3D { scale: Vector3::repeat(spec.model.scale), ..Default::default() });
    Node::new(&missile.id())
        .with_property(Transform3D { position, rotation, scale: Vector3::new(1.0, 1.0, 1.0) })
        .with_property(model)
        .with_behavior(missile)
}

/// Every missile in flight, with its node's id - in the order they were
/// fired.
pub fn in_flight(scene: &Scene) -> Vec<(&str, &Missile)> {
    let mut missiles: Vec<(&str, &Missile)> = scene
        .content
        .nodes
        .iter()
        .filter_map(|node| Some((node.id.as_str(), node.get_behavior::<Missile>()?)))
        .collect();
    missiles.sort_by_key(|(_, missile)| missile.number());
    missiles
}

/// The missile `id`, if it's in flight.
pub fn get<'a>(scene: &'a Scene, id: &str) -> Option<&'a Missile> {
    scene.content.nodes.get(id)?.get_behavior::<Missile>()
}
