use std::collections::HashMap;
use std::sync::mpsc::Sender;

use crate::game::play::plane::physics::wheels::wheel::{Wheel, WheelData};
use crate::game::play::plane::physics::wheels::wheel_manager::WheelManager;
use crate::game::play::plane::physics::wings::wing_manager::WingManager;
use crate::game::play::plane::plane::{PlaneControls};
use crate::engine::physics::physics::DebugPhysicsMessageType;
use crate::engine::physics::physics_handler::{ColliderDebugData, MetadataType, PhysicsData, PhysicsTick, SuspensionDebugData, WingDebugData};
use rapier3d::prelude::{ColliderSet, QueryPipeline, RigidBodySet};
use crate::game::play::plane::flight_system::FlightSystem;

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

        let suspension_debug_data = self.wheel_manager.update(physics_data, collider_set, rigidbody_set, query_pipeline);
        self.wing_manager.update(plane_controls, rigidbody_set.get_mut(physics_data.rigidbody_handle).unwrap());


        // Send wing and suspension debug data via metadata for main-thread rendering
        if self.debug_rendering_enabled {
            let wing_debug: Vec<WingDebugData> = self.wing_manager.wings.iter().map(|w| WingDebugData {
                pressure_center: w.pressure_center,
                last_lift_force: w.last_lift_force,
            }).collect();
            physics_data.metadata.insert("wings".to_string(), MetadataType::Wings(wing_debug));
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