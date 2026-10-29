//! The open scene's physics - `app.physics`. It lives on the main thread
//! with everything else, and steps at a fixed rate from `App::run`: each
//! frame runs as many steps as the time that passed calls for (none, one,
//! several), each one right after every node's `Behavior::fixed_update`. The
//! solver inside a step spreads its work over the CPU's cores (Rapier's
//! `parallel` feature).
//!
//! Behaviors reach it directly - `app.physics.body_mut(&node.id)` to push a
//! body, `app.physics.cast_ray(..)` to ask what's where - from any hook. A
//! body's forces and velocities belong in `fixed_update` (they then act the
//! same at any frame rate); reading where things are, or asking the world,
//! works from `update` too.
//!
//! Nodes are drawn where their body is right now: between the last two
//! steps, by how far the frame is toward the next one (see `render_pose`) -
//! so a body moves smoothly at any frame rate, without ever being drawn
//! somewhere it hasn't really been.

use std::collections::HashMap;

use nalgebra::{Isometry3, Point3, UnitQuaternion, Vector3};
use rapier3d::prelude::{
    CCDSolver, Collider, ColliderSet, DefaultBroadPhase, ImpulseJointSet, IntegrationParameters, IslandManager, MultibodyJointSet, NarrowPhase,
    PhysicsPipeline, QueryFilter, QueryPipeline, Ray, RigidBody, RigidBodyHandle, RigidBodySet, RigidBodyType, Shape,
};

use crate::physics::physics_resources::{load_physics_from_definitions, PhysicsObjectDef};
use crate::rendering::lines::Lines;

/// What a ray hit - see `PhysicsWorld::cast_ray`.
#[derive(Debug, Clone)]
pub struct RayHit {
    /// The node whose body was hit - None for a collider without one.
    pub node: Option<String>,
    /// How far along the ray, in meters (the ray's direction is normalized).
    pub distance: f32,
    pub point: Vector3<f32>,
    /// The way the surface faces where it was hit.
    pub normal: Vector3<f32>,
}

/// See the module doc.
pub struct PhysicsWorld {
    /// Every body, and every collider - Rapier's own, for anything the
    /// helpers here don't cover.
    pub bodies: RigidBodySet,
    pub colliders: ColliderSet,
    /// Joints between bodies.
    pub impulse_joints: ImpulseJointSet,
    pub multibody_joints: MultibodyJointSet,
    /// Contacts from the last step.
    pub narrow_phase: NarrowPhase,
    /// What's where, as of the last step - for rays and shape casts.
    pub query: QueryPipeline,
    pipeline: PhysicsPipeline,
    islands: IslandManager,
    broad_phase: DefaultBroadPhase,
    ccd: CCDSolver,
    integration: IntegrationParameters,
    gravity: Vector3<f32>,
    /// Each node's body, by node id - and the other way round.
    handles: HashMap<String, RigidBodyHandle>,
    owners: HashMap<RigidBodyHandle, String>,
    /// Where each moving body was before the last step - `render_pose`
    /// draws it between there and where it is now.
    previous: HashMap<RigidBodyHandle, Isometry3<f32>>,
    /// Scaled time passed that no step has used up yet.
    accumulator: f32,
    running: bool,
    /// Whether the physics debug view is on (`app.renderer.debug.physics`) -
    /// see `debug_lines`.
    debug_view: bool,
    lines: Lines,
}

impl PhysicsWorld {
    /// The most steps one frame runs - after a stall (a debugger break, the
    /// PC freezing a moment, steps slower than real time), time beyond this
    /// is dropped instead of made up: catching it all up would take longer
    /// still, and fall further behind every frame.
    const MAX_STEPS_PER_FRAME: u32 = 5;

    pub(crate) fn new() -> Self {
        Self {
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            narrow_phase: NarrowPhase::new(),
            query: QueryPipeline::new(),
            pipeline: PhysicsPipeline::new(),
            islands: IslandManager::new(),
            broad_phase: DefaultBroadPhase::new(),
            ccd: CCDSolver::new(),
            integration: IntegrationParameters { dt: 1.0 / 120.0, ..IntegrationParameters::default() },
            gravity: Vector3::new(0.0, -9.81, 0.0),
            handles: HashMap::new(),
            owners: HashMap::new(),
            previous: HashMap::new(),
            accumulator: 0.0,
            running: false,
            debug_view: false,
            lines: Lines::default(),
        }
    }

