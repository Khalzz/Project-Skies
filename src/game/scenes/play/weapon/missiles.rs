//! The scene's missiles in flight - each one its own model node, flown by
//! `MissileFlight` (see play::weapon::missile), with white smoke puffs
//! left behind it (from a node of its own, kept a few seconds after the
//! missile's gone so they fade out) and its plume lit while the motor burns.
//!
//! The plume is the model's `fire` mesh. A mesh's size is shared by every
//! node drawing that model, so two copies are kept: the plain one - weapons
//! hanging on pylons, and missiles whose motor has burnt out - with the
//! plume shrunk to nothing, and `FLYING_VARIANT`, the burning one, with it
//! lit and flickering. A missile switches over to the plain copy at burnout.

use nalgebra::{UnitQuaternion, Vector3};

use crate::app::App;
use crate::engine::particles::{Emitter, EmissionShape, Forces, ParticleEffect, ParticleEmitters};
use crate::engine::scene_manager::node::Node;
use crate::engine::scene_manager::properties::{Model, Transform3D};
use crate::engine::scene_manager::scene::Scene;
use crate::resources::DoubleSidedMeshes;
use crate::transform::Transform;

use super::missile::{MissileFlight, Outcome, Surroundings, TargetTrack};
use crate::game::scenes::play::world_collision::WorldCollision;
use super::weapon_spec::{Seeker, WeaponSpec};

/// The copy of a missile's model drawn while its motor burns - see the
/// module doc.
pub const FLYING_VARIANT: &str = "flying";
/// The plume's mesh in a missile's model.
const PLUME_MESH: &str = "fire";
/// How long (s) a puff of smoke hangs in the air, at most.
const SMOKE_SECONDS: f32 = 6.0;
/// How long (s) an explosion's node is kept - its longest particle.
const EXPLOSION_SECONDS: f32 = 6.0;

struct FlyingMissile {
    node_id: String,
    smoke_id: String,
    spec: WeaponSpec,
    flight: MissileFlight,
    /// Still drawn with the burning copy (see the module doc).
    burning_model: bool,
}

/// A node left behind to finish - a trail fading out, an explosion - and
/// how long until it's removed.
struct Leftover {
    node_id: String,
    seconds_left: f32,
}

/// What happened to the missiles this frame.
#[derive(Default)]
pub struct MissileEvents {
    /// Aircraft inside a warhead's lethal radius when it went off, by name.
    pub destroyed: Vec<String>,
}

#[derive(Default)]
pub struct Missiles {
    flying: Vec<FlyingMissile>,
    leftovers: Vec<Leftover>,
    launched: u32,
    /// For the plume's flicker.
    clock: f32,
    /// Each model's plume size as the file has it, by model key - what it's
    /// scaled from.
    plume_sizes: std::collections::HashMap<String, Vector3<f32>>,
}

impl Missiles {
    /// The node ids of the missiles in flight - for the camera to follow.
    pub fn node_ids(&self) -> impl Iterator<Item = &str> {
        self.flying.iter().map(|missile| missile.node_id.as_str())
    }

    /// Where the missile `node_id` is and which way it's flying (+Z along
    /// its flight - without its model's own turn) - for the camera to ride
    /// along.
    pub fn camera_pose(&self, node_id: &str) -> Option<(Vector3<f32>, UnitQuaternion<f32>)> {
        let missile = self.flying.iter().find(|missile| missile.node_id == node_id)?;
        Some((missile.flight.position, flight_rotation(&missile.flight)))
    }

    /// Every heat-seeking missile in flight homing on an aircraft: its id,
    /// and the aircraft's name - for the bots to react to.
    pub fn heat_seekers_homing(&self) -> Vec<(String, String)> {
        self.flying.iter()
            .filter(|missile| missile.spec.guidance.seeker == Seeker::InfraRed)
            .filter_map(|missile| Some((missile.node_id.clone(), missile.flight.target_aircraft()?.to_owned())))
            .collect()
    }

    /// How many are in flight.
    pub fn count(&self) -> usize {
        self.flying.len()
    }

    /// Fires `spec` from `position`, moving with its aircraft (`velocity`),
    /// pointing along the aircraft's nose (`rotation`) - homing on `target`
    /// if it has one.
    pub fn launch(&mut self, scene: &mut Scene, app: &mut App, spec: WeaponSpec, position: Vector3<f32>, rotation: UnitQuaternion<f32>, velocity: Vector3<f32>, target: Option<String>) {
        self.launched += 1;
        let node_id = format!("missile:{}", self.launched);
        let smoke_id = format!("{node_id}:smoke");
        let flight = MissileFlight::launch(position, velocity, rotation, target);
        let pose = pose(&spec, &flight);

        if let Err(error) = scene.spawn_node(app, missile_node(&node_id, &spec, pose, true)) {
            eprintln!("Couldn't launch {}: {error}", spec.id);
            return;
        }
        let _ = scene.spawn_node(app,
            Node::new(&smoke_id)
                .add_property(Transform3D { position: tail(&spec, &flight), rotation: pose.1, scale: Vector3::new(1.0, 1.0, 1.0) })
                .add_property(ParticleEmitters::new().add("smoke", Emitter::particles(smoke_puffs()))),
        );
        println!("Fox 2 - {} away{}", spec.id, flight.target_aircraft().map(|target| format!(" at {target}")).unwrap_or_default());
        self.flying.push(FlyingMissile { node_id, smoke_id, spec, flight, burning_model: true });
    }

