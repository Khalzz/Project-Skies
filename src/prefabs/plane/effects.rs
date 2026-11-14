//! The plane's particle effects - attached as the node's `ParticleEmitters`
//! (see play::spawn_world) and driven by `Plane` (see
//! `Plane::update_effects`).

use nalgebra::Vector3;
use serde::Deserialize;

use pankarta_software::particles::{EmissionShape, Facing, Forces, IntensityScales, ParticleEffect, TrailEffect};
use crate::ext::emitters::{Emitter, ParticleEmitters};

use super::aero_spec::WingSpec;
use super::gear_spec::GearSpec;

/// What an emitter in a plane's data.ron makes - each kind is one effect
/// below, driven by `Plane::update_effects`.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub enum EffectKind {
    /// Condensation off a wingtip under G (see `wingtip_vortex`).
    WingtipVortex,
    /// Smoke off the wreck after a crash (see `wreck_smoke`).
    WreckSmoke,
}

/// One particle emitter on the plane - `data.ron`'s `effects: [ ... ]`.
#[derive(Debug, Clone, Deserialize)]
pub struct EffectSpec {
    /// Its name on the plane's node (`ParticleEmitters`).
    pub name: String,
    pub effect: EffectKind,
    /// Where it sits, in meters in the jet's frame (+X left, +Y up, +Z nose).
    pub position: Vector3<f32>,
}

/// The tyre-smoke emitter every wheel gets (see `tyre_smoke`), by its mesh.
pub fn tyre_smoke_name(wheel_mesh: &str) -> String {
    format!("tyre_smoke:{wheel_mesh}")
}

/// Condensation streaming off a wingtip under G - a thin white wisp left
/// hanging in the air where the plane passed (barely drifting), slowly
/// spreading out and fading away over several seconds.
pub fn wingtip_vortex() -> TrailEffect {
    let mut effect = TrailEffect::new()
        .lifetime(6.0)
        .segment_spacing(2.0)
        .width_over_life(0.3, 2.5)
        .color_over_life([1.0, 1.0, 1.0, 0.5], [1.0, 1.0, 1.0, 0.0]);
    effect.wind = 0.2;
    effect
}

/// Dark, low-poly smoke off the wreck - a plume shaped like a triangle:
/// puffs start small right at the wreck, burst up ~SMOKE_BURST meters (the
/// launch, stopped by drag), then keep climbing slowly at SMOKE_CLIMB (hot
/// smoke is buoyant) while they grow - so the growing puffs stack into a
/// widening column instead of piling up into one ball.
pub fn wreck_smoke() -> ParticleEffect {
    // With no wind, a puff launched at `speed` against `drag` loses the
    // launch after ~speed / drag meters, then rises at the steady speed
    // where drag balances buoyancy: 9.81 * -gravity / drag.
    const SMOKE_BURST: f32 = 4.0;
    const SMOKE_CLIMB: f32 = 10.0;
    const SMOKE_DRAG: f32 = 1.0;
    // Few puffs - each grows big and lives long, so a handful a second is
    // already a solid column (more just piles up into a dark mass).
    ParticleEffect::new()
        .rate(10.0)
        .lifetime(5.0, 8.0)
        .shape(EmissionShape::Sphere { radius: 1.0 })
        .world_direction(Vector3::y())
        .cone(25.0)
        .speed(SMOKE_BURST * SMOKE_DRAG * 0.8, SMOKE_BURST * SMOKE_DRAG * 1.2)
        .forces(Forces { gravity: -SMOKE_CLIMB * SMOKE_DRAG / 9.81, drag: SMOKE_DRAG, wind: 0.0, turbulence: 0.3 })
        .spin(-20.0, 20.0)
        .faceted(6)
        .size_over_life(1.0, 40.0)
        .color_over_life([0.06, 0.06, 0.06, 0.75], [0.25, 0.25, 0.25, 0.0])
        .lit()
        .max_particles(100)
}

/// The fire emitter that goes with a wreck-smoke one (see `wreck_fire`), by
/// the smoke's name.
pub fn wreck_fire_name(smoke_name: &str) -> String {
    format!("{smoke_name}:fire")
}

/// Flames off a wreck - small, quick, flickering tongues of orange that
/// rise a few meters and die (falling, left streaming behind it), low-poly
/// like the smoke with them. `intensity` scales how many.
pub fn wreck_fire() -> ParticleEffect {
    ParticleEffect::new()
        .rate(45.0)
        .lifetime(0.4, 0.9)
        .shape(EmissionShape::Sphere { radius: 1.5 })
        .world_direction(Vector3::y())
        .cone(20.0)
        .speed(2.0, 4.5)
        .forces(Forces { gravity: -0.3, drag: 1.5, wind: 0.3, turbulence: 2.0 })
        .spin(-60.0, 60.0)
        .faceted(5)
        .size_over_life(1.6, 0.4)
        .color_over_life([1.0, 0.75, 0.3, 0.95], [0.9, 0.25, 0.05, 0.0])
        .brightness(5.0)
        .additive()
        .max_particles(60)
}

