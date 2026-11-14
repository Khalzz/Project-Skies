//! A missile's effects: the white smoke its motor leaves (on a node of its
//! own, so the trail outlives the missile and fades out), the explosion
//! where it ends, and `Expires` - what removes those two once they're done.
//! The plume is the model's `PLUME_MESH`, lit by the Missile itself.

use nalgebra::{UnitQuaternion, Vector3};

use pankarta_software::app::App;
use pankarta_software::particles::{EmissionShape, Forces, ParticleEffect};
use pankarta_software::prelude::{Behavior, SceneAccess};
use pankarta_software::scene_manager::node::properties::Transform3D;
use pankarta_software::scene_manager::node::Node;

use crate::ext::emitters::{Emitter, ParticleEmitters};

/// The plume's mesh in a missile's model - hidden on weapons hanging on
/// pylons (see Play::update_weapons) and once the motor's burnt out.
pub const PLUME_MESH: &str = "fire";
/// The smoke trail's emitter, on its node.
pub(super) const SMOKE: &str = "smoke";
/// How long (s) a puff of smoke hangs in the air, at most - long enough that
/// the trail from the launch is still there when the motor burns out.
const SMOKE_SECONDS: f32 = 18.0;
/// The smoke trail: a puff every 1/this meters, and how many one missile's
/// trail can hold. Its emitter is a ring - once full, each new puff replaces
/// the oldest - so this many puffs is how long the trail can be: 4096 at
/// 0.6 a meter is ~6.8 km, a whole burn at Mach 3 (an AIM-9X burns 5.2 s and
/// covers ~3-4 km). Too few and the trail is wiped from behind as fast as
/// it's laid (600 at 2.5 a meter used to keep only the last ~240 m).
const SMOKE_PUFFS_PER_METER: f32 = 0.6;
const SMOKE_MAX_PUFFS: u32 = 4096;
/// How long (s) an explosion's node is kept - its longest particle.
const EXPLOSION_SECONDS: f32 = 6.0;

/// The smoke trail of the missile `missile_id` - its node `id`, at its tail
/// (`position`, turned `rotation`). The missile moves it; once the missile's
/// gone, it fades for SMOKE_SECONDS and goes too.
pub(super) fn smoke_trail(id: &str, missile_id: &str, position: Vector3<f32>, rotation: UnitQuaternion<f32>) -> Node {
    Node::new(id)
        .with_property(Transform3D { position, rotation, scale: Vector3::new(1.0, 1.0, 1.0) })
        .with_property(ParticleEmitters::new().add(SMOKE, Emitter::particles(smoke_puffs())))
        .with_behavior(Expires::once_gone(missile_id, SMOKE_SECONDS))
}

/// A warhead going off at `at` - its node `id`, gone once its smoke has.
pub(super) fn explosion(id: &str, at: Vector3<f32>) -> Node {
    Node::new(id)
        .with_property(Transform3D { position: at, ..Transform3D::default() })
        .with_property(explosion_emitters())
        .with_behavior(Expires::after(EXPLOSION_SECONDS))
}

/// Removes its node after a while - counted while the game runs - from
/// when it's spawned, or from when the node it waits on is gone.
pub(super) struct Expires {
    waiting_on: Option<String>,
    seconds_left: f32,
}

impl Expires {
    fn after(seconds: f32) -> Self {
        Self { waiting_on: None, seconds_left: seconds }
    }

    fn once_gone(id: &str, seconds: f32) -> Self {
        Self { waiting_on: Some(id.to_owned()), seconds_left: seconds }
    }
}

impl Behavior for Expires {
    fn update(&mut self, node: &mut Node, scene: &mut SceneAccess, app: &mut App) {
        if app.time.is_paused() {
            return;
        }
        if let Some(id) = &self.waiting_on {
            if scene.get_node(id).is_some() {
                return;
            }
            self.waiting_on = None;
        }
        self.seconds_left -= app.time.delta_time;
        if self.seconds_left <= 0.0 {
            scene.despawn(node.id.clone());
        }
    }
}

/// The white smoke a rocket motor leaves. Puffs are spaced on the order of
/// their own size: a 9 m puff every 0.3 m meant dozens of them covered each
/// pixel, and that overdraw is what a launch cost. A soft billboard, not a
/// faceted ball, so the ones that do overlap stay cheap.
fn smoke_puffs() -> ParticleEffect {
    ParticleEffect::new()
        .rate(0.0)
        .per_meter(SMOKE_PUFFS_PER_METER)
        .lifetime(SMOKE_SECONDS * 0.75, SMOKE_SECONDS)
        .shape(EmissionShape::Sphere { radius: 0.4 })
        .world_direction(Vector3::y())
        .cone(180.0)
        .speed(0.5, 2.5)
        .forces(Forces { gravity: -0.02, drag: 1.0, wind: 0.5, turbulence: 0.4 })
        .spin(-30.0, 30.0)
        // Puffs SMOKE_PUFFS_PER_METER apart, born big enough to overlap the
        // next, so the trail reads as one unbroken line.
        .size_over_life(1.6, 14.0)
        .color_over_life([0.95, 0.95, 0.95, 0.7], [0.85, 0.85, 0.85, 0.0])
        .lit()
        .max_particles(SMOKE_MAX_PUFFS)
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
