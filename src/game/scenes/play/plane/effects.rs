//! The plane's particle effects - attached as the node's `ParticleEmitters`
//! (see play::scene::spawn_world) and driven by `Plane` (see
//! `Plane::update_effects`) - plus the spray it kicks up off the water,
//! which lives on its own node on the sea (see `spray_emitters`).

use nalgebra::Vector3;

use crate::engine::particles::{Curve, Emitter, Gradient, EmitterKind, EmissionShape, Forces, IntensityScales, ParticleEffect, ParticleEmitters, TrailEffect};

/// Wingtips, in the plane's frame (m) - where the vortices come off.
const LEFT_WINGTIP: Vector3<f32> = Vector3::new(-4.51, 0.23, 0.36);
const RIGHT_WINGTIP: Vector3<f32> = Vector3::new(4.51, 0.23, 0.36);

pub const LEFT_VORTEX: &str = "left_vortex";
pub const RIGHT_VORTEX: &str = "right_vortex";
pub const WRECK_SMOKE: &str = "wreck_smoke";

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
    ParticleEffect::new()
        .rate(30.0)
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
        .max_particles(300)
}

/// Where the spray starts, in the spray node's frame (+Z = the plane's
/// heading) - the V's tip, just behind the point under the plane.
const SPRAY_ORIGIN: Vector3<f32> = Vector3::new(0.0, 0.0, -3.82);
/// The wake's V (water.wgsl's WAKE_V_SLOPE - keep in sync): its arms run
/// this many meters out to the side per meter back.
const V_SLOPE: f32 = 0.36;
/// How far back along the V the curtains reach (m) - their lifetime is
/// set from it and the plane's speed, so they're this long at any speed.
const CURTAIN_LENGTH: f32 = 45.0;
const LEFT_CURTAIN: &str = "left_curtain";
const RIGHT_CURTAIN: &str = "right_curtain";

/// A ridge of spray thrown up off the water along one arm of the wake's V -
/// a low-poly 3D trail (see TrailFacing::Ridge) laid from the V's tip, each
/// piece sliding out sideways (see `update_spray`) so the ridge lies right
/// along the arm. Each piece grows as it's left behind - slowly at first,
/// then shooting up toward the far end - and fades out there; its jagged
/// peak line breaks it into flat-shaded triangles, like the terrain.
pub fn spray_curtain() -> TrailEffect {
    let mut effect = TrailEffect::new()
        .ridge()
        .lifetime(0.5)
        // One facet every 3 m - chunky, low-poly.
        .segment_spacing(3.0)
        // Growing the whole way back, faster and faster - low at the plane,
        // towering at the far end.
        .width_curve(Curve::Keys(vec![(0.0, 0.3), (0.4, 3.0), (0.7, 9.0), (1.0, 25.0)]));
    // Solid most of the way, fading out only over the far end.
    effect.color = Gradient::new([(0.0, [0.9, 0.95, 1.0, 0.85]), (0.65, [0.9, 0.95, 1.0, 0.7]), (1.0, [0.9, 0.95, 1.0, 0.0])]);
    effect.wind = 0.0;
    effect.soft_fade_distance = 0.0;
    effect.intensity_scales = IntensityScales { rate: false, alpha: true, size: true, speed: false };
    effect
}

/// For the "water_spray" node - kept on the sea under the plane and turned
/// to its heading (see play::scene::update_water_wake), off until it's low
/// and fast enough to stir the water: the two curtains along the V.
pub fn spray_emitters() -> ParticleEmitters {
    ParticleEmitters::new()
        .add(LEFT_CURTAIN, Emitter::trail(spray_curtain()).at(SPRAY_ORIGIN).intensity(0.0).disabled())
        .add(RIGHT_CURTAIN, Emitter::trail(spray_curtain()).at(SPRAY_ORIGIN).intensity(0.0).disabled())
}

/// Every frame: how hard the water is being stirred (the wake's strength,
/// 0..1) and how fast the plane is going over it (m/s, horizontal). The
/// curtains' pieces slide out sideways at V_SLOPE x that speed - while the
/// plane pulls ahead, each piece stays on the V's arm - and live just long
/// enough for the curtains to reach CURTAIN_LENGTH back.
pub fn update_spray(emitters: &mut ParticleEmitters, strength: f32, plane_speed: f32) {
    for (name, emitter) in emitters.iter_mut() {
        emitter.intensity = strength;
        emitter.enabled = strength > 0.02;
        let side = match name {
            LEFT_CURTAIN => -1.0,
            RIGHT_CURTAIN => 1.0,
            _ => continue,
        };
        if let EmitterKind::Trail(effect) = &mut emitter.kind {
            effect.point_velocity = Vector3::new(side * V_SLOPE * plane_speed, 0.0, 0.0);
            // The floor is below CURTAIN_LENGTH / top speed, so the length
            // holds even at full speed (it only kicks in past ~1100 m/s).
            effect.lifetime = (CURTAIN_LENGTH / plane_speed.max(1.0)).clamp(0.04, 1.5);
        }
    }
}

pub fn plane_emitters() -> ParticleEmitters {
    ParticleEmitters::new()
        .add(LEFT_VORTEX, Emitter::trail(wingtip_vortex()).at(LEFT_WINGTIP).intensity(0.0).disabled())
        .add(RIGHT_VORTEX, Emitter::trail(wingtip_vortex()).at(RIGHT_WINGTIP).intensity(0.0).disabled())
        .add(WRECK_SMOKE, Emitter::particles(wreck_smoke()).disabled())
}
