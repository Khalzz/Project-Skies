use nalgebra::{Point3, Vector3};
use rapier3d::prelude::RigidBody;

use crate::engine::physics::physics_behavior::{DebugDraw, PhysicsBehavior, PhysicsCtx, PhysicsPayload};

// Water, for the forces below.
const WATER_DENSITY: f32 = 1000.0;
const GRAVITY: f32 = 9.81;
/// A contact point counts as fully in the water this deep (m) - shallower,
/// its buoyancy and drag scale down with how deep it is.
const POINT_FULL_DEPTH: f32 = 1.0;
/// Pressure drag on the box's area facing the flow - a flat plate is ~1.2.
const DRAG_COEFFICIENT: f32 = 1.0;
/// Friction drag on the box's wetted skin - what slows a belly skidding
/// along the surface, where the area facing the flow is small.
const SKIN_FRICTION_COEFFICIENT: f32 = 0.05;
/// Most a point's drag may do in one step: stop 90% of its own motion
/// through the water. Water drag at crash speed is huge - uncapped, one
/// step would fling the point backward instead of stopping it.
const DRAG_IMPULSE_CAP: f32 = 0.9;
/// How much of its boxes' full displacement a body floats on. The collider
/// boxes are much fuller than a real airframe (which is mostly not sealed
/// air), so at 1.0 it would bob on top like a cork - lower sits it deeper
/// and lets it go under sooner once flooding starts.
const BUOYANCY_SCALE: f32 = 0.35;
/// While any point is in the water, the body can't rise faster than this
/// (m/s) - enough to bob and float back up, never to be launched out. A
/// hop's height goes with its square (1.4 m/s ≈ 10 cm).
const MAX_RISE_SPEED: f32 = 1.4;
/// Once every point is this deep (m), the body stops where it is - resting
/// down there instead of sinking forever.
const SUNK_DEPTH: f32 = 20.0;

const DEBUG_DRAW_SCALE: f32 = 1.0 / 20000.0;
const DEBUG_COLOR: [f32; 3] = [0.2, 0.6, 1.0];

/// Where and how hard something first hit the water.
#[derive(Clone, Copy)]
pub struct WaterImpact {
    /// World position of the first point that went in.
    pub point: Vector3<f32>,
    /// Which of the body's colliders it belongs to, in the order they were
    /// declared on the node.
    pub collider: usize,
    /// How fast that point was moving (m/s).
    pub speed: f32,
}

/// What `WaterContact` publishes to the main thread (`Node::physics_state`).
#[derive(Clone, Copy, Default)]
pub struct WaterContactState {
    /// Set the first time any point touches the water, never cleared.
    pub impact: Option<WaterImpact>,
    /// Contact points under the water this step.
    pub submerged_points: u32,
    /// How much it still floats, 1 = fully .. 0 = flooded (see `sinks`).
    pub buoyancy: f32,
    /// Sunk to SUNK_DEPTH and stopped there.
    pub sunk: bool,
}

/// A rigidbody meeting the (flat) sea: every corner of each of its box
/// colliders is a contact point, and each one that's under the water gets
///   - buoyancy, pushing it up, by the water its share of the box displaces;
///   - drag, against its own motion through the water, by the box's area
///     facing that motion plus skin friction.
/// Both act AT the point, so a wingtip clipping the water gets yanked back
/// right there and swings the whole body round it, while a belly landing
/// spreads over the bottom corners. Colliders that aren't boxes are ignored.
///
/// Add it after any behavior that resets the body's forces (it uses
/// impulses, so the order only matters for who sees whose velocity).
pub struct WaterContact {
    sea_level: f32,
    /// (float, sink) seconds - see `sinks`.
    sinking: Option<(f32, f32)>,
    time_since_impact: f32,
    state: WaterContactState,
    /// Last step's (point, force) per submerged point, for the F2 overlay.
    debug_forces: Vec<(Vector3<f32>, Vector3<f32>)>,
}

impl WaterContact {
    /// The sea's flat surface is at height `sea_level`.
    pub fn new(sea_level: f32) -> Self {
        Self {
            sea_level,
            sinking: None,
            time_since_impact: 0.0,
            state: WaterContactState { buoyancy: 1.0, ..Default::default() },
            debug_forces: Vec::new(),
        }
    }

    /// Floats for `float_seconds` after first touching the water, then
    /// floods over `sink_seconds` (buoyancy fading to nothing) and goes
    /// under. Without this it floats forever.
    pub fn sinks(mut self, float_seconds: f32, sink_seconds: f32) -> Self {
        self.sinking = Some((float_seconds, sink_seconds.max(0.001)));
        self
    }

    /// How much buoyancy is left `time_since_impact` after first touching.
    fn buoyancy(&self) -> f32 {
        match (self.sinking, self.state.impact) {
            (Some((float_seconds, sink_seconds)), Some(_)) => 1.0 - ((self.time_since_impact - float_seconds) / sink_seconds).clamp(0.0, 1.0),
            _ => 1.0,
        }
    }
}

/// One box corner, in world space, with its share of the box.
struct ContactPoint {
    position: Vector3<f32>,
    collider: usize,
    /// The box's half extents and rotation, to find its area facing a flow.
    half_extents: Vector3<f32>,
    rotation: nalgebra::UnitQuaternion<f32>,
}

impl ContactPoint {
    /// This corner's eighth of the box's area seen from `direction` (world).
    fn facing_area(&self, direction: Vector3<f32>) -> f32 {
        let local = self.rotation.inverse() * direction;
        let h = self.half_extents;
        // A box's silhouette along unit `local` is 4(hy·hz|x| + hx·hz|y| + hx·hy|z|).
        0.5 * (h.y * h.z * local.x.abs() + h.x * h.z * local.y.abs() + h.x * h.y * local.z.abs())
    }

