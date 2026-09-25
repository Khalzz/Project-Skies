use nalgebra::Vector3;
use rapier3d::prelude::RigidBody;

use crate::engine::physics::physics_behavior::{DebugDraw, PhysicsBehavior, PhysicsCtx, PhysicsPayload};

use super::aircraft_spec::AircraftSpec;
use super::airframe::Airframe;
use super::controls::PlaneControls;
use super::flight_system::FlightSystem;
use super::instrumentation::Instrumentation;
use super::messages::{AircraftEvent, AircraftState};
use super::physics::rolling_rate::{commanded_roll_rate_deg_s, RollRateParams};
use super::physics::wheels::wheel_manager::{WheelInputs, WheelManager};
use super::physics::wings::wing_manager::WingManager;

/// TESTING ONLY - the force-based aileron simulation (Wing::physics_force,
/// via wing_manager.update) still runs untouched; this just overwrites the
/// roll AXIS COMPONENT of the resulting angular velocity afterward, directly
/// following rolling_rate::commanded_roll_rate_deg_s (the same curve the F7
/// debug overlay plots). Pitch/yaw stay fully force-driven. Set to false to
/// go back to pure force-based roll - nothing else needs to change.
const USE_KINEMATIC_ROLL: bool = true;

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
// Nose-wheel heading, drawn from its contact point while on the ground.
const STEERING_DRAW_LENGTH: f32 = 4.0;
const STEERING_COLOR: [f32; 3] = [0.2, 0.9, 1.0];

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
}

impl AircraftPhysics {
    pub fn new(spec: &AircraftSpec) -> Self {
        Self {
            wing_manager: WingManager::new(spec),
            wheel_manager: WheelManager::new(),
            flight_system: FlightSystem::new(),
            airframe: Airframe::new(),
            instrumentation: Instrumentation::new(),
        }
    }

    fn handle_event(&mut self, event: AircraftEvent) {
        let landing_gear = &mut self.airframe.landing_gear;
        match event {
            AircraftEvent::ToggleGear => landing_gear.request_toggle(),
            AircraftEvent::GearUp => landing_gear.set_commanded_down(false),
            AircraftEvent::GearDown => landing_gear.set_commanded_down(true),
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

    /// See `USE_KINEMATIC_ROLL`. Skipped with weight on wheels: the landing
    /// gear reacts the rolling moment into the runway, so the jet can't roll
    /// there no matter the aileron input.
    fn apply_kinematic_roll(&self, ctx: &mut PhysicsCtx, controls: &PlaneControls) {
        let on_ground = self.wheel_manager.renderizable_wheels.values().any(|wheel| wheel.grounded);
        if !USE_KINEMATIC_ROLL || on_ground {
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
            self.handle_event(event);
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
