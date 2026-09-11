use std::collections::HashMap;
use std::sync::mpsc::Sender;

use crate::game::scenes::play::plane::physics::wheels::wheel::{Wheel, WheelData};
use crate::game::scenes::play::plane::physics::wheels::wheel_manager::WheelManager;
use crate::game::scenes::play::plane::physics::wings::wing_manager::WingManager;
use crate::game::scenes::play::plane::physics::rolling_rate::{commanded_roll_rate_deg_s, RollRateParams};
use crate::game::scenes::play::plane::controls::PlaneControls;
use crate::engine::physics::physics::DebugPhysicsMessageType;
use crate::engine::physics::physics_handler::{ColliderDebugData, MetadataType, PhysicsData, PhysicsTick, SuspensionDebugData, WingDebugData};
use rapier3d::prelude::{ColliderSet, QueryPipeline, RigidBodySet};
use crate::game::scenes::play::plane::flight_system::FlightSystem;

pub struct PlanePhysicsLogic {
    pub wheel_manager: WheelManager,
    pub wing_manager: WingManager,
    pub renderizable_wheels: HashMap<String, WheelData>,
    pub renderizable_lines: Vec<DebugPhysicsMessageType>,
    pub flight_system: FlightSystem,
    pub debug_rendering_enabled: bool,
}

impl PlanePhysicsLogic {
    pub fn new() -> Self {
        let wheel_manager = WheelManager::new();
        let wing_manager = WingManager::new();

        Self {
            wheel_manager,
            wing_manager,
            renderizable_wheels: HashMap::new(),
            renderizable_lines: Vec::new(),
            flight_system: FlightSystem::new(),
            debug_rendering_enabled: false,
        }
    }
    
    /// Toggle debug rendering on/off
    pub fn toggle_debug_rendering(&mut self) {
        self.debug_rendering_enabled = !self.debug_rendering_enabled;
        println!("Debug rendering: {}", if self.debug_rendering_enabled { "ENABLED" } else { "DISABLED" });
    }