    /// This corner's eighth of the box's whole surface.
    fn skin_area(&self) -> f32 {
        let h = self.half_extents;
        h.x * h.y + h.y * h.z + h.x * h.z
    }

    /// This corner's eighth of the box's volume.
    fn volume(&self) -> f32 {
        let h = self.half_extents;
        h.x * h.y * h.z
    }
}

/// How much mass resists an impulse along unit `direction` at `point` -
/// low at a wingtip (it just spins the body), up to the whole mass at the
/// center of mass.
fn effective_mass(body: &RigidBody, point: Vector3<f32>, direction: Vector3<f32>) -> f32 {
    let mass = body.mass();
    if mass <= 0.0 {
        return 0.0;
    }
    let mass_properties = body.mass_properties();
    let arm = point - mass_properties.world_com.coords;
    let angular = mass_properties.effective_world_inv_inertia_sqrt * arm.cross(&direction);
    1.0 / (1.0 / mass + angular.norm_squared())
}

impl PhysicsBehavior for WaterContact {
    fn fixed_update(&mut self, ctx: &mut PhysicsCtx, dt: f32) {
        self.debug_forces.clear();
        if self.state.sunk {
            return;
        }

        // Every box corner, in world space.
        let body = ctx.rigidbody();
        let body_pose = *body.position();
        let mut points = Vec::new();
        for (index, handle) in body.colliders().iter().enumerate() {
            let Some(collider) = ctx.colliders.get(*handle) else { continue };
            let Some(cuboid) = collider.shape().as_cuboid() else { continue };
            let pose = match collider.position_wrt_parent() {
                Some(relative) => body_pose * relative,
                None => *collider.position(),
            };
            let h = cuboid.half_extents;
            for corner in 0..8 {
                let local = Point3::new(
                    if corner & 1 == 0 { -h.x } else { h.x },
                    if corner & 2 == 0 { -h.y } else { h.y },
                    if corner & 4 == 0 { -h.z } else { h.z },
                );
                points.push(ContactPoint { position: (pose * local).coords, collider: index, half_extents: h, rotation: pose.rotation });
            }
        }

        if self.state.impact.is_some() {
            self.time_since_impact += dt;
        }
        let buoyancy = self.buoyancy();
        self.state.buoyancy = buoyancy;

        let rigidbody = ctx.rigidbody_mut();
        let mut submerged = 0;
        let mut all_sunk = !points.is_empty();
        for point in &points {
            let depth = self.sea_level - point.position.y;
            all_sunk &= depth > SUNK_DEPTH;
            if depth <= 0.0 {
                continue;
            }
            submerged += 1;
            let wet = (depth / POINT_FULL_DEPTH).min(1.0);
            let at = Point3::from(point.position);

            if self.state.impact.is_none() {
                self.state.impact = Some(WaterImpact {
                    point: point.position,
                    collider: point.collider,
                    speed: rigidbody.velocity_at_point(&at).magnitude(),
                });
            }

            // Buoyancy - the weight of the water its share of the box displaces.
            let lift = WATER_DENSITY * GRAVITY * point.volume() * wet * buoyancy * BUOYANCY_SCALE;
            rigidbody.apply_impulse_at_point(Vector3::y() * lift * dt, at, true);

            // Drag against its motion through the water (read after the
            // impulses so far, so each point sees the others' effect).
            let velocity = rigidbody.velocity_at_point(&at);
            let speed = velocity.magnitude();
            let mut force = Vector3::y() * lift;
            if speed > 1e-3 {
                let direction = velocity / speed;
                let area = DRAG_COEFFICIENT * point.facing_area(direction) + SKIN_FRICTION_COEFFICIENT * point.skin_area();
                let drag = 0.5 * WATER_DENSITY * area * speed * speed * wet;
                let max_impulse = DRAG_IMPULSE_CAP * speed * effective_mass(rigidbody, point.position, direction);
                let impulse = (drag * dt).min(max_impulse);
                rigidbody.apply_impulse_at_point(-direction * impulse, at, true);
                force -= direction * (impulse / dt);
            }
            self.debug_forces.push((point.position, force));
        }
        self.state.submerged_points = submerged;

        // In the water it can't be thrown back up (see MAX_RISE_SPEED) - so
        // it gets grabbed and stays down instead of skipping.
        if submerged > 0 {
            let mut linvel = *rigidbody.linvel();
            if linvel.y > MAX_RISE_SPEED {
                linvel.y = MAX_RISE_SPEED;
                rigidbody.set_linvel(linvel, true);
            }
        }

        // All the way down - stop and stay there.
        if all_sunk {
            rigidbody.set_linvel(Vector3::zeros(), false);
            rigidbody.set_angvel(Vector3::zeros(), false);
            rigidbody.set_gravity_scale(0.0, false);
            rigidbody.reset_forces(false);
            rigidbody.reset_torques(false);
            self.state.sunk = true;
        }
    }

    fn publish(&self) -> Option<PhysicsPayload> {
        Some(Box::new(self.state))
    }

    /// Each submerged point's buoyancy + drag, as an arrow from the point.
    fn debug_draw(&self, _body: &RigidBody, draw: &mut DebugDraw) {
        for (point, force) in &self.debug_forces {
            draw.ray(*point, force * DEBUG_DRAW_SCALE, DEBUG_COLOR);
        }
    }
}
