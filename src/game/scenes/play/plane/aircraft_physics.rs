use std::sync::atomic::{AtomicBool, Ordering};

use nalgebra::{Point3, Vector3};
use rapier3d::prelude::RigidBody;

use crate::engine::physics::physics_behavior::{DebugDraw, PhysicsBehavior, PhysicsCtx, PhysicsPayload};
use crate::game::scenes::play::plane::aero_spec::AeroSpec;

use super::airframe::{dynamic_drag, Airframe};
use super::controls::PlaneControls;
use super::flight_system::FlightSystem;
use super::instrumentation::Instrumentation;
use super::messages::{AircraftEvent, AircraftReload, AircraftState, ProfileSwitch};
use crate::engine::game_nodes::game_object::{wing_collider_id, ColliderType};
use crate::engine::physics::physics_resources::compute_principal_inertia;
use super::physics::rolling_rate::{commanded_roll_rate_deg_s, RollRateParams};
use super::physics::wheels::wheel_manager::{WheelInputs, WheelManager};
use super::physics::wings::wing_manager::WingManager;

/// TESTING ONLY - the force-based aileron simulation (Wing::physics_force,
/// via wing_manager.update) still runs untouched; when on, this just
/// overwrites the roll AXIS COMPONENT of the resulting angular velocity
/// afterward, directly following rolling_rate::commanded_roll_rate_deg_s
/// (the same curve the F7 debug overlay plots). Pitch/yaw stay fully
/// force-driven. Off = pure force-based roll. Switchable at runtime from the
/// F7 "Wing Surfaces" window - an atomic since that's on the main thread and
/// this is read on the physics thread.
pub static KINEMATIC_ROLL: AtomicBool = AtomicBool::new(false);

// F5 overlay: how long a wing's lift arrow is per newton (m/N) - ~9 m for
// one main wing's share of level-flight lift, ~100 m at its force clamp.
const LIFT_DRAW_SCALE: f32 = 1.0 / 5000.0;
const LIFT_COLOR: [f32; 3] = [0.3, 1.0, 0.3];
const SUSPENSION_GROUNDED_COLOR: [f32; 3] = [1.0, 0.8, 0.2];
const SUSPENSION_AIRBORNE_COLOR: [f32; 3] = [0.5, 0.5, 0.5];
// Each wheel's tyre friction, drawn from its contact point - same scale as
// the lift arrows, so the two read against each other.
const TYRE_FORCE_DRAW_SCALE: f32 = LIFT_DRAW_SCALE;
const TYRE_FORCE_COLOR: [f32; 3] = [1.0, 0.3, 1.0];
// Wing geometry (F5): each wing's pressure center, its outline, and its
// hinged surfaces - from the live wings, so a data.ron reload shows at once.
const PRESSURE_CENTER_COLOR: [f32; 3] = [1.0, 1.0, 1.0];
const WING_OUTLINE_COLOR: [f32; 3] = [0.6, 0.6, 0.6];
const SURFACE_COLOR: [f32; 3] = [1.0, 0.9, 0.2];
// One step of the whole jet's weight is about 1,000 N·s. A contact past
// these is a strike, not the airframe resting on the surface.
/// Fuselage into the ground: the jet breaks up.
const FUSELAGE_STRIKE_IMPULSE: f32 = 8_000.0;
/// A wing, stabilator or fin into the ground: that surface is torn off.
const SURFACE_STRIKE_IMPULSE: f32 = 2_500.0;
const MARKER_SIZE: f32 = 0.6;
// The rigidbody's center of mass, and the pilot's seat (where seat G is
// measured).
const CENTER_OF_MASS_COLOR: [f32; 3] = [1.0, 0.2, 1.0];
const PILOT_SEAT_COLOR: [f32; 3] = [0.2, 1.0, 0.4];
// Nose-wheel heading, drawn from its contact point while on the ground.
const STEERING_DRAW_LENGTH: f32 = 4.0;
const STEERING_COLOR: [f32; 3] = [0.2, 0.9, 1.0];
// The speed-brake boards on the F5 overlay.
const SPEED_BRAKE_COLOR: [f32; 3] = [0.35, 0.75, 1.0];
// Particle emitters (the data.ron's `effects`): a cross with a tick up.
const EMITTER_COLOR: [f32; 3] = [1.0, 0.55, 0.1];
const EMITTER_TICK: f32 = 1.2;

