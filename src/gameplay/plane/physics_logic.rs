use std::collections::HashMap;
use std::sync::mpsc::Sender;

use crate::gameplay::plane::plane::{Plane, PlaneControls};
use crate::gameplay::wing::{Wing};
use crate::gameplay::airfoil::AirFoil;
use crate::gameplay::wheel::{Wheel, WheelData};
use crate::physics::physics::DebugPhysicsMessageType;
use crate::physics::physics_handler::{ColliderDebugData, MetadataType, PhysicsData, WingDebugData};
use crate::primitive::manual_vertex::ManualVertex;
use rapier3d::prelude::{ColliderSet, QueryPipeline, RigidBodySet, RigidBody};
use nalgebra::vector;
use nalgebra::Vector3;
use crate::gameplay::plane::flight_system::FlightSystem;

/// Safely add torque to rigidbody, skipping if NaN/Inf


/// Check if rigidbody state is valid (no NaN/Inf in position or velocity)
fn is_rigidbody_valid(rigidbody: &RigidBody) -> bool {
    let pos = rigidbody.translation();
    let vel = rigidbody.linvel();
    let angvel = rigidbody.angvel();
    
    pos.x.is_finite() && pos.y.is_finite() && pos.z.is_finite() &&
    vel.x.is_finite() && vel.y.is_finite() && vel.z.is_finite() &&
    angvel.x.is_finite() && angvel.y.is_finite() && angvel.z.is_finite()
}

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
            Wheel::new("wheel-f".to_string(), vector![0.0, 0.0, 0.7], 0.6, 15000.0, 1000.0),
            Wheel::new("wheel-lb".to_string(), vector![-0.1, 0.0, 0.0], 0.3, 10000.0, 1000.0),
            Wheel::new("wheel-rb".to_string(), vector![0.1, 0.0, 0.0], 0.3, 10000.0, 1000.0)
        ];

        // load airfoil:
        let naca_2412 = AirFoil::new("assets/aero_data/f16.ron".to_owned());
        let naca_0012 = AirFoil::new("assets/aero_data/f16-elevators.ron".to_owned());

        // i have to also add left and right ailerons
        let wings = vec![
            Wing::new("Left wing".to_string(), vector![0.4, 0.0, 0.1], 2.50, 0.0, naca_2412.clone(), vector![1.0,0.0, 0.0], true, false, -3.0), // left wing (-3° incidence for lift)
            Wing::new("Right wing".to_string(), vector![-0.4, 0.0, 0.1], 2.50, 0.0, naca_2412.clone(), vector![1.0, 0.0, 0.0], true, false, -3.0), // right wing (-3° incidence for lift)
            Wing::new("Right elevator wing".to_string(), vector![0.3, 0.0, -0.5], 2.70, 0.0, naca_0012.clone(), vector![1.0, 0.0, 0.0], false, false, 0.0), // right elevator wing
            Wing::new("Left elevator wing".to_string(), vector![-0.3, 0.0, -0.5], 2.70, 0.0, naca_0012.clone(), vector![1.0, 0.0, 0.0], false, false, 0.0), // left elevator wing
            Wing::new("Rudder wing".to_string(), vector![0.0, 0.3, -0.5], 1.0, 0.0, naca_0012.clone(), vector![0.0, 1.0, 0.0], false, true, 0.0) // rudder wing
        ];

        Self {
            wheels,
            wings,
            renderizable_wheels: HashMap::new(),
            renderizable_lines: Vec::new(),
            flight_system: FlightSystem::new(),
            debug_rendering_enabled: true,
        }
    }
    
    /// Toggle debug rendering on/off
    pub fn toggle_debug_rendering(&mut self) {
        self.debug_rendering_enabled = !self.debug_rendering_enabled;
        println!("Debug rendering: {}", if self.debug_rendering_enabled { "ENABLED" } else { "DISABLED" });
    }

    /// Renders debug lines for all cuboid colliders (wireframe boxes)
    fn render_collider_debug(&mut self, collider_set: &ColliderSet, rigidbody_set: &RigidBodySet, physics_data: &PhysicsData) {
        let rigidbody = match rigidbody_set.get(physics_data.rigidbody_handle) {
            Some(rb) => rb,
            None => return,
        };
        let rb_translation = rigidbody.translation();
        let rb_rotation = rigidbody.rotation();

        // Iterate over all collider handles
        for collider_handle in &physics_data.collider_handles {
            if let Some(collider) = collider_set.get(*collider_handle) {
                // Collider position is LOCAL to the rigidbody
                let col_local = collider.position_wrt_parent().unwrap_or(collider.position());
                let col_offset = col_local.translation.vector;
                let col_rotation = col_local.rotation;
                    
                // Try to get cuboid shape
                if let Some(cuboid) = collider.shape().as_cuboid() {
                    let half_extents = cuboid.half_extents;
                    
                    // Define the 8 corners of the cuboid in local space
                    let corners_local = [
                        Vector3::new(-half_extents.x, -half_extents.y, -half_extents.z),
                        Vector3::new( half_extents.x, -half_extents.y, -half_extents.z),
                        Vector3::new( half_extents.x,  half_extents.y, -half_extents.z),
                        Vector3::new(-half_extents.x,  half_extents.y, -half_extents.z),
                        Vector3::new(-half_extents.x, -half_extents.y,  half_extents.z),
                        Vector3::new( half_extents.x, -half_extents.y,  half_extents.z),
                        Vector3::new( half_extents.x,  half_extents.y,  half_extents.z),
                        Vector3::new(-half_extents.x,  half_extents.y,  half_extents.z),
                    ];
                    
                    // Transform: collider local -> rigidbody local -> world
                    let corners_world: Vec<Vector3<f32>> = corners_local.iter()
                        .map(|c| {
                            let in_rb_space = col_offset + col_rotation * c;
                            rb_translation + rb_rotation * in_rb_space
                        })
                        .collect();
                    
                    // Collider color (cyan/teal for visibility)
                    let color = [0.0, 1.0, 1.0];
                    
                    // Draw the 12 edges of the cuboid
                    // Bottom face edges
                    self.add_debug_line(corners_world[0], corners_world[1], color);
                    self.add_debug_line(corners_world[1], corners_world[2], color);
                    self.add_debug_line(corners_world[2], corners_world[3], color);
                    self.add_debug_line(corners_world[3], corners_world[0], color);
                    
                    // Top face edges
                    self.add_debug_line(corners_world[4], corners_world[5], color);
                    self.add_debug_line(corners_world[5], corners_world[6], color);
                    self.add_debug_line(corners_world[6], corners_world[7], color);
                    self.add_debug_line(corners_world[7], corners_world[4], color);
                    
                    // Vertical edges connecting top and bottom
                    self.add_debug_line(corners_world[0], corners_world[4], color);
                    self.add_debug_line(corners_world[1], corners_world[5], color);
                    self.add_debug_line(corners_world[2], corners_world[6], color);
                    self.add_debug_line(corners_world[3], corners_world[7], color);
                }
            }
        }
    }
    
    /// Helper function to add a debug line between two points
    fn add_debug_line(&mut self, start: Vector3<f32>, end: Vector3<f32>, color: [f32; 3]) {
        self.renderizable_lines.push(DebugPhysicsMessageType::RenderizableLines([
            ManualVertex { position: [start.x, start.y, start.z], color },
            ManualVertex { position: [end.x, end.y, end.z], color }
        ]));
    }

    /// Configure roll damping for different aircraft types
    pub fn update(&mut self, plane_controls: &PlaneControls, collider_set: &ColliderSet, rigidbody_set: &mut RigidBodySet, query_pipeline: &QueryPipeline, physics_data: &mut PhysicsData, debug_physics_tx: &Sender<Vec<DebugPhysicsMessageType>>, delta_time: f32) {
        self.renderizable_lines.clear();

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
                    "Left wing"           => -plane_controls.aileron,   // ailerons roll opposite
                    "Right wing"          => plane_controls.aileron,
                    "Left elevator wing"  => plane_controls.elevator,
                    "Right elevator wing" => plane_controls.elevator,
                    "Rudder wing"         =>  plane_controls.rudder,
                    _ => 0.0,
                };

                wing.physics_force(rigidbody);
            }
        }

        self.renderizable_wheels.clear();
        
        for (index, wheel) in self.wheels.iter_mut().enumerate() {
            if let Some((suspension_force, suspension_origin, wheel_position)) = wheel.update_wheel(&physics_data, &collider_set, rigidbody_set, &query_pipeline) {
                self.renderizable_wheels.insert(wheel.mesh_name.clone(), WheelData { wheel_position, suspension_origin });
                
                if let Some(rigidbody) = rigidbody_set.get_mut(physics_data.rigidbody_handle) {
                    rigidbody.add_force_at_point(suspension_force, suspension_origin.into(), true);
                }
            }
        }

        // Send wing debug data via metadata for main-thread rendering
        if self.debug_rendering_enabled {
            let wing_debug: Vec<WingDebugData> = self.wings.iter().map(|w| WingDebugData {
                pressure_center: w.pressure_center,
                last_lift_force: w.last_lift_force,
            }).collect();
            physics_data.metadata.insert("wings".to_string(), MetadataType::Wings(wing_debug));
        }

        physics_data.metadata.insert("wheels".to_string(), MetadataType::Wheels(self.renderizable_wheels.clone()));        
    }

    /* 
    public void SetControlInput(Vector3 input) {
        if (Dead) return;
        controlInput = Vector3.ClampMagnitude(input, 1);
    }
    */
}

/// Element-wise multiplication for Vector3
fn vector3_scale(a: Vector3<f32>, b: Vector3<f32>) -> Vector3<f32> {
    Vector3::new(a.x * b.x, a.y * b.y, a.z * b.z)
}