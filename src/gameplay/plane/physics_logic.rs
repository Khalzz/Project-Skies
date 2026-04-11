use std::collections::HashMap;
use std::sync::mpsc::Sender;

use crate::gameplay::plane::plane::{PlaneControls};
use crate::gameplay::wing::{Wing};
use crate::gameplay::airfoil::AirFoil;
use crate::gameplay::wheel::{Wheel, WheelData};
use crate::physics::physics::DebugPhysicsMessageType;
use crate::physics::physics_handler::{ColliderDebugData, MetadataType, PhysicsData, SuspensionDebugData, WingDebugData};
use rapier3d::prelude::{ColliderSet, QueryPipeline, RigidBodySet};
use nalgebra::vector;
use crate::gameplay::plane::flight_system::FlightSystem;

pub struct PlanePhysicsLogic {
    pub wheels: Vec<Wheel>,
    pub wings: Vec<Wing>,
    pub renderizable_wheels: HashMap<String, WheelData>,
    pub renderizable_lines: Vec<DebugPhysicsMessageType>,
    pub flight_system: FlightSystem,
    pub debug_rendering_enabled: bool,
}

impl PlanePhysicsLogic {
    pub fn new() -> Self {
        let wheels = vec![
            Wheel::new("wheel-f".to_string(), vector![0.0, 0.0, 0.7], 0.4, 300000.0, 50000.0),
            Wheel::new("wheel-lb".to_string(), vector![-0.1, 0.0, 0.0], 0.3, 100000.0, 20000.0),
            Wheel::new("wheel-rb".to_string(), vector![0.1, 0.0, 0.0], 0.3, 100000.0, 20000.0)
        ];

        // load airfoil:
        let naca_2412 = AirFoil::new("assets/aero_data/f16.ron".to_owned());
        let naca_0012 = AirFoil::new("assets/aero_data/f16-elevators.ron".to_owned());

        // i have to also add left and right ailerons
        let wings = vec![
            Wing::new("Left wing".to_string(), vector![0.4, 0.0, 0.1], 16.5, 0.0, naca_2412.clone(), vector![1.0,0.0, 0.0], true, false, 4.0, 500_000.0), // left wing (+4° incidence, includes LEX area)
            Wing::new("Right wing".to_string(), vector![-0.4, 0.0, 0.1], 16.5, 0.0, naca_2412.clone(), vector![1.0, 0.0, 0.0], true, false, 4.0, 500_000.0), // right wing (+4° incidence, includes LEX area)
            Wing::new("Right elevator wing".to_string(), vector![0.3, 0.0, -0.5], 2.70, 0.0, naca_0012.clone(), vector![1.0, 0.0, 0.0], false, false, 0.0, 120_000.0), // right elevator wing
            Wing::new("Left elevator wing".to_string(), vector![-0.3, 0.0, -0.5], 2.70, 0.0, naca_0012.clone(), vector![1.0, 0.0, 0.0], false, false, 0.0, 120_000.0), // left elevator wing
            Wing::new("Rudder wing".to_string(), vector![0.0, 0.3, -0.8], 1.70, 0.0, naca_0012.clone(), vector![0.0, 1.0, 0.0], false, true, 0.0, 200_000.0) // rudder wing
        ];

        Self {
            wheels,
            wings,
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

            for wing in &mut self.wings {
                wing.control_input = match wing.label.as_str() {
                    "Left wing"           => (-plane_controls.aileron + plane_controls.trim_roll).clamp(-1.0, 1.0),
                    "Right wing"          => (plane_controls.aileron + plane_controls.trim_roll).clamp(-1.0, 1.0),
                    "Left elevator wing"  => (plane_controls.elevator + plane_controls.trim_pitch).clamp(-1.0, 1.0),
                    "Right elevator wing" => (plane_controls.elevator + plane_controls.trim_pitch).clamp(-1.0, 1.0),
                    "Rudder wing"         => (plane_controls.rudder + plane_controls.trim_yaw).clamp(-1.0, 1.0),
                    _ => 0.0,
                };

                wing.physics_force(rigidbody);
            }

            // Fuselage side force: the fuselage acts as a flat plate during sideslip,
            // generating a lateral force at the CoM that redirects the velocity vector
            // to align with the aircraft heading. Without this, the rudder only rotates
            // the nose but the plane keeps sliding in the original direction.
            let local_vel = rigidbody.rotation().inverse() * rigidbody.linvel();
            let sideslip_speed = local_vel.x;
            let air_density = 1.225f32;
            let fuselage_side_area = 20.0; // m² - approximate F-16 fuselage side profile
            let fuselage_cd = 1.2;         // bluff body drag coefficient
            let fuselage_side_force_mag = -0.5 * air_density * sideslip_speed * sideslip_speed.abs() * fuselage_side_area * fuselage_cd;
            let fuselage_side_force = rigidbody.rotation() * nalgebra::Vector3::new(fuselage_side_force_mag, 0.0, 0.0);
            rigidbody.add_force(fuselage_side_force, true);
        }

        self.renderizable_wheels.clear();
        let mut suspension_debug_data: Vec<SuspensionDebugData> = Vec::new();
        
        for (index, wheel) in self.wheels.iter_mut().enumerate() {
            if let Some((suspension_force, suspension_origin, wheel_position)) = wheel.update_wheel(&physics_data, &collider_set, rigidbody_set, &query_pipeline) {
                if let Some(rigidbody) = rigidbody_set.get_mut(physics_data.rigidbody_handle) {
                    rigidbody.add_force_at_point(suspension_force, suspension_origin.into(), true);
                }
                if let Some(rigidbody) = rigidbody_set.get(physics_data.rigidbody_handle) {
                    let rb_pos = rigidbody.translation();
                    let rb_rot = rigidbody.rotation();
                    let local_position = rb_rot.inverse() * (wheel_position - rb_pos);
                    let local_origin = rb_rot.inverse() * (suspension_origin - rb_pos);
                    self.renderizable_wheels.insert(wheel.mesh_name.clone(), WheelData { local_position });
                    suspension_debug_data.push(SuspensionDebugData {
                        local_origin,
                        local_wheel: local_position,
                    });
                }
            }
        }

        // Send wing and suspension debug data via metadata for main-thread rendering
        if self.debug_rendering_enabled {
            let wing_debug: Vec<WingDebugData> = self.wings.iter().map(|w| WingDebugData {
                pressure_center: w.pressure_center,
                last_lift_force: w.last_lift_force,
            }).collect();
            physics_data.metadata.insert("wings".to_string(), MetadataType::Wings(wing_debug));
            physics_data.metadata.insert("suspensions".to_string(), MetadataType::Suspensions(suspension_debug_data));
        }

        physics_data.metadata.insert("wheels".to_string(), MetadataType::Wheels(self.renderizable_wheels.clone()));        
    }
}