/// An aircraft's physics half - its aero, thrust, wheels, landing gear and
/// instruments, run once per fixed physics step on the physics thread.
/// Attached to the plane's node with `add_physics_behavior`, next to its
/// main-thread half, `Plane`. See messages.rs for everything the two halves
/// exchange.
pub struct AircraftPhysics {
    wing_manager: WingManager,
    wheel_manager: WheelManager,
    flight_system: FlightSystem,
    airframe: Airframe,
    instrumentation: Instrumentation,
    /// See `AircraftEvent::Wreck`.
    wrecked: bool,
    /// See `AircraftEvent::ShotDown` - the stick roll its jammed surfaces
    /// hold (which way it rolls off).
    shot_down: Option<f32>,
    /// Where the plane's particle emitters sit (its data.ron `effects`,
    /// jet's frame) - only drawn, for the F5 overlay.
    effect_positions: Vec<Vector3<f32>>,
    /// The jet with its tanks full (kg) - the mass it was spawned with -
    /// and where that mass sits. Fuel burned comes off it.
    full_mass: f32,
    /// The additional mass last written onto the body. Starts as
    /// `full_mass`, so a report that the tanks are still full doesn't
    /// rebuild the mass properties (that kick, on the opening frame, was
    /// enough to put the jet in the water).
    applied_mass: f32,
    com: Vector3<f32>,
    colliders: Vec<ColliderType>,
    fuel_kg: f32,
    fuel_capacity: f32,
    /// Tanks ran dry - no thrust until fuel is added.
    engine_out: bool,
    /// A blast reached the pilot's seat. Never cleared.
    pilot_hit: bool,
    /// The fuselage has hit the ground hard enough to break the jet up.
    /// Stays set. A wing strike does not: that surface comes off instead.
    fuselage_struck: bool,
    /// Warheads applied. The main thread diffs part HP when this advances,
    /// so a ground strike or an F10 toggle is not credited as a hit.
    blasts_handled: u32,
}

impl AircraftPhysics {
    pub fn new(spec: &AeroSpec, engine: &super::engine::EngineSpec, gear: &super::gear_spec::GearSpec, effects: &[super::effects::EffectSpec], com: Vector3<f32>, colliders: Vec<ColliderType>, full_mass: f32, gear_down: bool) -> Self {
        Self {
            wing_manager: WingManager::new(spec),
            wheel_manager: WheelManager::new(gear),
            flight_system: FlightSystem::new(engine.clone()),
            airframe: Airframe::new(gear_down),
            instrumentation: Instrumentation::new(spec.pilot_position),
            wrecked: false,
            shot_down: None,
            effect_positions: effects.iter().map(|effect| effect.position).collect(),
            full_mass,
            applied_mass: full_mass,
            com,
            colliders,
            fuel_kg: 0.0,
            fuel_capacity: 0.0,
            engine_out: false,
            pilot_hit: false,
            fuselage_struck: false,
            blasts_handled: 0,
        }
    }

    /// data.ron edited mid-flight: the new wings, engine, wheels, pilot seat
    /// and mass, on the running jet - its motion carries on from wherever it
    /// was.
    fn apply_reload(&mut self, reload: AircraftReload, ctx: &mut PhysicsCtx) {
        self.wing_manager.set_wings(&reload.aero);
        self.wheel_manager.set_gear(&reload.gear);
        self.effect_positions = reload.effects.iter().map(|effect| effect.position).collect();
        self.flight_system.engine = reload.engine;
        self.instrumentation.set_pilot_position(reload.aero.pilot_position);
        self.full_mass = reload.mass;
        self.applied_mass = reload.mass;
        self.com = reload.center_of_mass;
        self.colliders = reload.colliders.clone();
        let inertia = compute_principal_inertia(reload.mass, reload.center_of_mass, &reload.colliders);
        let mass_properties = rapier3d::prelude::MassProperties::new(reload.center_of_mass.into(), reload.mass, inertia);
        ctx.rigidbody_mut().set_additional_mass_properties(mass_properties, true);
        self.apply_fuel_mass(ctx);
    }