/// A puff of tyre smoke where a wheel touches down fast - the tyre
/// spinning up from still to rolling speed in an instant. A burst the
/// moment it's switched on, plus a short stream while it stays on (see
/// `Plane::update_effects`), low-poly like the other smoke, trailing back
/// off the wheel and fading within a second or two.
pub fn tyre_smoke() -> ParticleEffect {
    let mut effect = ParticleEffect::new()
        .burst(18, None)
        .rate(70.0)
        .lifetime(0.8, 1.8)
        .shape(EmissionShape::Sphere { radius: 0.3 })
        .world_direction(Vector3::y())
        .cone(70.0)
        .speed(1.0, 4.0)
        // Leaves the wheel with some of its speed, then the air stops it -
        // the puff streams back off the wheel.
        .inherit_velocity(0.35)
        .forces(Forces { gravity: -0.05, drag: 3.0, wind: 1.0, turbulence: 0.6 })
        .spin(-40.0, 40.0)
        .faceted(6)
        .facing(Facing::Camera)
        .size_over_life(0.6, 5.0)
        .color_over_life([0.78, 0.78, 0.76, 0.65], [0.85, 0.85, 0.83, 0.0])
        .lit()
        .max_particles(200);
    effect.intensity_scales = IntensityScales { rate: true, alpha: true, size: true, speed: false };
    effect
}

/// Smoke off a damaged surface, named by its wing label.
pub fn damage_smoke_name(label: &str) -> String {
    format!("damage:{label}")
}

/// Fire on a badly damaged surface, beside `damage_smoke_name`.
pub fn damage_fire_name(label: &str) -> String {
    format!("damage:{label}:fire")
}

/// Smoke off a hit surface - the engine's particles example's smoke
/// (examples/3D/particles.rs): a steady 30 puffs a second rising and
/// spreading, not a trail laid by the meter. `Plane::update_effects` turns
/// it up with how much HP is gone, so a nick is a wisp and a ruined wing is
/// a plume.
pub fn damage_smoke() -> ParticleEffect {
    let mut effect = ParticleEffect::new()
        .rate(30.0)
        .lifetime(3.0, 5.0)
        // Always up, however the jet is turned.
        .world_direction(Vector3::y())
        .cone(15.0)
        .speed(0.8, 1.5)
        .forces(Forces { gravity: -0.05, drag: 0.5, wind: 1.0, turbulence: 1.5 })
        .spin(-30.0, 30.0)
        .size_over_life(0.8, 4.0)
        .color_over_life([0.25, 0.25, 0.25, 0.6], [0.4, 0.4, 0.4, 0.0])
        .lit();
    effect.intensity_scales = IntensityScales { rate: true, alpha: true, size: true, speed: false };
    effect
}

/// Flames on a surface that's lost about half its HP or more - the engine's
/// particles example's fire (examples/3D/particles.rs), rising off the skin
/// under the smoke.
pub fn damage_fire() -> ParticleEffect {
    let mut effect = ParticleEffect::new()
        .rate(150.0)
        .lifetime(0.6, 1.2)
        .shape(EmissionShape::Disk { radius: 0.5 })
        // Up, however the jet is turned (the example's fire node faces up).
        .world_direction(Vector3::y())
        .cone(10.0)
        .speed(1.5, 3.0)
        // Rises.
        .gravity(-0.3)
        .size_over_life(1.2, 0.3)
        .color_over_life([3.0, 1.2, 0.3, 1.0], [0.6, 0.1, 0.0, 0.0])
        .brightness(2.0)
        .additive();
    effect.intensity_scales = IntensityScales { rate: true, alpha: true, size: true, speed: false };
    effect
}

/// The plane's emitters: every one its data.ron's `effects` lists, a
/// tyre-smoke one per wheel in its `gear` (placed on the wheel's contact
/// each frame), and smoke plus fire on every lifting surface (off until
/// that surface is damaged). All start off - `Plane::update_effects`
/// switches them on.
pub fn plane_emitters(effects: &[EffectSpec], gear: &GearSpec, wings: &[WingSpec]) -> ParticleEmitters {
    let mut emitters = ParticleEmitters::new();
    for spec in effects {
        let emitter = match spec.effect {
            EffectKind::WingtipVortex => Emitter::trail(wingtip_vortex()).intensity(0.0),
            EffectKind::WreckSmoke => {
                // And its fire, once the wreck's at rest.
                emitters.insert(wreck_fire_name(&spec.name), Emitter::particles(wreck_fire()).at(spec.position).intensity(0.0).disabled());
                Emitter::particles(wreck_smoke())
            }
        };
        emitters.insert(spec.name.clone(), emitter.at(spec.position).disabled());
    }
    for wheel in &gear.wheels {
        emitters.insert(tyre_smoke_name(&wheel.mesh), Emitter::particles(tyre_smoke()).at(wheel.position).intensity(0.0).disabled());
    }
    for wing in wings {
        emitters.insert(damage_smoke_name(&wing.label), Emitter::particles(damage_smoke()).at(wing.pressure_center).intensity(0.0).disabled());
        // Drawn over its smoke, as in the example.
        emitters.insert(damage_fire_name(&wing.label), Emitter::particles(damage_fire()).at(wing.pressure_center).sort_offset(-2.0).intensity(0.0).disabled());
    }
    emitters
}