    /// Starts the open scene's physics from nothing: its `bodies`, with the
    /// gravity and timestep set now.
    pub(crate) fn start(&mut self, bodies: Vec<PhysicsObjectDef>) {
        self.stop();
        self.running = true;
        for def in bodies {
            self.add_body(def);
        }
    }

    /// Drops every body - the scene's closing. The gravity and timestep stay.
    pub(crate) fn stop(&mut self) {
        let (gravity, timestep) = (self.gravity, self.integration.dt);
        *self = Self::new();
        self.gravity = gravity;
        self.integration.dt = timestep;
    }

    /// Whether the open scene's physics has started - from when the scene
    /// starts until it closes.
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// How long one step is, in seconds - what a `fixed_update` moves
    /// things by. 1/120 by default.
    pub fn timestep(&self) -> f32 {
        self.integration.dt
    }

    /// Sets how long one step is, in seconds (1/1000 at the least) - for
    /// every scene, from the next step on. Shorter steps are steadier for
    /// fast things, and cost more.
    pub fn set_timestep(&mut self, timestep: f32) {
        self.integration.dt = timestep.max(1.0 / 1000.0);
    }

    /// The world's gravity, in m/s² - what every body falls by (times its
    /// own `RigidBodyData::gravity_scale`). (0, -9.81, 0) by default.
    pub fn gravity(&self) -> Vector3<f32> {
        self.gravity
    }

    /// Sets the world's gravity - for every scene, from the next step on,
    /// until it's set again. Particles fall by it too.
    pub fn set_gravity(&mut self, gravity: Vector3<f32>) {
        self.gravity = gravity;
    }

    /// The body of node `id`.
    pub fn body(&self, id: &str) -> Option<&RigidBody> {
        self.handles.get(id).and_then(|handle| self.bodies.get(*handle))
    }

    pub fn body_mut(&mut self, id: &str) -> Option<&mut RigidBody> {
        self.handles.get(id).and_then(|handle| self.bodies.get_mut(*handle))
    }

    /// The handle of node `id`'s body - for Rapier's own calls (joints,
    /// query filters).
    pub fn handle(&self, id: &str) -> Option<RigidBodyHandle> {
        self.handles.get(id).copied()
    }

    /// The node whose body `handle` is.
    pub fn owner(&self, handle: RigidBodyHandle) -> Option<&str> {
        self.owners.get(&handle).map(String::as_str)
    }

    /// The first thing a ray from `origin` along `direction` hits within
    /// `max_distance` - skipping node `ignore`'s body (the one casting it,
    /// usually). As of the last step.
    pub fn cast_ray(&self, origin: Vector3<f32>, direction: Vector3<f32>, max_distance: f32, ignore: Option<&str>) -> Option<RayHit> {
        let direction = direction.try_normalize(1e-6)?;
        let mut filter = QueryFilter::default().exclude_sensors();
        if let Some(handle) = ignore.and_then(|id| self.handle(id)) {
            filter = filter.exclude_rigid_body(handle);
        }
        let ray = Ray::new(Point3::from(origin), direction);
        let (collider, hit) = self.query.cast_ray_and_get_normal(&self.bodies, &self.colliders, &ray, max_distance, true, filter)?;
        let node = self.colliders.get(collider).and_then(Collider::parent).and_then(|body| self.owner(body)).map(str::to_owned);
        Some(RayHit { node, distance: hit.time_of_impact, point: origin + direction * hit.time_of_impact, normal: hit.normal })
    }

    /// Puts node `id`'s body at `position`, turned `rotation`, stopped -
    /// drawn there right away, not slid there from where it was.
    pub fn teleport(&mut self, id: &str, position: Vector3<f32>, rotation: UnitQuaternion<f32>) {
        let Some(handle) = self.handle(id) else { return };
        let Some(body) = self.bodies.get_mut(handle) else { return };
        body.set_position(Isometry3::from_parts(position.into(), rotation), true);
        body.set_linvel(Vector3::zeros(), true);
        body.set_angvel(Vector3::zeros(), true);
        self.previous.insert(handle, *body.position());
    }