    /// One profile on or off. On gives it back the HP it was defined with.
    fn set_profile(&mut self, label: &str, on: bool) {
        let Some(wing) = self.wing_manager.wings.iter_mut().find(|wing| wing.label == label) else { return };
        wing.hp = if on { wing.max_hp } else { 0.0 };
    }

    /// The body's mass with the fuel that's left. `full_mass` is the jet
    /// with the tanks full - the mass it spawned at, not whatever the body
    /// reports (that total also counts the colliders, and reading it back
    /// as the new mass made the jet heavier every report).
    fn apply_fuel(&mut self, kg: f32, capacity: f32, ctx: &mut PhysicsCtx) {
        self.fuel_kg = kg.max(0.0);
        self.fuel_capacity = capacity.max(0.0);
        if self.fuel_kg > 0.0 {
            self.engine_out = false;
        }
        self.apply_fuel_mass(ctx);
    }

    fn apply_fuel_mass(&mut self, ctx: &mut PhysicsCtx) {
        if self.fuel_capacity <= 0.0 || self.full_mass <= 0.0 {
            return;
        }
        let mass = (self.full_mass - (self.fuel_capacity - self.fuel_kg)).max(1.0);
        // Still the spawned weight: leave the body alone.
        if (mass - self.applied_mass).abs() < 0.5 {
            return;
        }
        self.applied_mass = mass;
        let inertia = compute_principal_inertia(mass, self.com, &self.colliders);
        let mass_properties = rapier3d::prelude::MassProperties::new(self.com.into(), mass, inertia);
        ctx.rigidbody_mut().set_additional_mass_properties(mass_properties, true);
    }

    fn handle_event(&mut self, event: AircraftEvent, ctx: &mut PhysicsCtx) {
        match event {
            AircraftEvent::ToggleGear => self.airframe.landing_gear.request_toggle(),
            AircraftEvent::GearUp => self.airframe.landing_gear.set_commanded_down(false),
            AircraftEvent::GearDown => self.airframe.landing_gear.set_commanded_down(true),
            AircraftEvent::ToggleSpeedBrake => self.airframe.speed_brake.request_toggle(),
            AircraftEvent::SpeedBrakeExtend => self.airframe.speed_brake.set_commanded_out(true),
            AircraftEvent::SpeedBrakeRetract => self.airframe.speed_brake.set_commanded_out(false),
            AircraftEvent::Wreck => {
                self.wrecked = true;
                self.flight_system.cut_engine();
                // Rapier keeps user forces until reset - drop the last
                // step's thrust and lift, since nothing will replace them.
                let rigidbody = ctx.rigidbody_mut();
                rigidbody.reset_forces(true);
                rigidbody.reset_torques(true);
            }
            AircraftEvent::ShotDown => {
                self.flight_system.cut_engine();
                // Rolls off the way it was already rolling (left if level).
                let rigidbody = ctx.rigidbody();
                let roll_rate = (rigidbody.rotation().inverse() * rigidbody.angvel()).z;
                self.shot_down = Some(if roll_rate < 0.0 { -SHOT_DOWN_ROLL } else { SHOT_DOWN_ROLL });
            }
            AircraftEvent::EngineOut => {
                self.engine_out = true;
                self.flight_system.cut_engine();
            }
            AircraftEvent::Fuel { kg, capacity } => self.apply_fuel(kg, capacity, ctx),
            AircraftEvent::Blast { at, radius, damage, impulse } => self.apply_blast(at, radius, damage, impulse, ctx),
        }
    }

    /// A warhead at `at`. Each wing loses HP by how far the blast is from
    /// its planform. The pilot dies when the seat is inside the sphere.
    /// The jet is shoved at the closest point on it, so a hit off the
    /// center also spins it.
    fn apply_blast(&mut self, at: Vector3<f32>, radius: f32, damage: f32, impulse: f32, ctx: &mut PhysicsCtx) {
        self.blasts_handled = self.blasts_handled.saturating_add(1);
        let rigidbody = ctx.rigidbody();
        let rotation = *rigidbody.rotation();
        let translation = *rigidbody.translation();
        let local = rotation.inverse() * (at - translation);
        let contact_local = self.blast_contact(local);
        for wing in &mut self.wing_manager.wings {
            wing.take_blast(local, radius, damage);
        }
        let seat = rotation * self.instrumentation.pilot_position() + translation;
        if (seat - at).magnitude() <= radius {
            self.pilot_hit = true;
        }
        self.shove(at, radius, impulse, rotation, translation, contact_local, ctx);
    }

