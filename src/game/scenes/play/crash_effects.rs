//! What an aircraft does when it ends - a big fireball and black smoke
//! where it hits the ground, a tall burst of spray and mist where it goes
//! into the sea. Each is a node of its own, removed once its longest
//! particle's gone.

use nalgebra::Vector3;

use crate::app::App;
use crate::engine::particles::{Emitter, EmissionShape, Forces, ParticleEffect, ParticleEmitters};
use crate::engine::scene_manager::node::Node;
use crate::engine::scene_manager::properties::Transform3D;
use crate::engine::scene_manager::scene::Scene;

/// How long (s) an effect's node is kept - its longest particle.
const EXPLOSION_SECONDS: f32 = 9.0;
const SPLASH_SECONDS: f32 = 7.0;

/// How an aircraft ended.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Crash {
    /// Into the ground - it blows up.
    Ground,
    /// Into the sea - a splash.
    Water,
}

#[derive(Default)]
pub struct CrashEffects {
    /// Each effect's node, and how long until it's removed.
    playing: Vec<(String, f32)>,
    spawned: u32,
}

impl CrashEffects {
    /// Sets off `crash`'s effect at `at`.
    pub fn spawn(&mut self, scene: &mut Scene, app: &mut App, crash: Crash, at: Vector3<f32>) {
        self.spawned += 1;
        let node_id = format!("crash:{}", self.spawned);
        let (emitters, seconds) = match crash {
            Crash::Ground => (explosion(), EXPLOSION_SECONDS),
            Crash::Water => (splash(), SPLASH_SECONDS),
        };
        let spawned = scene.spawn_node(app,
            Node::new(&node_id)
                .add_property(Transform3D { position: at, ..Transform3D::default() })
                .add_property(emitters),
        );
        if spawned.is_ok() {
            self.playing.push((node_id, seconds));
        }
    }

    /// Removes the effects that are over - once a frame the game runs.
    pub fn update(&mut self, scene: &mut Scene, app: &mut App, delta_time: f32) {
        for (node_id, seconds_left) in &mut self.playing {
            *seconds_left -= delta_time;
            if *seconds_left <= 0.0 {
                scene.despawn_node(app, node_id, None);
            }
        }
        self.playing.retain(|(_, seconds_left)| *seconds_left > 0.0);
    }
}

/// An aircraft blowing up on the ground: a big fireball, burning debris
/// thrown out and falling, and a column of black smoke.
fn explosion() -> ParticleEmitters {
    let fireball = ParticleEffect::new()
        .rate(0.0)
        .burst(80, None)
        .lifetime(0.6, 1.6)
        .shape(EmissionShape::Sphere { radius: 6.0 })
        .world_direction(Vector3::y())
        .cone(110.0)
        .speed(20.0, 60.0)
        .drag(3.0)
        .size_over_life(10.0, 35.0)
        .color_over_life([1.0, 0.7, 0.3, 1.0], [0.7, 0.2, 0.05, 0.0])
        .brightness(8.0)
        .additive()
        .max_particles(80);
    let debris = ParticleEffect::new()
        .rate(0.0)
        .burst(60, None)
        .lifetime(1.5, 3.0)
        .shape(EmissionShape::Sphere { radius: 3.0 })
        .world_direction(Vector3::y())
        .cone(70.0)
        .speed(25.0, 70.0)
        .forces(Forces { gravity: 1.0, drag: 0.3, wind: 0.0, turbulence: 0.0 })
        .size_over_life(1.5, 0.5)
        .color_over_life([1.0, 0.6, 0.2, 1.0], [0.3, 0.1, 0.05, 0.0])
        .brightness(5.0)
        .additive()
        .max_particles(60);
    let smoke = ParticleEffect::new()
        .rate(0.0)
        .burst(50, None)
        .lifetime(5.0, EXPLOSION_SECONDS)
        .shape(EmissionShape::Sphere { radius: 8.0 })
        .world_direction(Vector3::y())
        .cone(60.0)
        .speed(6.0, 20.0)
        .forces(Forces { gravity: -0.15, drag: 1.0, wind: 1.0, turbulence: 0.4 })
        .faceted(6)
        .size_over_life(12.0, 50.0)
        .color_over_life([0.08, 0.07, 0.06, 0.85], [0.25, 0.25, 0.25, 0.0])
        .lit()
        .max_particles(50);
    ParticleEmitters::new()
        .add("fireball", Emitter::particles(fireball))
        .add("debris", Emitter::particles(debris))
        .add("smoke", Emitter::particles(smoke))
}

/// An aircraft going into the sea: a tall column of spray thrown up and
/// falling back, a ring of it out to the sides, and mist hanging after.
fn splash() -> ParticleEmitters {
    let column = ParticleEffect::new()
        .rate(0.0)
        .burst(120, None)
        .lifetime(1.5, 3.5)
        .shape(EmissionShape::Sphere { radius: 4.0 })
        .world_direction(Vector3::y())
        .cone(20.0)
        .speed(20.0, 55.0)
        .forces(Forces { gravity: 1.0, drag: 0.4, wind: 0.2, turbulence: 0.0 })
        .faceted(6)
        .size_over_life(3.0, 9.0)
        .color_over_life([0.95, 0.97, 1.0, 0.95], [0.85, 0.9, 0.95, 0.0])
        .lit()
        .max_particles(120);
    let ring = ParticleEffect::new()
        .rate(0.0)
        .burst(80, None)
        .lifetime(1.0, 2.5)
        .shape(EmissionShape::Sphere { radius: 6.0 })
        .world_direction(Vector3::y())
        .cone(75.0)
        .speed(10.0, 30.0)
        .forces(Forces { gravity: 1.0, drag: 0.6, wind: 0.2, turbulence: 0.0 })
        .faceted(6)
        .size_over_life(2.5, 7.0)
        .color_over_life([0.95, 0.97, 1.0, 0.9], [0.85, 0.9, 0.95, 0.0])
        .lit()
        .max_particles(80);
    let mist = ParticleEffect::new()
        .rate(0.0)
        .burst(40, None)
        .lifetime(4.0, SPLASH_SECONDS)
        .shape(EmissionShape::Sphere { radius: 8.0 })
        .world_direction(Vector3::y())
        .cone(90.0)
        .speed(2.0, 8.0)
        .forces(Forces { gravity: -0.02, drag: 1.0, wind: 1.0, turbulence: 0.3 })
        .faceted(6)
        .size_over_life(10.0, 35.0)
        .color_over_life([0.9, 0.93, 0.96, 0.5], [0.9, 0.93, 0.96, 0.0])
        .lit()
        .max_particles(40);
    ParticleEmitters::new()
        .add("column", Emitter::particles(column))
        .add("ring", Emitter::particles(ring))
        .add("mist", Emitter::particles(mist))
}