    /// Holds node `id`'s body still - nothing pushes it, it doesn't fall,
    /// whatever moves its node moves it (`RigidBody::set_next_kinematic_position`)
    /// - or lets it go again. A static body stays static.
    pub fn set_kinematic(&mut self, id: &str, kinematic: bool) {
        let Some(body) = self.body_mut(id).filter(|body| !body.is_fixed()) else { return };
        let body_type = if kinematic { RigidBodyType::KinematicPositionBased } else { RigidBodyType::Dynamic };
        body.set_body_type(body_type, true);
        if kinematic {
            body.set_linvel(Vector3::zeros(), true);
            body.set_angvel(Vector3::zeros(), true);
        }
    }

    /// How much of the world's gravity node `id`'s body feels from now on -
    /// see `RigidBodyData::gravity_scale`.
    pub fn set_gravity_scale(&mut self, id: &str, scale: f32) {
        if let Some(body) = self.body_mut(id) {
            body.set_gravity_scale(scale, true);
        }
    }

    /// Lines for the physics debug view, in world space - `None` while the
    /// view is off (`app.renderer.debug.physics`), so nothing's drawn for
    /// nobody. Emptied every frame: draw them again every frame.
    pub fn debug_lines(&mut self) -> Option<&mut Lines> {
        self.debug_view.then_some(&mut self.lines)
    }

    pub(crate) fn lines(&self) -> &Lines {
        &self.lines
    }

    /// A node's body - replacing any it had.
    pub(crate) fn add_body(&mut self, def: PhysicsObjectDef) {
        let id = def.id.clone();
        self.remove_body(&id);
        let mut loaded = HashMap::new();
        load_physics_from_definitions(std::slice::from_ref(&def), &mut self.colliders, &mut self.bodies, &mut loaded);
        if let Some(Some(data)) = loaded.remove(&id) {
            self.owners.insert(data.rigidbody_handle, id.clone());
            self.handles.insert(id, data.rigidbody_handle);
        }
    }

    /// Drops node `id`'s body, with its colliders - nothing if it has none.
    pub(crate) fn remove_body(&mut self, id: &str) {
        let Some(handle) = self.handles.remove(id) else { return };
        self.owners.remove(&handle);
        self.previous.remove(&handle);
        self.bodies.remove(handle, &mut self.islands, &mut self.colliders, &mut self.impulse_joints, &mut self.multibody_joints, true);
    }

    /// Adds a frame's `elapsed` (scaled) seconds, and says how many steps
    /// are due now.
    pub(crate) fn steps_due(&mut self, elapsed: f32, debug_view: bool) -> u32 {
        self.debug_view = debug_view;
        self.lines.clear();
        if !self.running {
            return 0;
        }
        let dt = self.integration.dt;
        self.accumulator = (self.accumulator + elapsed.max(0.0)).min(dt * Self::MAX_STEPS_PER_FRAME as f32);
        let steps = (self.accumulator / dt) as u32;
        self.accumulator -= steps as f32 * dt;
        steps
    }

    /// Advances the world by one step.
    pub(crate) fn step(&mut self) {
        // Where the moving bodies are before it - drawn between there and
        // after (see render_pose).
        self.previous.clear();
        for (handle, body) in self.bodies.iter().filter(|(_, body)| !body.is_fixed()) {
            self.previous.insert(handle, *body.position());
        }
        self.pipeline.step(
            &self.gravity,
            &self.integration,
            &mut self.islands,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.ccd,
            Some(&mut self.query),
            &(),
            &(),
        );
    }

    /// Where `handle`'s body is drawn this frame: between where it was before
    /// the last step and where it is now, by how much of the next step's time
    /// has passed already.
    fn render_pose(&self, handle: RigidBodyHandle, body: &RigidBody) -> Isometry3<f32> {
        let current = *body.position();
        match self.previous.get(&handle) {
            Some(previous) => previous.lerp_slerp(&current, (self.accumulator / self.integration.dt).clamp(0.0, 1.0)),
            None => current,
        }
    }

    /// Every moving body's node id and where it's drawn this frame (see
    /// `render_pose`).
    pub(crate) fn render_poses(&self) -> impl Iterator<Item = (&str, Isometry3<f32>)> {
        self.handles.iter().filter_map(|(id, handle)| {
            let body = self.bodies.get(*handle).filter(|body| !body.is_fixed())?;
            Some((id.as_str(), self.render_pose(*handle, body)))
        })
    }