    /// Closest point on the jet to a blast in the jet's frame: a wing
    /// planform or the fuselage box, whichever is nearer.
    fn blast_contact(&self, local: Vector3<f32>) -> Vector3<f32> {
        let mut best = self.com;
        let mut best_distance = (local - self.com).magnitude();
        for wing in &self.wing_manager.wings {
            let point = wing.closest_on_planform(local);
            let distance = (local - point).magnitude();
            if distance < best_distance {
                best = point;
                best_distance = distance;
            }
        }
        for collider in &self.colliders {
            let ColliderType::Cuboid { half_extents, position } = collider else { continue };
            let center = Vector3::new(position.0, position.1, position.2);
            let half = Vector3::new(half_extents.0, half_extents.1, half_extents.2);
            let point = closest_on_cuboid(local, center, half);
            let distance = (local - point).magnitude();
            if distance < best_distance {
                best = point;
                best_distance = distance;
            }
        }
        best
    }

    /// Pushes the jet away from the blast, at `contact_local`. Full
    /// `impulse` on the surface, nothing at `radius`.
    fn shove(&self, at: Vector3<f32>, radius: f32, impulse: f32, rotation: nalgebra::UnitQuaternion<f32>, translation: Vector3<f32>, contact_local: Vector3<f32>, ctx: &mut PhysicsCtx) {
        if impulse <= 0.0 || radius <= 0.0 {
            return;
        }
        let contact = rotation * contact_local + translation;
        let to_contact = contact - at;
        let distance = to_contact.magnitude();
        if distance >= radius {
            return;
        }
        let mut direction = to_contact;
        if direction.magnitude() < 1e-3 {
            direction = rotation * self.com + translation - at;
        }
        let length = direction.magnitude();
        if length < 1e-3 {
            return;
        }
        let push = direction / length * impulse * (1.0 - distance / radius);
        ctx.rigidbody_mut().apply_impulse_at_point(push, Point3::from(contact), true);
    }

    /// Thrust plus fuselage drag, applied fresh this step - forces are
    /// reset first, since rapier keeps user forces until told otherwise.
    /// Side, belly and axial drag are the body's own share; the wings and
    /// tails carry theirs in their airfoil tables.
    fn apply_body_forces(&mut self, ctx: &mut PhysicsCtx, controls: &PlaneControls, dt: f32, engine_alive: bool) {
        let rigidbody = ctx.rigidbody_mut();
        rigidbody.reset_forces(true);
        rigidbody.reset_torques(true);

        let rotation = *rigidbody.rotation();
        let thrust_force = if engine_alive {
            self.flight_system.compute_thrust(rigidbody.translation().y, rotation, controls.throttle, dt)
        } else {
            Vector3::zeros()
        };

        let local_velocity = rotation.inverse() * rigidbody.linvel();
        let air_density = 1.225f32;
        let fuselage_side_area = 20.0; // m² - approximate F-16 fuselage side profile
        let fuselage_cd = 1.2;         // bluff body drag coefficient
        let fuselage_side_force_mag = -0.5 * air_density * local_velocity.x * local_velocity.x.abs() * fuselage_side_area * fuselage_cd;
        // The wing and tail tables already carry about 0.012 of drag
        // coefficient on the 27.9 m² wing. This area brings a clean jet to
        // about 0.02 while the wind is on the nose.
        let fuselage_axial_area = 0.24;
        let fuselage_axial_mag = -0.5 * air_density * local_velocity.z * local_velocity.z.abs() * fuselage_axial_area;
        // Forebody and inlet, missing from the wing reference. With the
        // wings and the stalled stabilators this puts the whole jet near
        // 1.9 (on that wing area) when the belly is square to the wind.
        let fuselage_belly_area = 4.5;
        let fuselage_belly_mag = -0.5 * air_density * local_velocity.y * local_velocity.y.abs() * fuselage_belly_area * fuselage_cd;
        let fuselage_force = rotation * Vector3::new(fuselage_side_force_mag, fuselage_belly_mag, fuselage_axial_mag);

        // Gear and the speed brake: extra flat-plate area into the wind,
        // scaled by how far each is out. The gear's struts are the smaller
        // of the two; the boards are the larger.
        const GEAR_DRAG_AREA: f32 = 1.2;
        const SPEED_BRAKE_DRAG_AREA: f32 = 1.8;
        let drag_area = GEAR_DRAG_AREA * self.airframe.landing_gear.deploy + SPEED_BRAKE_DRAG_AREA * self.airframe.speed_brake.deploy;
        let drag = dynamic_drag(*rigidbody.linvel(), air_density, drag_area);

        rigidbody.add_force(thrust_force, true);
        rigidbody.add_force(fuselage_force, true);
        rigidbody.add_force(drag, true);
    }

