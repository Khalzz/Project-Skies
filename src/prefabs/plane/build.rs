//! An aircraft's node, as the map places it (`aircraft`) - and what goes
//! into it: its views, its look, its weight with its loadout on.

use std::collections::BTreeMap;

use nalgebra::Vector3;

use pankarta_software::prelude::DoubleSidedMeshes;
use pankarta_software::scene_manager::node::properties::{Material, Model, Transform3D};
use pankarta_software::scene_manager::node::Node;

use super::aircraft_physics::AircraftPhysics;
use super::aircraft_spec::AircraftSpec;
use super::ai::autopilot::Speed;
use super::effects;
use super::plane::Plane;
use super::pylon_spec::PylonSpec;
use crate::ext::game_object::{Camera as GameObjectCamera, Cameras};
use crate::ext::physics_behavior::PhysicsNodeExt;
use crate::ext::water_contact::WaterContact;
use crate::prefabs::camera;
use crate::scenes::play::map::{AircraftPlacement, DEFAULT_RADAR_WARN_M};
use crate::scenes::play::weapon::weapon_spec::WeaponSpec;

// After crashing into the water: how long the wreck floats, then how long it
// takes to flood and go under - see WaterContact::sinks.
const WRECK_FLOAT_SECONDS: f32 = 2.0;
const WRECK_SINK_SECONDS: f32 = 6.0;

/// The airspeed a bot holds flying its path when its map entry doesn't say
/// (m/s) - see `cruise_speed`.
const DEFAULT_CRUISE_SPEED: f32 = 150.0;

/// The aircraft `placement` puts on the map, flying `spec` - its node, named
/// as the placement is: the `Plane` behavior (a bot's autopilot flying
/// `flight_plan` - its points, and whether round and round - unless it's
/// the player's), its model, views, effects, body and its physics half, and
/// how it floats on a sea at `sea_level` once wrecked.
pub fn aircraft(
    spec: &AircraftSpec,
    placement: &AircraftPlacement,
    flight_plan: (Vec<(f32, f32, f32)>, bool),
    sea_level: f32,
) -> Node {
    let rotation = placement.rotation();
    let mut physics = spec.physics.clone();
    // Its loadout's weight on, where it hangs.
    let (mass, center_of_mass) = loaded_mass(spec, &placement.loadout);
    physics.rigidbody.mass = mass;
    physics.rigidbody.center_of_mass = center_of_mass;
    let colliders = physics.colliders.clone();
    if let Some(speed) = placement.speed {
        physics.rigidbody.initial_velocity = rotation * Vector3::new(0.0, 0.0, speed);
    }

    let mut plane = Plane::new()
        .with_effects(&spec.effects)
        .with_throttle(placement.throttle)
        .with_military(spec.engine.afterburner_gate)
        .with_fuel(&spec.fuel);
    plane.set_surface_max(&spec.aero.wings);
    if let Some(kg) = placement.fuel_kg {
        plane.set_fuel_kg(kg);
    }
    plane.bingo_kg = placement.bingo_kg.filter(|kg| *kg > 0.0);
    plane.radar_warn_m = placement.radar_warn_m.unwrap_or(DEFAULT_RADAR_WARN_M).max(0.0);
    plane.set_colliders(colliders.clone());
    plane.arm_gear(&spec.gear, placement.start.gear_down());
    plane.set_path(&flight_plan.0, Speed::Hold(cruise_speed(placement)), placement.difficulty, flight_plan.1);
    let plane = if placement.player { plane } else { plane.bot() };

    let mut model = Model::file(crate::asset_relative(&spec.model.model))
        .double_sided(DoubleSidedMeshes::Meshes(spec.model.double_sided.clone()))
        .transform(Transform3D { scale: Vector3::repeat(spec.model.scale), ..Default::default() });
    model.material = material(spec, Some(placement));

    Node::new(&placement.name)
        .with_behavior(plane)
        .with_property(Transform3D { position: placement.position(), rotation, scale: Vector3::new(1.0, 1.0, 1.0) })
        // One shared load of the plane's model (the engine can't move a file
        // model's meshes per node, so there's nothing for a copy of its own
        // to keep apart - see Plane::update).
        .with_property(model)
        // Its views - the cockpit at the pilot's seat.
        .with_property(cameras(spec))
        .with_property(effects::plane_emitters(&spec.effects, &spec.gear, &spec.aero.wings))
        .with_property(physics.to_engine())
        .add_physics_behavior(AircraftPhysics::new(&spec.aero, &spec.engine, &spec.gear, &spec.effects, center_of_mass, colliders, mass, placement.start.gear_down()))
        .add_physics_behavior(WaterContact::new(sea_level).sinks(WRECK_FLOAT_SECONDS, WRECK_SINK_SECONDS))
}

