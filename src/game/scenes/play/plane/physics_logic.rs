use std::collections::HashMap;
use std::sync::mpsc::Sender;

use crate::game::scenes::play::plane::aircraft_spec::AircraftSpec;
use crate::game::scenes::play::plane::physics::wheels::wheel::WheelData;
use crate::game::scenes::play::plane::physics::wheels::wheel_manager::WheelManager;
use crate::game::scenes::play::plane::physics::wings::wing_manager::WingManager;
use crate::game::scenes::play::plane::physics::rolling_rate::{commanded_roll_rate_deg_s, RollRateParams};
use crate::game::scenes::play::plane::controls::PlaneControls;
use crate::engine::physics::physics::DebugPhysicsMessageType;
use crate::engine::physics::physics_handler::{ColliderDebugData, MetadataType, PhysicsData, PhysicsTick, WingDebugData};
use rapier3d::prelude::{ColliderSet, QueryPipeline, RigidBodySet};
use crate::game::scenes::play::plane::flight_system::FlightSystem;
use rayon::prelude::*;

/// What `AircraftUnit::compute` produces and `AircraftUnit::apply` applies -
/// the boundary between the parallel and serial halves of one physics tick
/// (see `PlanePhysicsLogic::tick`). Only the forces that can be derived from
/// a read-only snapshot of the rigidbody live here; anything that needs to
/// mutate `RigidBodySet` while computing (wheel suspension raycasts, wing
/// force application, the kinematic roll override) still happens inside
/// `apply`, unchanged and still serial.
struct ComputedForces {
    thrust_force: nalgebra::Vector3<f32>,
    fuselage_side_force: nalgebra::Vector3<f32>,
}

/// One aircraft's own aero/thrust/wheel simulation state - everything
/// `PlanePhysicsLogic` used to own directly for the single hardcoded
/// "player" body, now one instance per node that carries its own
/// `AircraftSpec` property (see that type's own doc comment).
struct AircraftUnit {
    wheel_manager: WheelManager,
    wing_manager: WingManager,
    renderizable_wheels: HashMap<String, WheelData>,
    flight_system: FlightSystem,
}

impl AircraftUnit {
    fn new(spec: &AircraftSpec) -> Self {
        Self {
            wheel_manager: WheelManager::new(),
            wing_manager: WingManager::new(spec),
            renderizable_wheels: HashMap::new(),
            flight_system: FlightSystem::new(),
        }
    }

    /// The parallel half of one tick: everything derivable from a read-only
    /// snapshot of this aircraft's own rigidbody, touching nothing but this
    /// `AircraftUnit`'s own state (safe to call from inside a rayon
    /// `par_iter_mut` over every aircraft at once - see
    /// `PlanePhysicsLogic::tick`) and `rigidbody_set` *immutably*. Doesn't
    /// touch `physics_data.metadata` or apply anything to the rigidbody -
    /// that's `apply`'s job, run serially afterward.
    fn compute(&mut self, plane_controls: &PlaneControls, rigidbody_set: &RigidBodySet, physics_data: &PhysicsData) -> ComputedForces {
        let Some(rigidbody) = rigidbody_set.get(physics_data.rigidbody_handle) else {
            return ComputedForces { thrust_force: nalgebra::Vector3::zeros(), fuselage_side_force: nalgebra::Vector3::zeros() };
        };

        let thrust_force = self.flight_system.compute_thrust(rigidbody.translation().y, *rigidbody.rotation(), plane_controls.throttle);

        let local_vel = rigidbody.rotation().inverse() * rigidbody.linvel();
        let sideslip_speed = local_vel.x;
        let air_density = 1.225f32;
        let fuselage_side_area = 20.0; // m² - approximate F-16 fuselage side profile
        let fuselage_cd = 1.2;         // bluff body drag coefficient
        let fuselage_side_force_mag = -0.5 * air_density * sideslip_speed * sideslip_speed.abs() * fuselage_side_area * fuselage_cd;
        let fuselage_side_force = rigidbody.rotation() * nalgebra::Vector3::new(fuselage_side_force_mag, 0.0, 0.0);

        ComputedForces { thrust_force, fuselage_side_force }
    }