    /// Undoes the rigidbody's blanket `angular_damping` (see
    /// `load_physics_from_definitions`) on pitch, yaw and roll. Rapier
    /// scales angvel by 1 / (1 + dt * damping) each step, so pre-scaling
    /// every axis by (1 + dt * damping) cancels it. Off on all three for
    /// the tailless-jet test; pitch and yaw used to keep it.
    fn cancel_angular_damping(&self, ctx: &mut PhysicsCtx, dt: f32) {
        let rigidbody = ctx.rigidbody_mut();
        let rotation = *rigidbody.rotation();
        let mut local_angvel = rotation.inverse() * rigidbody.angvel();
        let undo = 1.0 + dt * rigidbody.angular_damping();
        local_angvel.x *= undo;
        local_angvel.y *= undo;
        local_angvel.z *= undo;
        rigidbody.set_angvel(rotation * local_angvel, true);
    }

    /// See `KINEMATIC_ROLL`. Skipped with weight on wheels: the landing
    /// gear reacts the rolling moment into the runway, so the jet can't roll
    /// there no matter the aileron input.
    fn apply_kinematic_roll(&self, ctx: &mut PhysicsCtx, controls: &PlaneControls) {
        let on_ground = self.wheel_manager.renderizable_wheels.values().any(|wheel| wheel.grounded);
        if !KINEMATIC_ROLL.load(Ordering::Relaxed) || on_ground {
            return;
        }

        let rigidbody = ctx.rigidbody_mut();
        let rotation = *rigidbody.rotation();
        let true_airspeed_ms = rigidbody.linvel().magnitude();
        let altitude_m = rigidbody.translation().y;
        // Same aoa_y derivation as Instrumentation::update. Absolute value
        // since rolling_rate's own aoa_taper only treats positive/nose-up AoA
        // as authority-reducing, same as wing.rs's own use of it.
        let local_vel = rotation.inverse() * rigidbody.linvel();
        let aoa_deg = (-local_vel.y).atan2(local_vel.z).to_degrees().abs();

        let commanded_rate_rad_s = commanded_roll_rate_deg_s(controls.aileron, true_airspeed_ms, altitude_m, aoa_deg, &RollRateParams::default()).to_radians();

        // Decompose world angvel into body axes, replace only the roll (local
        // Z) component, recompose - direct/instant, no easing (ramp, lag and
        // lerp were all tried and reverted - exact agreement with the F7
        // chart's curve matters more here than spool-up feel).
        let local_angvel = rotation.inverse() * rigidbody.angvel();
        let new_local_angvel = Vector3::new(local_angvel.x, local_angvel.y, commanded_rate_rad_s);
        rigidbody.set_angvel(rotation * new_local_angvel, true);
    }