    /// Configure roll damping for different aircraft types
    pub fn update(&mut self, plane_controls: &PlaneControls, collider_set: &ColliderSet, rigidbody_set: &mut RigidBodySet, query_pipeline: &QueryPipeline, physics_data: &mut PhysicsData, debug_physics_tx: &Sender<Vec<DebugPhysicsMessageType>>, delta_time: f32) {
        self.renderizable_lines.clear();
        physics_data.metadata.clear();

        // Send collider shapes as metadata so the main thread can render them in sync with the model
        if self.debug_rendering_enabled {
            let mut collider_debug: Vec<ColliderDebugData> = Vec::new();
            for collider_handle in &physics_data.collider_handles {
                if let Some(collider) = collider_set.get(*collider_handle) {
                    if let Some(cuboid) = collider.shape().as_cuboid() {
                        let local_pos = collider.position_wrt_parent()
                            .map(|p| p.translation.vector)
                            .unwrap_or_default();
                        collider_debug.push(ColliderDebugData {
                            half_extents: cuboid.half_extents,
                            local_offset: local_pos,
                        });
                    }
                }
            }
            physics_data.metadata.insert("colliders".to_string(), MetadataType::Colliders(collider_debug));
        }


        if let Some(rigidbody) = rigidbody_set.get_mut(physics_data.rigidbody_handle) {
            rigidbody.reset_forces(true);
            rigidbody.reset_torques(true);

            // State calculations
            // NOTE: debug_text!() should be called from the main thread (play.rs), not physics thread
            // Use physics_data.metadata to pass debug values to the main thread if needed

            //self.flight_system.calculate_state(rigidbody, delta_time);
            self.flight_system.update_thrust(rigidbody, delta_time, plane_controls.throttle);

            let local_vel = rigidbody.rotation().inverse() * rigidbody.linvel();
            let sideslip_speed = local_vel.x;
            let air_density = 1.225f32;
            let fuselage_side_area = 20.0; // m² - approximate F-16 fuselage side profile
            let fuselage_cd = 1.2;         // bluff body drag coefficient
            let fuselage_side_force_mag = -0.5 * air_density * sideslip_speed * sideslip_speed.abs() * fuselage_side_area * fuselage_cd;
            let fuselage_side_force = rigidbody.rotation() * nalgebra::Vector3::new(fuselage_side_force_mag, 0.0, 0.0);
            rigidbody.add_force(fuselage_side_force, true);
        }

        let suspension_debug_data = self.wheel_manager.update(plane_controls.gear_deploy, physics_data, collider_set, rigidbody_set, query_pipeline);
        // WheelManager::update fills its OWN `renderizable_wheels`; pull it up
        // so the "wheels" metadata below (which Plane::apply_physics_feedback
        // reads to place the wheel meshes at their suspension-raycast points)
        // isn't left permanently empty.
        self.renderizable_wheels = self.wheel_manager.renderizable_wheels.clone();
        self.wing_manager.update(plane_controls, rigidbody_set.get_mut(physics_data.rigidbody_handle).unwrap());

        // TESTING ONLY - see USE_KINEMATIC_ROLL's own doc comment. The
        // force-based aileron simulation above (Wing::physics_force, via
        // wing_manager.update) is left completely untouched/still running -
        // this just overwrites the roll AXIS COMPONENT of the resulting
        // angular velocity afterward, directly following
        // rolling_rate::commanded_roll_rate_deg_s (the same curve the F7
        // debug overlay plots) instead of whatever the force sim produced.
        // Pitch/yaw are untouched, still fully force-driven. Set
        // USE_KINEMATIC_ROLL to false to go back to pure force-based roll -
        // nothing else needs to change to revert.
        const USE_KINEMATIC_ROLL: bool = true;
        // Weight-on-wheels: with a wheel on the ground the landing gear reacts
        // the rolling moment into the runway - the jet physically can't roll
        // no matter the aileron input. Skip the kinematic override entirely
        // then and let the force sim + suspension keep it level, instead of
        // teleporting angvel.z from a curve that still returns a few deg/s at
        // taxi speed (which read as "the plane rolls standing still").
        let on_ground = self.renderizable_wheels.values().any(|w| w.grounded);
        if USE_KINEMATIC_ROLL && !on_ground {
            if let Some(rigidbody) = rigidbody_set.get_mut(physics_data.rigidbody_handle) {
                let rotation = *rigidbody.rotation();
                let true_airspeed_ms = rigidbody.linvel().magnitude();
                let altitude_m = rigidbody.translation().y;
                // Same aoa_y derivation as Plane::apply_physics_feedback -
                // forward is local +Z, up is local +Y (see that fn's own
                // comment on the axis convention). Absolute value since
                // rolling_rate's own aoa_taper only treats positive/nose-up
                // AoA as authority-reducing, same as wing.rs's own use of it.
                let local_vel = rotation.inverse() * rigidbody.linvel();
                let aoa_deg = (-local_vel.y).atan2(local_vel.z).to_degrees().abs();

                let commanded_rate_deg_s = commanded_roll_rate_deg_s(plane_controls.aileron, true_airspeed_ms, altitude_m, aoa_deg, &RollRateParams::default());
                let commanded_rate_rad_s = commanded_rate_deg_s.to_radians();

                // Decompose world angvel into body axes, replace only the
                // roll (local Z) component, recompose back to world space -
                // same body-axis convention as Plane::apply_physics_feedback
                // (roll_rate = body_angvel.z). Direct/instant, no easing
                // (ramp, first-order lag, and lerp were all tried and
                // reverted - see git history) - any easing means the
                // measured rate lags behind the target while spooling,
                // which reads as a mismatch against the chart's curve; this
                // is a kinematic-match test, exact agreement with the curve
                // matters more here than spool-up feel does. Revisit easing
                // once this is confirmed matching, applied in a way that
                // doesn't read as "wrong" against the chart (e.g. only
                // shown/measured after it's had time to settle).
                let local_angvel = rotation.inverse() * rigidbody.angvel();
                let new_local_angvel = nalgebra::Vector3::new(local_angvel.x, local_angvel.y, commanded_rate_rad_s);
                let new_world_angvel = rotation * new_local_angvel;
                rigidbody.set_angvel(new_world_angvel, true);
            }
        }


        // Wing debug data (control_input included) is sent every tick,
        // unconditionally - unlike colliders/suspensions below, this is
        // also how Plane::apply_physics_feedback finds out what the
        // simulated elevator wings' control_input actually is (e.g. a
        // fly-by-wire solve override), not just an F-key-gated visual aid,
        // so it can't be gated behind debug_rendering_enabled. 5 small
        // structs, cheap either way.
        let wing_debug: Vec<WingDebugData> = self.wing_manager.wings.iter().map(|w| WingDebugData {
            label: w.label.clone(),
            pressure_center: w.pressure_center,
            last_lift_force: w.last_lift_force,
            control_input: w.control_input,
        }).collect();
        physics_data.metadata.insert("wings".to_string(), MetadataType::Wings(wing_debug));

        if self.debug_rendering_enabled {
            physics_data.metadata.insert("suspensions".to_string(), MetadataType::Suspensions(suspension_debug_data));
        }

        physics_data.metadata.insert("wheels".to_string(), MetadataType::Wheels(self.renderizable_wheels.clone()));
    }
}

impl PhysicsTick for PlanePhysicsLogic {
    // The "player" key is a plane-specific convention, so it belongs here rather
    // than in the generic physics engine module.
    fn tick(&mut self, controls: &PlaneControls, collider_set: &ColliderSet, rigidbody_set: &mut RigidBodySet, query_pipeline: &QueryPipeline, physics_elements: &mut HashMap<String, Option<PhysicsData>>, debug_physics_tx: &Sender<Vec<DebugPhysicsMessageType>>, delta_time: f32) {
        match physics_elements.get_mut("player") {
            Some(Some(physics_data)) => {
                self.update(controls, collider_set, rigidbody_set, query_pipeline, physics_data, debug_physics_tx, delta_time);
            },
            _ => println!("Player not found"),
        }
    }

    fn toggle_debug_rendering(&mut self) {
        PlanePhysicsLogic::toggle_debug_rendering(self);
    }

    fn debug_lines(&self) -> &[DebugPhysicsMessageType] {
        &self.renderizable_lines
    }
}