    /// The serial half of one tick: applies `forces` (from `compute`, run
    /// beforehand) to the shared `RigidBodySet`, then runs the wheel/wing/
    /// kinematic-roll logic exactly as the old single-phase `update` did -
    /// those still mutate `rigidbody_set` directly mid-computation (wheel
    /// suspension raycasts, force-based aileron application, the roll
    /// override) and haven't been split into their own compute/apply halves,
    /// so they stay untouched and sequential here rather than being
    /// rewritten blind.
    fn apply(&mut self, plane_controls: &PlaneControls, collider_set: &ColliderSet, rigidbody_set: &mut RigidBodySet, query_pipeline: &QueryPipeline, physics_data: &mut PhysicsData, _debug_physics_tx: &Sender<Vec<DebugPhysicsMessageType>>, _delta_time: f32, debug_rendering_enabled: bool, forces: ComputedForces) {
        physics_data.metadata.clear();

        // Send collider shapes as metadata so the main thread can render them in sync with the model
        if debug_rendering_enabled {
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
            rigidbody.add_force(forces.thrust_force, true);
            rigidbody.add_force(forces.fuselage_side_force, true);
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

        if debug_rendering_enabled {
            physics_data.metadata.insert("suspensions".to_string(), MetadataType::Suspensions(suspension_debug_data));
        }

        physics_data.metadata.insert("wheels".to_string(), MetadataType::Wheels(self.renderizable_wheels.clone()));
    }
}

/// Drives every aircraft's own physics, keyed by node id - one `AircraftUnit`
/// built from that node's own `AircraftSpec` property (see that type's own
/// doc comment), instead of a single hardcoded "player" body. `controls`
/// (see `PhysicsTick::tick`'s own signature) is still a single value shared
/// by every entry - routing distinct controls per aircraft is a separate,
/// already-identified follow-up, not done here.
pub struct PlanePhysicsLogic {
    aircraft: HashMap<String, AircraftUnit>,
    // Shared debug viz across every aircraft, not per-instance - see
    // `tick`'s own comment on why it's cleared once per tick rather than
    // once per aircraft.
    renderizable_lines: Vec<DebugPhysicsMessageType>,
    // Shared F2 toggle across every aircraft, not per-instance.
    debug_rendering_enabled: bool,
}

impl PlanePhysicsLogic {
    pub fn new(aircraft_specs: HashMap<String, AircraftSpec>) -> Self {
        let aircraft = aircraft_specs.iter().map(|(id, spec)| (id.clone(), AircraftUnit::new(spec))).collect();

        Self {
            aircraft,
            renderizable_lines: Vec::new(),
            debug_rendering_enabled: false,
        }
    }

    /// Toggle debug rendering on/off
    pub fn toggle_debug_rendering(&mut self) {
        self.debug_rendering_enabled = !self.debug_rendering_enabled;
        println!("Debug rendering: {}", if self.debug_rendering_enabled { "ENABLED" } else { "DISABLED" });
    }
}

impl PhysicsTick for PlanePhysicsLogic {
    fn tick(&mut self, controls: &PlaneControls, collider_set: &ColliderSet, rigidbody_set: &mut RigidBodySet, query_pipeline: &QueryPipeline, physics_elements: &mut HashMap<String, Option<PhysicsData>>, debug_physics_tx: &Sender<Vec<DebugPhysicsMessageType>>, delta_time: f32) {
        // Cleared once per tick, not inside each aircraft's own update - with
        // more than one aircraft, clearing per-aircraft would wipe an
        // earlier aircraft's debug lines this same tick.
        self.renderizable_lines.clear();
        let debug_rendering_enabled = self.debug_rendering_enabled;

        // Phase 1 (parallel): each aircraft computes its own thrust/fuselage-
        // drag forces from a read-only snapshot of the shared rigidbody set -
        // no aircraft reads or writes another's data, so rayon can farm these
        // out across threads instead of walking the HashMap one entry at a
        // time. `rigidbody_set`/`physics_elements` are only reborrowed
        // immutably for this pass; the mutable borrows they came in as are
        // free again by the time phase 2 needs them (NLL ends these once
        // `computed` is done collecting).
        let rigidbody_set_ref: &RigidBodySet = rigidbody_set;
        let physics_elements_ref: &HashMap<String, Option<PhysicsData>> = physics_elements;
        let computed: Vec<(String, Option<ComputedForces>)> = self.aircraft
            .par_iter_mut()
            .map(|(id, aircraft)| {
                let forces = match physics_elements_ref.get(id) {
                    Some(Some(physics_data)) => Some(aircraft.compute(controls, rigidbody_set_ref, physics_data)),
                    _ => None,
                };
                (id.clone(), forces)
            })
            .collect();

        // Phase 2 (serial): apply each aircraft's computed forces, then run
        // the still-sequential wheel/wing/kinematic-roll logic exactly as
        // before (see `AircraftUnit::apply`'s own doc comment on why those
        // aren't parallelized yet).
        for (id, forces) in computed {
            let Some(forces) = forces else {
                println!("Aircraft '{id}' not found in physics_elements");
                continue;
            };
            if let (Some(aircraft), Some(Some(physics_data))) = (self.aircraft.get_mut(&id), physics_elements.get_mut(&id)) {
                aircraft.apply(controls, collider_set, rigidbody_set, query_pipeline, physics_data, debug_physics_tx, delta_time, debug_rendering_enabled, forces);
            }
        }
    }

    fn toggle_debug_rendering(&mut self) {
        PlanePhysicsLogic::toggle_debug_rendering(self);
    }

    fn debug_lines(&self) -> &[DebugPhysicsMessageType] {
        &self.renderizable_lines
    }
}
