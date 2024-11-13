use std::f32::consts::PI;

use rapier3d::prelude::RigidBody;

use crate::primitive::manual_vertex::ManualVertex;

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
            efficiency_factor: 0.1,
            control_input: 0.0
        }
    }

    // fix the fact that im setting all based on the orientation of the plane instead of the wing orientation
    pub fn physics_force(&mut self, rigidbody: &mut RigidBody, renderizable_lines: &mut Vec<[ManualVertex; 2]>) {    

        let inverse_transform_direction = rigidbody.rotation().inverse() * rigidbody.linvel();
        let local_velocity = inverse_transform_direction + rigidbody.angvel().cross(&self.pressure_center);

        let speed = local_velocity.magnitude();

        if speed <= 1.0 {
            return
        }

        let drag_direction = -local_velocity.normalize();

        let lift_direction = drag_direction.cross(&self.normal).cross(&drag_direction).normalize();

        let mut angle_of_attack = drag_direction.dot(&self.normal).asin().to_degrees();

        if angle_of_attack > self.air_foil.max_alpha {
            angle_of_attack = self.air_foil.max_alpha;
        }

        if angle_of_attack < self.air_foil.min_alpha {
            angle_of_attack = self.air_foil.min_alpha;
        }

        let (mut lift_coeff, mut drag_coeff) = self.air_foil.sample(angle_of_attack);

        if self.flap_ratio > 0.0 {
            let cl_max = 1.1039;

            let deflection_rato = self.control_input;

            let delta_lift_coeff = self.flap_ratio.sqrt() * cl_max * deflection_rato;
            lift_coeff += delta_lift_coeff;
        }

        let induced_drag_coeff = lift_coeff.powi(2) / (PI * self.aspect_ratio * self.efficiency_factor);
        drag_coeff += induced_drag_coeff;

        let air_density = 1.255;

        let dynamic_pressure = 0.5 * speed.powi(2) * air_density * self.wing_area;

        let lift = lift_direction * lift_coeff * dynamic_pressure;
        let drag = drag_direction * drag_coeff * dynamic_pressure;

        let world_pressure_center = rigidbody.rotation() * self.pressure_center + rigidbody.translation();

        let world_drag = rigidbody.rotation() * drag;
        let world_lift = rigidbody.rotation() * lift;

        // rendering of the lift direction
        renderizable_lines.push([
            ManualVertex {
                position: world_pressure_center.into(),
                color: [0.0, 0.0, 1.0],
            },
            ManualVertex {
                position: (world_pressure_center - world_lift).into(),
                color: [0.0, 0.0, 1.0],
            },
        ]);

        // rendering of the drag direction
        renderizable_lines.push([
            ManualVertex {
                position: world_pressure_center.into(),
                color: [1.0, 0.0, 0.0],
            },
            ManualVertex {
                position: (world_pressure_center - world_drag).into(),
                color: [1.0, 0.0, 0.0],
            },
        ]);

        // Add the force at the rotated pressure center position in world coordinates.
        rigidbody.add_force_at_point(world_lift + world_drag, world_pressure_center.into(), true);
    }
}