    /// Flies every missile `delta_time` seconds - `aircraft`: every aircraft
    /// that's still flying, where it is, how it's moving and how hot it is;
    /// `ground`: the land they can fly into; `sun`: which way the sun is.
    /// Call once a frame while the game runs; `update_plumes` every frame.
    pub fn update(&mut self, scene: &mut Scene, app: &mut App, aircraft: &[TargetTrack], ground: &WorldCollision, sun: Option<Vector3<f32>>, delta_time: f32) -> MissileEvents {
        let mut events = MissileEvents::default();
        let surroundings = Surroundings { aircraft, ground: Some(ground), sea_level: app.water.ocean.sea_level(), sun };

        let mut ended = Vec::new();
        for (index, missile) in self.flying.iter_mut().enumerate() {
            let outcome = missile.flight.step(&missile.spec, delta_time, &surroundings);

            // Burnt out: over to the plain copy, plume gone.
            if missile.burning_model && !missile.flight.burning(&missile.spec) {
                missile.burning_model = false;
                scene.despawn_node(app, &missile.node_id, None);
                let _ = scene.spawn_node(app, missile_node(&missile.node_id, &missile.spec, pose(&missile.spec, &missile.flight), false));
            }

            let (position, rotation) = pose(&missile.spec, &missile.flight);
            if let Some(instance) = scene.content.renderizable_instances.get_mut(&missile.node_id) {
                instance.instance.transform.position = position;
                instance.instance.transform.rotation = rotation;
            }
            if let Some(transform) = scene.content.nodes.get_mut(&missile.node_id).and_then(|node| node.get_property_mut::<Transform3D>()) {
                transform.position = position;
                transform.rotation = rotation;
            }
            let burning = missile.flight.burning(&missile.spec);
            if let Some(smoke) = scene.content.nodes.get_mut(&missile.smoke_id) {
                if let Some(transform) = smoke.get_property_mut::<Transform3D>() {
                    transform.position = tail(&missile.spec, &missile.flight);
                    transform.rotation = rotation;
                }
                // Smoke only while the motor's burning.
                if let Some(emitter) = smoke.get_property_mut::<ParticleEmitters>().and_then(|emitters| emitters.get_mut("smoke")) {
                    emitter.enabled = burning;
                }
            }

            let (at, fuzed_by) = match outcome {
                Outcome::Flying => continue,
                Outcome::Detonated { at, target } => {
                    println!("{} detonated by {target}", missile.spec.id);
                    (at, Some(target))
                }
                Outcome::HitGround { at } => {
                    println!("{} hit the ground", missile.spec.id);
                    (at, None)
                }
                Outcome::HitWater { at } => {
                    println!("{} went into the sea", missile.spec.id);
                    (at, None)
                }
                Outcome::SelfDestructed { at } => {
                    println!("{} self-destructed", missile.spec.id);
                    (at, None)
                }
            };
            // What set the fuze off, and anything else inside the blast.
            events.destroyed.extend(fuzed_by.clone());
            events.destroyed.extend(aircraft.iter()
                .filter(|track| Some(&track.name) != fuzed_by.as_ref() && (track.position - at).magnitude() <= missile.spec.warhead.lethal_radius)
                .map(|track| track.name.clone()));
            ended.push((index, Some(at)));
        }

        // Ended: the missile goes, its smoke is left to fade, and a blast
        // where it went off.
        for (index, blast) in ended.into_iter().rev() {
            let missile = self.flying.remove(index);
            scene.despawn_node(app, &missile.node_id, None);
            if let Some(emitter) = scene.content.nodes.get_mut(&missile.smoke_id)
                .and_then(|node| node.get_property_mut::<ParticleEmitters>())
                .and_then(|emitters| emitters.get_mut("smoke")) {
                emitter.enabled = false;
            }
            self.leftovers.push(Leftover { node_id: missile.smoke_id, seconds_left: SMOKE_SECONDS });
            if let Some(at) = blast {
                let explosion_id = format!("{}:explosion", missile.node_id);
                let spawned = scene.spawn_node(app,
                    Node::new(&explosion_id)
                        .add_property(Transform3D { position: at, ..Transform3D::default() })
                        .add_property(explosion_emitters()),
                );
                if spawned.is_ok() {
                    self.leftovers.push(Leftover { node_id: explosion_id, seconds_left: EXPLOSION_SECONDS });
                }
            }
        }

        for leftover in &mut self.leftovers {
            leftover.seconds_left -= delta_time;
            if leftover.seconds_left <= 0.0 {
                scene.despawn_node(app, &leftover.node_id, None);
            }
        }
        self.leftovers.retain(|leftover| leftover.seconds_left > 0.0);
        events
    }

