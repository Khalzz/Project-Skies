use rapier3d::prelude::RigidBody;
use std::f32::consts::PI;
use nalgebra::vector;

use crate::{primitive::manual_vertex::ManualVertex, rendering::render_line::render_basic_line};

use super::airfoil::AirFoil;

pub struct Wing {
    pub pressure_center: nalgebra::Vector3<f32>,
    pub wing_area: f32,
    pub wing_span: f32,
    pub aspect_ratio: f32,
    pub chord: f32,
    pub air_foil: AirFoil,
    pub normal: nalgebra::Vector3<f32>,
    pub flap_ratio: f32,
    pub efficiency_factor: f32,
    pub control_input: f32
}

impl Wing {
    pub fn new(pressure_center: nalgebra::Vector3<f32>, wing_span: f32, wing_area: f32, chord: f32, air_foil: AirFoil, normal: nalgebra::Vector3<f32>, flap_ratio: f32) -> Self {
        Self { 
            wing_area, 
            wing_span, 
            chord,
            air_foil, 
            normal, 
            flap_ratio,
            pressure_center,
            aspect_ratio: wing_span.powi(2) / wing_area,
            efficiency_factor: 1.0,
            control_input: 0.0
        }
    }

    /// # Simplified Physics Force
    /// This function will apply a force on a especific point of the plane 
    pub fn _physics_force(&mut self, rigidbody: &mut RigidBody, renderizable_lines: &mut Vec<[ManualVertex; 2]>) {    
        // To make this movement first try setting a force point on each aero body based on controller
        
        // Show the position of each aero foil.

        // Define a force based on the direction is applied and input
            // rudder will apply a force on (rudder position), with a base direction of vector.x, with the input of yaw
            // elevator will apply a force on (elevator position), with a base direction of vector.y, with the input of y
            // aleron will apply a force on (aleron 1 or 2 position), with a base direction of vector.y, with the input of x
        let world_pressure_center = rigidbody.rotation() * self.pressure_center + rigidbody.translation();

        render_basic_line(renderizable_lines, world_pressure_center, [0.0, 1.0, 1.0], world_pressure_center + ((rigidbody.rotation() * self.normal) * self.control_input), [0.0, 1.0, 1.0]);
        // rigidbody.add_force_at_point(world_pressure_center + ((rigidbody.rotation() * self.normal * 100.0) * self.control_input), world_pressure_center.into(), true);
    }

    pub fn physics_force(&mut self, rigidbody: &mut RigidBody, renderizable_lines: &mut Vec<[ManualVertex; 2]>) {    
        // Transform the local pressure center into world space
        let world_pressure_center = rigidbody.rotation() * self.pressure_center + rigidbody.translation();
    
        // Calculate local velocity in the wing's local space, adjusting for rotation
        let inverse_transform_direction = rigidbody.rotation().inverse() * rigidbody.linvel();
        let local_velocity = inverse_transform_direction + rigidbody.angvel();
        // let local_velocity = inverse_transform_direction + rigidbody.angvel().cross(&world_pressure_center);

        // let local_velocity = Self::get_point_velocity(&rigidbody, &self.pressure_center);

        let speed = local_velocity.magnitude();
        if speed <= 1.0 {
            return;
        }
    
        // Calculate drag and lift directions in the world space
        let drag_direction = -local_velocity.normalize();
        let lift_direction = drag_direction.cross(&self.normal).cross(&drag_direction).normalize();
    
        // Calculate the angle of attack, ensuring it is based on the plane's orientation
        let angle_of_attack = (drag_direction.dot(&self.normal).asin().to_degrees()).clamp(self.air_foil.min_alpha, self.air_foil.max_alpha);
    

        // Sample the lift and drag coefficients based on the angle of attack
        let (mut lift_coeff, mut drag_coeff) = self.air_foil.sample(angle_of_attack);
    
        // Apply flap effects if any
        if self.flap_ratio > 0.0 {
            let cl_max = 1.1039;
            let deflection_ratio = self.control_input;
            let delta_lift_coeff = self.flap_ratio.sqrt() * cl_max * deflection_ratio;
            lift_coeff += delta_lift_coeff;
        }

        // Calculate induced drag based on lift and wing characteristics
        let induced_drag_coeff = lift_coeff.powi(2) / (PI * self.aspect_ratio * self.efficiency_factor);
        drag_coeff += induced_drag_coeff;
    
        let air_density = 1.255;
        let dynamic_pressure = 0.5 * speed.powi(2) * air_density * self.wing_area;
    
        // Calculate lift and drag forces in local space
        let lift = lift_direction * lift_coeff * dynamic_pressure;
        let drag = drag_direction * drag_coeff * dynamic_pressure;
    
        // Rotate lift and drag forces into world space
        let world_drag = rigidbody.rotation() * drag;
        let world_lift = rigidbody.rotation() * lift;
    
        // lift debug
        render_basic_line(renderizable_lines, world_pressure_center.into(), [0.0, 0.0, 1.0],  ((world_pressure_center - ((world_lift.normalize() * 5.0) * lift_coeff))).into(), [0.0, 0.0, 1.0]);

        // Drag debug
        render_basic_line(renderizable_lines, world_pressure_center.into(), [1.0, 0.0, 0.0],  (world_pressure_center - world_drag).into(), [1.0, 0.0, 0.0]);

        // Wing Direction debug
        render_basic_line(renderizable_lines, world_pressure_center.into(), [1.0, 1.0, 1.0], (world_pressure_center + (world_lift + world_drag)).into(), [1.0, 1.0, 1.0]);

    
        // Apply forces at the rotated pressure center position in world coordinates
        rigidbody.add_force_at_point(world_lift + world_drag, world_pressure_center.into(), true);
        
        
        let angular_velocity = rigidbody.angvel();
        let angular_damping_factor = 0.99;
        rigidbody.set_angvel(angular_velocity * angular_damping_factor, true);
        
    }
}