    /// Contacts against the ground (anything fixed) from the step that just
    /// ran. A hard hit on the fuselage breaks the jet up. The same hit on a
    /// wing, stabilator or fin tears that surface off and leaves the rest
    /// flying. The impulse is what the solver applied, so a strike still
    /// counts after it has already stopped the impact.
    fn ground_strikes(&mut self, ctx: &PhysicsCtx) {
        let handles: Vec<_> = ctx.rigidbody().colliders().iter().copied().collect();
        let mut hits: Vec<(u128, f32)> = Vec::new();
        for handle in handles {
            let (id, enabled) = {
                let Some(collider) = ctx.colliders.get(handle) else { continue };
                (collider.user_data, collider.is_enabled())
            };
            if !enabled {
                continue;
            }
            let mut impulse = 0.0;
            for pair in ctx.narrow_phase.contact_pairs_with(handle) {
                if !pair.has_any_active_contact {
                    continue;
                }
                let other = if pair.collider1 == handle { pair.collider2 } else { pair.collider1 };
                let Some(other_collider) = ctx.colliders.get(other) else { continue };
                let Some(other_body) = other_collider.parent().and_then(|body| ctx.bodies.get(body)) else { continue };
                if !other_body.is_fixed() {
                    continue;
                }
                for manifold in &pair.manifolds {
                    for point in &manifold.points {
                        if point.dist > 0.05 {
                            continue;
                        }
                        impulse += point.data.impulse.abs();
                    }
                }
            }
            if impulse > 0.0 {
                hits.push((id, impulse));
            }
        }

        for (id, impulse) in hits {
            if id == 0 {
                if impulse >= FUSELAGE_STRIKE_IMPULSE {
                    self.fuselage_struck = true;
                }
            } else if impulse >= SURFACE_STRIKE_IMPULSE {
                if let Some(wing) = self.wing_manager.wings.iter_mut().find(|wing| wing_collider_id(&wing.label) == id) {
                    wing.hp = 0.0;
                }
            }
        }
    }

    /// A surface at 0 HP no longer collides: its slab is switched off, and
    /// the F5 overlay skips disabled colliders. Turning the profile back on
    /// brings the slab back.
    fn sync_surface_colliders(&self, ctx: &mut PhysicsCtx) {
        let handles: Vec<_> = ctx.rigidbody().colliders().iter().copied().collect();
        for handle in handles {
            let Some(collider) = ctx.colliders.get_mut(handle) else { continue };
            let id = collider.user_data;
            if id == 0 {
                continue;
            }
            let alive = self.wing_manager.wings.iter().any(|wing| wing.hp > 0.0 && wing_collider_id(&wing.label) == id);
            collider.set_enabled(alive);
        }
    }
}

/// A shot-down jet's jammed surfaces: the stick roll they hold (either way
/// - see `AircraftEvent::ShotDown`) and pitch (+ is stick forward) - a
/// rolling dive.
const SHOT_DOWN_ROLL: f32 = 0.45;
const SHOT_DOWN_PITCH: f32 = 0.15;

impl PhysicsBehavior for AircraftPhysics {
    fn fixed_update(&mut self, ctx: &mut PhysicsCtx, dt: f32) {
        // Default (throttle idle, stick centred) until the main half has sent
        // its first controls.
        let controls = ctx.input::<PlaneControls>().cloned().unwrap_or_else(PlaneControls::new);
        let events: Vec<AircraftEvent> = ctx.events::<AircraftEvent>().copied().collect();
        for event in events {
            self.handle_event(event, ctx);
        }
        let reloads: Vec<AircraftReload> = ctx.events::<AircraftReload>().cloned().collect();
        for reload in reloads {
            self.apply_reload(reload, ctx);
        }
        let switches: Vec<ProfileSwitch> = ctx.events::<ProfileSwitch>().cloned().collect();
        for switch in switches {
            self.set_profile(&switch.label, switch.on);
        }
        if self.wrecked {
            self.sync_surface_colliders(ctx);
            self.instrumentation.update(ctx.rigidbody(), dt);
            return;
        }
        // Before the wings push: a surface torn off on this hit must not
        // still be lifting, and its collider has to be gone before the
        // world steps again.
        self.ground_strikes(ctx);
        self.sync_surface_colliders(ctx);
        // Shot down: whatever the pilot does, nothing answers - engine off,
        // no fly-by-wire, the surfaces jammed.
        let controls = match self.shot_down {
            Some(roll) => PlaneControls {
                throttle: 0.0,
                elevator: SHOT_DOWN_PITCH,
                aileron: roll,
                rudder: 0.0,
                fly_by_wire_pitch_autotrim: false,
                ..PlaneControls::new()
            },
            None => controls,
        };

        self.airframe.landing_gear.tick(dt);
        self.airframe.speed_brake.tick(dt);
        self.apply_body_forces(ctx, &controls, dt, self.shot_down.is_none() && !self.engine_out);

        // Nose-wheel steering rides on the rudder pedals, like the real jet.
        let wheel_inputs = WheelInputs {
            steering: controls.rudder,
            brake: if controls.parking_brake { 1.0 } else { controls.wheel_brake },
        };
        self.wheel_manager.update(self.airframe.landing_gear.deploy, &wheel_inputs, ctx.body, ctx.colliders, ctx.bodies, ctx.query);
        self.airframe.landing_gear.any_grounded = self.wheel_manager.renderizable_wheels.values().any(|wheel| wheel.grounded);

        self.wing_manager.update(&controls, ctx.rigidbody_mut(), dt);
        self.apply_kinematic_roll(ctx, &controls);

        self.instrumentation.update(ctx.rigidbody(), dt);
        // After instrumentation, so it reads the real rates, not the
        // pre-scaled ones Rapier's damping will bring back down this step.
        self.cancel_angular_damping(ctx, dt);
    }