    /// Sizes every missile model's plume - nothing on the plain copies
    /// (`models`: weapons' model keys), lit and flickering on the burning
    /// ones. Every frame, paused or not.
    pub fn update_plumes(&mut self, app: &mut App, models: &[String], delta_time: f32) {
        self.clock += delta_time;
        // A quick, uneven flicker - two waves out of step.
        let flicker = 1.0 + 0.08 * (self.clock * 47.0).sin() + 0.05 * (self.clock * 83.0).sin();
        for model_key in models {
            let flying_key = format!("{model_key}#{FLYING_VARIANT}");
            self.size_plume(app, model_key, 0.0);
            self.size_plume(app, &flying_key, flicker);
        }
    }

    /// The model `model_key`'s plume at `scale` times its own size.
    fn size_plume(&mut self, app: &mut App, model_key: &str, scale: f32) {
        let Some(model) = app.game_models.get_mut(model_key) else { return };
        let Some(mesh) = model.model.mesh_lists.values_mut().find_map(|meshes| meshes.get_mut(PLUME_MESH)) else { return };
        let size = *self.plume_sizes.entry(model_key.to_owned()).or_insert(mesh.transform.scale);
        // Not quite zero - a degenerate matrix isn't worth the risk.
        let size = size * scale.max(1e-4);
        mesh.change_transform(&app.renderer.queue, Transform::new(mesh.transform.position, mesh.transform.rotation, size));
    }
}

/// Where a missile's model goes - its position, turned along its flight
/// (its model's own rotation on top).
fn pose(spec: &WeaponSpec, flight: &MissileFlight) -> (Vector3<f32>, UnitQuaternion<f32>) {
    (flight.position, flight_rotation(flight) * spec.model.rotation())
}

/// How it's turned in flight: +Z its nose.
fn flight_rotation(flight: &MissileFlight) -> UnitQuaternion<f32> {
    flight.attitude
}

/// Where its smoke comes out - its tail.
fn tail(spec: &WeaponSpec, flight: &MissileFlight) -> Vector3<f32> {
    let length = spec.model.scale;
    flight.position - flight.heading() * length * 0.5
}

/// A missile's model node - the burning copy or the plain one.
fn missile_node(node_id: &str, spec: &WeaponSpec, (position, rotation): (Vector3<f32>, UnitQuaternion<f32>), burning: bool) -> Node {
    let model = Model::from_file(&spec.model.model).double_sided(DoubleSidedMeshes::Meshes(spec.model.double_sided.clone()));
    let model = if burning { model.variant(FLYING_VARIANT) } else { model };
    Node::new(node_id)
        .add_property(Transform3D { position, rotation, scale: Vector3::new(1.0, 1.0, 1.0) })
        .add_property(model.scaled(spec.model.scale))
}

/// The white smoke a rocket motor leaves - puffs laid every few meters
/// along its path (however fast it goes), small at the missile, billowing
/// out, drifting and fading as they hang there.
fn smoke_puffs() -> ParticleEffect {
    ParticleEffect::new()
        .rate(0.0)
        .per_meter(0.3)
        .lifetime(SMOKE_SECONDS * 0.6, SMOKE_SECONDS)
        .shape(EmissionShape::Sphere { radius: 0.3 })
        .world_direction(Vector3::y())
        .cone(180.0)
        .speed(0.5, 2.5)
        .forces(Forces { gravity: -0.02, drag: 1.0, wind: 0.5, turbulence: 0.4 })
        .spin(-30.0, 30.0)
        .faceted(6)
        .size_over_life(0.8, 9.0)
        .color_over_life([0.95, 0.95, 0.95, 0.7], [0.85, 0.85, 0.85, 0.0])
        .lit()
        .max_particles(2500)
}

/// A warhead going off: a quick fireball and a puff of dark smoke.
fn explosion_emitters() -> ParticleEmitters {
    let fireball = ParticleEffect::new()
        .rate(0.0)
        .burst(40, None)
        .lifetime(0.3, 0.8)
        .shape(EmissionShape::Sphere { radius: 2.0 })
        .world_direction(Vector3::y())
        .cone(180.0)
        .speed(15.0, 45.0)
        .drag(4.0)
        .size_over_life(4.0, 14.0)
        .color_over_life([1.0, 0.65, 0.25, 1.0], [0.6, 0.15, 0.05, 0.0])
        .brightness(6.0)
        .additive()
        .max_particles(40);
    let smoke = ParticleEffect::new()
        .rate(0.0)
        .burst(25, None)
        .lifetime(3.5, EXPLOSION_SECONDS)
        .shape(EmissionShape::Sphere { radius: 3.0 })
        .world_direction(Vector3::y())
        .cone(180.0)
        .speed(5.0, 15.0)
        .forces(Forces { gravity: -0.1, drag: 1.5, wind: 1.0, turbulence: 0.3 })
        .faceted(6)
        .size_over_life(5.0, 22.0)
        .color_over_life([0.12, 0.11, 0.1, 0.8], [0.3, 0.3, 0.3, 0.0])
        .lit()
        .max_particles(25);
    ParticleEmitters::new()
        .add("fireball", Emitter::particles(fireball))
        .add("smoke", Emitter::particles(smoke))
}
