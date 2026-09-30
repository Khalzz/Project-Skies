use std::sync::atomic::{AtomicBool, Ordering};

use nalgebra::Vector3;
use rapier3d::prelude::RigidBody;

use crate::engine::physics::physics_behavior::{DebugDraw, PhysicsBehavior, PhysicsCtx, PhysicsPayload};
use crate::game::scenes::play::plane::aero_spec::AeroSpec;

use super::airframe::Airframe;
use super::controls::PlaneControls;
use super::flight_system::FlightSystem;
use super::instrumentation::Instrumentation;
use super::messages::{AircraftEvent, AircraftReload, AircraftState};
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

// F2 overlay: how long a wing's lift arrow is per newton (m/N) - ~9 m for
// one main wing's share of level-flight lift, ~100 m at its force clamp.
const LIFT_DRAW_SCALE: f32 = 1.0 / 5000.0;
const LIFT_COLOR: [f32; 3] = [0.3, 1.0, 0.3];
const SUSPENSION_GROUNDED_COLOR: [f32; 3] = [1.0, 0.8, 0.2];
const SUSPENSION_AIRBORNE_COLOR: [f32; 3] = [0.5, 0.5, 0.5];
// Each wheel's tyre friction, drawn from its contact point - same scale as
// the lift arrows, so the two read against each other.
const TYRE_FORCE_DRAW_SCALE: f32 = LIFT_DRAW_SCALE;
const TYRE_FORCE_COLOR: [f32; 3] = [1.0, 0.3, 1.0];
// Wing geometry (F2): each wing's pressure center, its outline, and its
// hinged surfaces - from the live wings, so a data.ron reload shows at once.
const PRESSURE_CENTER_COLOR: [f32; 3] = [1.0, 1.0, 1.0];
const WING_OUTLINE_COLOR: [f32; 3] = [0.6, 0.6, 0.6];
const SURFACE_COLOR: [f32; 3] = [1.0, 0.9, 0.2];
const MARKER_SIZE: f32 = 0.6;
// The rigidbody's center of mass, and the pilot's seat (where seat G is
// measured).
const CENTER_OF_MASS_COLOR: [f32; 3] = [1.0, 0.2, 1.0];
const PILOT_SEAT_COLOR: [f32; 3] = [0.2, 1.0, 0.4];
// Nose-wheel heading, drawn from its contact point while on the ground.
const STEERING_DRAW_LENGTH: f32 = 4.0;
const STEERING_COLOR: [f32; 3] = [0.2, 0.9, 1.0];
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
    /// Where the plane's particle emitters sit (its data.ron `effects`,
    /// jet's frame) - only drawn, for the F1 overlay.
    effect_positions: Vec<Vector3<f32>>,
}