/// A plane's views as its data.ron places them - the cockpit at the pilot's
/// seat (`aero.pilot_position`) unless `cameras` says otherwise; the rest,
/// the camera's own defaults (see prefabs::camera). The cockpit gets the
/// plane's `head_limits`, when it has them.
pub fn cameras(spec: &AircraftSpec) -> Cameras {
    let mut cameras: Cameras = spec.cameras.clone();
    let cockpit = cameras
        .entry(camera::COCKPIT_CAMERA.to_owned())
        .or_insert(GameObjectCamera { position: spec.aero.pilot_position, fov: 70.0, head_limits: None });
    if spec.head_limits.is_some() {
        cockpit.head_limits = spec.head_limits;
    }
    cameras
}

/// What an aircraft is drawn with: its map placement's material (else
/// engine/standard), with its plane's mesh materials under the placement's
/// own - None when that's just engine/standard.
pub fn material(spec: &AircraftSpec, placement: Option<&AircraftPlacement>) -> Option<Material> {
    let mut material = placement
        .and_then(|placement| placement.material.clone())
        .unwrap_or_else(|| Material::shader(pankarta_software::rendering::shader_library::STANDARD));
    for (mesh, mesh_material) in &spec.materials {
        material.meshes.entry(mesh.clone()).or_insert_with(|| mesh_material.clone());
    }
    (!material.is_standard()).then_some(material)
}

/// A plane's mass and center of mass with a loadout on (kg, the jet's frame)
/// - its own, plus every weapon hung on it at its pylon. Weapons the loadout
/// can't hang (see flight_manager::loadout_problems) don't count.
pub fn loaded_mass(spec: &AircraftSpec, loadout: &BTreeMap<String, String>) -> (f32, Vector3<f32>) {
    let mut mass = spec.physics.rigidbody.mass;
    let mut moment = spec.physics.rigidbody.center_of_mass * mass;
    for (pylon, weapon) in loaded_stores(spec, loadout) {
        let weapon_mass = weapon.physics.rigidbody.mass;
        mass += weapon_mass;
        moment += pylon.position * weapon_mass;
    }
    (mass, moment / mass)
}

/// The weapons a loadout hangs on a plane: each with the pylon it's on -
/// only the ones that can be (the pylon's there, takes its kind, the weapon
/// loads).
pub fn loaded_stores<'a>(spec: &'a AircraftSpec, loadout: &BTreeMap<String, String>) -> Vec<(&'a PylonSpec, WeaponSpec)> {
    loadout
        .iter()
        .filter_map(|(pylon_name, weapon)| {
            let pylon = spec.pylons.iter().find(|pylon| pylon.name == *pylon_name)?;
            let weapon = WeaponSpec::read(weapon).ok()?;
            pylon.accepts(weapon.category).then_some((pylon, weapon))
        })
        .collect()
}

/// The airspeed a bot holds flying its path (m/s) - its map `speed`, or
/// DEFAULT_CRUISE_SPEED without one.
pub fn cruise_speed(placement: &AircraftPlacement) -> f32 {
    placement.speed.filter(|speed| *speed > 1.0).unwrap_or(DEFAULT_CRUISE_SPEED)
}
