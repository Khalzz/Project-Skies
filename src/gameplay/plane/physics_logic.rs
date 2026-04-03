use std::collections::HashMap;
use std::sync::mpsc::Sender;

use crate::gameplay::plane::plane::{Plane, PlaneControls};
use crate::gameplay::wing::{Wing};
use crate::gameplay::airfoil::AirFoil;
use crate::gameplay::wheel::{Wheel, WheelData};
use crate::physics::physics::DebugPhysicsMessageType;
use crate::physics::physics_handler::{MetadataType, PhysicsData};
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
    // pub wings: Vec<Wing>,
    pub renderizable_wheels: HashMap<String, WheelData>,
    pub renderizable_lines: Vec<DebugPhysicsMessageType>,
    pub flight_system: FlightSystem,
    pub debug_rendering_enabled: bool,
}

impl PlanePhysicsLogic {
    pub fn new() -> Self {
        let wheels = vec![
            Wheel::new("wheel-f".to_string(), vector![0.0, 0.0, 0.7], 0.5, 15000.0, 100000.0),
            Wheel::new("wheel-lb".to_string(), vector![-0.1, 0.0, 0.0], 0.3, 10000.0, 1000.0),
            Wheel::new("wheel-rb".to_string(), vector![0.1, 0.0, 0.0], 0.3, 10000.0, 1000.0)
        ];

        // load airfoil:
        let naca_2412 = AirFoil::new("assets/aero_data/f16.ron".to_owned());
        let naca_0012 = AirFoil::new("assets/aero_data/f16-elevators.ron".to_owned());

        // i have to also add left and right ailerons
        let wings = vec![
            Wing::new(vector![8.5, 0.0, 1.0], 6.96, 2.50, 0.0, naca_2412.clone(), vector![0.0, 1.0, 0.0], 0.5, true), // left wing
            Wing::new(vector![-8.5, 0.0, 1.0], 6.96, 2.50, 0.0, naca_2412.clone(), vector![0.0, 1.0, 0.0], 0.5, true), // right wing
            Wing::new(vector![0.0, 0.0, -6.0], 6.54, 2.70, 0.0, naca_0012.clone(), vector![0.0, 1.0, 0.0], 1.0, false), // elevator wing
            Wing::new(vector![0.0, 5.0, -7.0], 6.96, 2.50, 0.0, naca_0012.clone(), vector![1.0, 0.0, 0.0], 0.5, false) // rudder wing
        ];

        Self {
            wheels,
            // wings,
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
        // Iterate over all collider handles
        for collider_handle in &physics_data.collider_handles {
            if let Some(collider) = collider_set.get(*collider_handle) {
                // Get the collider's world position (includes local offset)
                let collider_pos = collider.position();
                let col_translation = collider_pos.translation.vector;
                let col_rotation = collider_pos.rotation;
                    
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
                    
                    // Transform corners to world space using collider's actual world position
                    let corners_world: Vec<Vector3<f32>> = corners_local.iter()
                        .map(|c| col_translation + col_rotation * c)
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
        // Render collider debug wireframe only if debug is enabled
        if self.debug_rendering_enabled {
            self.render_collider_debug(collider_set, rigidbody_set, physics_data);
        }

        if let Some(rigidbody) = rigidbody_set.get_mut(physics_data.rigidbody_handle) {
            // Check if rigidbody state is valid before proceeding
            if !is_rigidbody_valid(rigidbody) {
                eprintln!("ERROR: Rigidbody has invalid state (NaN/Inf). Resetting velocity.");
                rigidbody.set_linvel(vector![0.0, 0.0, 0.0], true);
                rigidbody.set_angvel(vector![0.0, 0.0, 0.0], true);
                return;
            }

            rigidbody.reset_forces(true);
            rigidbody.reset_torques(true);

            // State calculations
            // NOTE: debug_text!() should be called from the main thread (play.rs), not physics thread
            // Use physics_data.metadata to pass debug values to the main thread if needed

            self.flight_system.calculate_state(rigidbody, delta_time);
            self.flight_system.update_thrust(rigidbody, delta_time, plane_controls.throttle);
            self.flight_system.update_drag(rigidbody);

            if self.flight_system.local_velocity.magnitude() > 0.1 {
                // flaps
                let lift_force = self.flight_system.get_lift( 1.0, Vector3::new(1.0, 0.0, 0.0));
                let yaw_force = self.flight_system.get_lift(1.0, Vector3::new(0.0, 1.0, 0.0));

                rigidbody.add_force(lift_force, true);
                rigidbody.add_force(yaw_force, true);

                let local_velocity = self.flight_system.local_velocity;
                let speed = local_velocity.z.max(0.0);

                println!("{}", speed);
                
                const MIN_STEERING_POWER: f32 = 0.2; // Minimum control authority even at zero speed
                let steering_power = MIN_STEERING_POWER + (1.0 - MIN_STEERING_POWER) * (speed / 100.0).clamp(0.0, 1.0);

                let input = Vector3::new(plane_controls.elevator, plane_controls.rudder, plane_controls.aileron);

                let turn_speed = Vector3::new(30.0, 15.0, 270.0);
                let turn_acceleration = Vector3::new(60.0, 30.0, 540.0);

                let target_av = vector3_scale(input, turn_speed * steering_power);

                let av = Vector3::new(
                    self.flight_system.local_angular_velocity.x.to_degrees(),
                    self.flight_system.local_angular_velocity.y.to_degrees(),
                    self.flight_system.local_angular_velocity.z.to_degrees(),
                );

                let correction = Vector3::new(
                    self.flight_system.calculate_steering(delta_time, av.x, target_av.x, turn_acceleration.x),
                    self.flight_system.calculate_steering(delta_time, av.y, target_av.y, turn_acceleration.y),
                    self.flight_system.calculate_steering(delta_time, av.z, target_av.z, turn_acceleration.z)
                );


                // Scale torque to overcome rigidbody inertia - correction is in degrees, convert to radians and multiply by force factor
                const TORQUE_MULTIPLIER: f32 = 100.0;
                let torque = rigidbody.rotation() * Vector3::new(
                    correction.x.to_radians() * TORQUE_MULTIPLIER,
                    correction.y.to_radians() * TORQUE_MULTIPLIER,
                    correction.z.to_radians() * TORQUE_MULTIPLIER
                );

                rigidbody.add_torque(torque, true);
            }

            
        }

        /*  
            if let Some(rigidbody) = rigidbody_set.get_mut(physics_data.rigidbody_handle) {
                for wing in &mut self.wings {
                    // wing.physics_force(rigidbody, &mut self.renderizable_lines);
                    // Update wing physics here
                }
            }
        */


        self.renderizable_wheels.clear();
        
        for (index, wheel) in self.wheels.iter_mut().enumerate() {
            if let Some((suspension_force, suspension_origin, wheel_position)) = wheel.update_wheel(&physics_data, &collider_set, rigidbody_set, &query_pipeline) {
                self.renderizable_wheels.insert(wheel.mesh_name.clone(), WheelData { wheel_position: wheel_position });
                self.renderizable_lines.push(DebugPhysicsMessageType::RenderizableLines([ManualVertex { position: [suspension_origin.x, suspension_origin.y, suspension_origin.z], color: [0.0, 1.0, 0.0] }, ManualVertex { position: [wheel_position.x, wheel_position.y, wheel_position.z], color: [0.0, 1.0, 0.0] }]));
                
                if let Some(rigidbody) = rigidbody_set.get_mut(physics_data.rigidbody_handle) {
                    rigidbody.add_force_at_point(suspension_force, suspension_origin.into(), true);
                }
            }
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