impl AircraftPhysics {
    pub fn new(spec: &AeroSpec, engine: &super::engine::EngineSpec, gear: &super::gear_spec::GearSpec, effects: &[super::effects::EffectSpec]) -> Self {
        Self {
            wing_manager: WingManager::new(spec),
            wheel_manager: WheelManager::new(gear),
            flight_system: FlightSystem::new(engine.clone()),
            airframe: Airframe::new(),
            instrumentation: Instrumentation::new(spec.pilot_position),
            wrecked: false,
            effect_positions: effects.iter().map(|effect| effect.position).collect(),
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
        let inertia = compute_principal_inertia(reload.mass, reload.center_of_mass, &reload.colliders);
        let mass_properties = rapier3d::prelude::MassProperties::new(reload.center_of_mass.into(), reload.mass, inertia);
        ctx.rigidbody_mut().set_additional_mass_properties(mass_properties, true);
    }

    fn handle_event(&mut self, event: AircraftEvent, ctx: &mut PhysicsCtx) {
        let landing_gear = &mut self.airframe.landing_gear;
        match event {
            AircraftEvent::ToggleGear => landing_gear.request_toggle(),
            AircraftEvent::GearUp => landing_gear.set_commanded_down(false),
            AircraftEvent::GearDown => landing_gear.set_commanded_down(true),
            AircraftEvent::Wreck => {
                self.wrecked = true;
                // Rapier keeps user forces until reset - drop the last
                // step's thrust and lift, since nothing will replace them.
                let rigidbody = ctx.rigidbody_mut();
                rigidbody.reset_forces(true);
                rigidbody.reset_torques(true);
            }
        }
    }

    /// Thrust plus fuselage side-drag, applied fresh this step - forces are
    /// reset first, since rapier keeps user forces until told otherwise.
    fn apply_body_forces(&mut self, ctx: &mut PhysicsCtx, controls: &PlaneControls, dt: f32) {
        let rigidbody = ctx.rigidbody_mut();
        rigidbody.reset_forces(true);
        rigidbody.reset_torques(true);

        let rotation = *rigidbody.rotation();
        let thrust_force = self.flight_system.compute_thrust(rigidbody.translation().y, rotation, controls.throttle, dt);

        let sideslip_speed = (rotation.inverse() * rigidbody.linvel()).x;
        let air_density = 1.225f32;
        let fuselage_side_area = 20.0; // m² - approximate F-16 fuselage side profile
        let fuselage_cd = 1.2;         // bluff body drag coefficient
        let fuselage_side_force_mag = -0.5 * air_density * sideslip_speed * sideslip_speed.abs() * fuselage_side_area * fuselage_cd;
        let fuselage_side_force = rotation * Vector3::new(fuselage_side_force_mag, 0.0, 0.0);

        rigidbody.add_force(thrust_force, true);
        rigidbody.add_force(fuselage_side_force, true);
    }

    /// Undoes the rigidbody's blanket `angular_damping` (see
    /// `load_physics_from_definitions`) on the YAW axis only, while any wheel
    /// is on the ground. That damping is an in-flight stability aid; on the
    /// ground the tyres already provide real yaw damping, and on top of them
    /// it resisted yaw ~6x harder than all the tyre grip combined - which
    /// capped nose-wheel steering at a few deg/s. Pitch and roll keep it.
    ///
    /// Rapier applies damping by scaling angvel by 1 / (1 + dt * damping)
    /// each step, so pre-scaling the yaw component by (1 + dt * damping)
    /// cancels it.
    fn cancel_ground_yaw_damping(&self, ctx: &mut PhysicsCtx, dt: f32) {
        let on_ground = self.wheel_manager.renderizable_wheels.values().any(|wheel| wheel.grounded);
        if !on_ground {
            return;
        }

        let rigidbody = ctx.rigidbody_mut();
        let rotation = *rigidbody.rotation();
        let mut local_angvel = rotation.inverse() * rigidbody.angvel();
        local_angvel.y *= 1.0 + dt * rigidbody.angular_damping();
        rigidbody.set_angvel(rotation * local_angvel, true);
    }

    /// Undoes the rigidbody's blanket `angular_damping` on the ROLL axis,
    /// always - same pre-scaling trick as `cancel_ground_yaw_damping`. That
    /// damping is a generic stability aid; on roll it stacked on top of the
    /// wings' own aerodynamic roll damping and held the physics roll to a
    /// fraction of the roll-rate curve. Pitch/yaw keep it.
    fn cancel_roll_damping(&self, ctx: &mut PhysicsCtx, dt: f32) {
        let rigidbody = ctx.rigidbody_mut();
        let rotation = *rigidbody.rotation();
        let mut local_angvel = rotation.inverse() * rigidbody.angvel();
        local_angvel.z *= 1.0 + dt * rigidbody.angular_damping();
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
}

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
        if self.wrecked {
            self.instrumentation.update(ctx.rigidbody(), dt);
            return;
        }

        self.airframe.landing_gear.tick(dt);
        self.apply_body_forces(ctx, &controls, dt);

        // Nose-wheel steering rides on the rudder pedals, like the real jet.
        let wheel_inputs = WheelInputs {
            steering: controls.rudder,
            brake: if controls.parking_brake { 1.0 } else { controls.wheel_brake },
        };
        self.wheel_manager.update(self.airframe.landing_gear.deploy, &wheel_inputs, ctx.body, ctx.colliders, ctx.bodies, ctx.query);
        self.airframe.landing_gear.any_grounded = self.wheel_manager.renderizable_wheels.values().any(|wheel| wheel.grounded);
        self.cancel_ground_yaw_damping(ctx, dt);

        self.wing_manager.update(&controls, ctx.rigidbody_mut(), dt);
        self.apply_kinematic_roll(ctx, &controls);

        self.instrumentation.update(ctx.rigidbody(), dt);
        // After instrumentation, so it reads the real roll rate, not the
        // pre-scaled one Rapier's damping will bring back down this step.
        self.cancel_roll_damping(ctx, dt);
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
            wheels: self.wheel_manager.renderizable_wheels.clone(),
            elevator_control_input,
            afterburner: self.flight_system.afterburner_activation(),
            wing_lift_forces: wings.iter().map(|wing| (wing.label.clone(), wing.last_lift_force)).collect(),
        }))
    }

    /// Each wing's lift as an arrow from its pressure center, each wheel's
    /// suspension ray from its origin to where it touched down (or its full
    /// length, airborne), the tyre friction each grounded wheel applies, and
    /// where the steerable wheel points on the ground.
    fn debug_draw(&self, body: &RigidBody, draw: &mut DebugDraw) {
        let to_world = |local: Vector3<f32>| body.rotation() * local + body.translation();

        for wing in &self.wing_manager.wings {
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
    }
}