    /// Every moving body's colliders, into the debug lines where they're
    /// drawn this frame - static ones (the ground) skipped, they'd bury the
    /// screen. Only while the debug view is on.
    pub(crate) fn draw_colliders(&mut self) {
        if !self.debug_view {
            return;
        }
        const COLLIDER_COLOR: [f32; 3] = [1.0, 0.55, 0.1];
        let mut lines = std::mem::take(&mut self.lines);
        for (handle, body) in self.bodies.iter().filter(|(_, body)| !body.is_fixed()) {
            let pose = self.render_pose(handle, body);
            for collider in body.colliders().iter().filter_map(|collider| self.colliders.get(*collider)).filter(|collider| collider.is_enabled()) {
                let local = collider.position_wrt_parent().copied().unwrap_or_else(Isometry3::identity);
                draw_shape(&mut lines, &(pose * local), collider.shape(), COLLIDER_COLOR);
            }
        }
        self.lines = lines;
    }
}

/// `shape` at `pose` into `lines` - boxes as their 12 edges, balls as 3
/// circles, cylinders and capsules as their circles and 4 sides. Other
/// shapes aren't drawn.
pub fn draw_shape(lines: &mut Lines, pose: &Isometry3<f32>, shape: &dyn Shape, color: [f32; 3]) {
    let world = |local: Vector3<f32>| (pose * Point3::from(local)).coords;

    if let Some(cuboid) = shape.as_cuboid() {
        let h = cuboid.half_extents;
        let corner = |x: f32, y: f32, z: f32| world(Vector3::new(h.x * x, h.y * y, h.z * z));
        for (sy, sz) in [(-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)] {
            lines.line(corner(-1.0, sy, sz), corner(1.0, sy, sz), color);
        }
        for (sx, sz) in [(-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)] {
            lines.line(corner(sx, -1.0, sz), corner(sx, 1.0, sz), color);
        }
        for (sx, sy) in [(-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)] {
            lines.line(corner(sx, sy, -1.0), corner(sx, sy, 1.0), color);
        }
    } else if let Some(ball) = shape.as_ball() {
        let r = ball.radius;
        circle(lines, &world, Vector3::zeros(), Vector3::x() * r, Vector3::y() * r, color);
        circle(lines, &world, Vector3::zeros(), Vector3::y() * r, Vector3::z() * r, color);
        circle(lines, &world, Vector3::zeros(), Vector3::z() * r, Vector3::x() * r, color);
    } else if let Some(cylinder) = shape.as_cylinder() {
        let (r, h) = (cylinder.radius, cylinder.half_height);
        for y in [-h, h] {
            circle(lines, &world, Vector3::y() * y, Vector3::x() * r, Vector3::z() * r, color);
        }
        for side in [Vector3::x(), -Vector3::x(), Vector3::z(), -Vector3::z()] {
            lines.line(world(side * r - Vector3::y() * h), world(side * r + Vector3::y() * h), color);
        }
    } else if let Some(capsule) = shape.as_capsule() {
        // Its two ball centers, wherever along its axis they are.
        let (a, b, r) = (capsule.segment.a.coords, capsule.segment.b.coords, capsule.radius);
        let axis = (b - a).try_normalize(1e-6).unwrap_or_else(Vector3::y);
        let side_a = axis.cross(&Vector3::x()).try_normalize(1e-6).unwrap_or_else(|| axis.cross(&Vector3::z()).normalize());
        let side_b = axis.cross(&side_a);
        for center in [a, b] {
            circle(lines, &world, center, side_a * r, side_b * r, color);
            circle(lines, &world, center, side_a * r, axis * r, color);
            circle(lines, &world, center, side_b * r, axis * r, color);
        }
        for side in [side_a, -side_a, side_b, -side_b] {
            lines.line(world(a + side * r), world(b + side * r), color);
        }
    }
}

/// A circle in local space (center + cos*a + sin*b), mapped by `world`.
fn circle(lines: &mut Lines, world: &dyn Fn(Vector3<f32>) -> Vector3<f32>, center: Vector3<f32>, a: Vector3<f32>, b: Vector3<f32>, color: [f32; 3]) {
    const SEGMENTS: usize = 24;
    let point = |i: usize| {
        let angle = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        world(center + a * angle.cos() + b * angle.sin())
    };
    for i in 0..SEGMENTS {
        lines.line(point(i), point(i + 1), color);
    }
}