    fn publish(&self) -> Option<PhysicsPayload> {
        let wings = &self.wing_manager.wings;
        let elevator_control_input = wings.iter()
            .find(|wing| wing.label == "Right elevator wing")
            .map(|wing| wing.control_input)
            .unwrap_or(0.0);

        Some(Box::new(AircraftState {
            flight_data: self.instrumentation.flight_data.clone(),
            gear_deploy: self.airframe.landing_gear.deploy,
            speed_brake: self.airframe.speed_brake.deploy,
            wheels: self.wheel_manager.renderizable_wheels.clone(),
            elevator_control_input,
            afterburner: self.flight_system.afterburner_activation(),
            wing_lift_forces: wings.iter().map(|wing| (wing.label.clone(), wing.last_lift_force)).collect(),
            g_command: self.wing_manager.pitch_flcs.last_g_cmd,
            profiles: wings.iter().map(|wing| (wing.label.clone(), wing.hp)).collect(),
            pilot_hit: self.pilot_hit,
            fuselage_struck: self.fuselage_struck,
            blasts_handled: self.blasts_handled,
        }))
    }

    /// Each wing's lift as an arrow from its pressure center, each wheel's
    /// suspension ray from its origin to where it touched down (or its full
    /// length, airborne), the tyre friction each grounded wheel applies, and
    /// where the steerable wheel points on the ground.
    fn debug_draw(&self, body: &RigidBody, draw: &mut DebugDraw) {
        let to_world = |local: Vector3<f32>| body.rotation() * local + body.translation();

        for wing in &self.wing_manager.wings {
            // Gone: no outline, no lift arrow. The collider itself is off,
            // so the F5 collider pass doesn't draw it either.
            if wing.hp <= 0.0 {
                continue;
            }
            draw.ray(to_world(wing.pressure_center), wing.last_lift_force * LIFT_DRAW_SCALE, LIFT_COLOR);
            draw.cross(to_world(wing.pressure_center), MARKER_SIZE, PRESSURE_CENTER_COLOR);

            // The outline: with a chord, the same geometry the surfaces are
            // placed on (pressure center = mid-span, quarter chord - see
            // Wing::surface_center); without one (stabilators, fin), a
            // square of the wing's area around its pressure center.
            // A shaped wing: its real planform, surfaces on it.
            if let Some(shape) = &wing.shape {
                let corners = shape.corners(&wing.normal).map(|corner| to_world(corner));
                draw.outline(&corners, WING_OUTLINE_COLOR);
                for surface in &wing.surfaces {
                    let hinge = 1.0 - surface.chord_ratio;
                    draw.outline(&[
                        to_world(shape.point(&wing.normal, surface.span_start, hinge)),
                        to_world(shape.point(&wing.normal, surface.span_end, hinge)),
                        to_world(shape.point(&wing.normal, surface.span_end, 1.0)),
                        to_world(shape.point(&wing.normal, surface.span_start, 1.0)),
                    ], SURFACE_COLOR);
                }
                continue;
            }

            let span_axis = wing.tip_direction();
            let forward = Vector3::z();
            let (span, leading, trailing) = if wing.chord > 0.0 {
                (wing.wing_area / wing.chord, 0.25 * wing.chord, 0.75 * wing.chord)
            } else {
                let side = wing.wing_area.sqrt();
                (side, side * 0.5, side * 0.5)
            };
            let at = |span_fraction: f32, chord_offset: f32| to_world(wing.pressure_center + span_axis * ((span_fraction - 0.5) * span) + forward * chord_offset);
            draw.outline(&[at(0.0, leading), at(1.0, leading), at(1.0, -trailing), at(0.0, -trailing)], WING_OUTLINE_COLOR);

            if wing.chord > 0.0 {
                for surface in &wing.surfaces {
                    let hinge = -trailing + surface.chord_ratio * wing.chord;
                    draw.outline(&[
                        at(surface.span_start, hinge), at(surface.span_end, hinge),
                        at(surface.span_end, -trailing), at(surface.span_start, -trailing),
                    ], SURFACE_COLOR);
                }
            }
        }

        draw.cross(body.center_of_mass().coords, MARKER_SIZE, CENTER_OF_MASS_COLOR);
        draw.cross(to_world(self.instrumentation.pilot_position()), MARKER_SIZE, PILOT_SEAT_COLOR);
        for position in &self.effect_positions {
            draw.cross(to_world(*position), MARKER_SIZE, EMITTER_COLOR);
            draw.ray(to_world(*position), body.rotation() * Vector3::y() * EMITTER_TICK, EMITTER_COLOR);
        }

        for wheel in &self.wheel_manager.wheels {
            let Some(contact) = self.wheel_manager.renderizable_wheels.get(&wheel.mesh_name) else { continue };
            let color = if contact.grounded { SUSPENSION_GROUNDED_COLOR } else { SUSPENSION_AIRBORNE_COLOR };
            draw.line(to_world(wheel.offset), to_world(contact.local_position), color);

            if contact.grounded {
                draw.ray(to_world(contact.local_position), wheel.last_tyre_force * TYRE_FORCE_DRAW_SCALE, TYRE_FORCE_COLOR);
            }

            if wheel.steerable && contact.grounded {
                if let Some(heading) = wheel.heading(body.rotation()) {
                    draw.ray(to_world(contact.local_position), heading * STEERING_DRAW_LENGTH, STEERING_COLOR);
                }
            }
        }

        // The speed-brake boards, hinged on the aft fuselage and swinging
        // out to each side as they open. Stowed, they lie along the tail.
        let open = self.airframe.speed_brake.deploy;
        let swing = open * 55.0_f32.to_radians();
        for side in [-1.0_f32, 1.0] {
            let hinge = Vector3::new(side * 0.42, 0.2, -2.5);
            let outward = Vector3::new(side * swing.sin(), 0.0, -swing.cos());
            let tip = hinge + outward * 0.9;
            let up = Vector3::y() * 0.22;
            draw.outline(&[
                to_world(hinge - up * 0.5),
                to_world(hinge + up * 0.5),
                to_world(tip + up * 0.5),
                to_world(tip - up * 0.5),
            ], SPEED_BRAKE_COLOR);
        }
    }
}

/// Closest point on a box to `point`. A point inside the box lands on the
/// nearest face, so the shove has a direction.
fn closest_on_cuboid(point: Vector3<f32>, center: Vector3<f32>, half: Vector3<f32>) -> Vector3<f32> {
    let d = point - center;
    let inside = d.x.abs() <= half.x && d.y.abs() <= half.y && d.z.abs() <= half.z;
    if !inside {
        return center + Vector3::new(d.x.clamp(-half.x, half.x), d.y.clamp(-half.y, half.y), d.z.clamp(-half.z, half.z));
    }
    let (px, py, pz) = (half.x - d.x.abs(), half.y - d.y.abs(), half.z - d.z.abs());
    let sign = |value: f32| if value == 0.0 { 1.0 } else { value.signum() };
    let mut out = d;
    if px <= py && px <= pz {
        out.x = half.x * sign(d.x);
    } else if py <= pz {
        out.y = half.y * sign(d.y);
    } else {
        out.z = half.z * sign(d.z);
    }
    center + out
}
