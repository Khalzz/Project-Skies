use rapier3d::prelude::RigidBody;
use std::f32::consts::PI;
use nalgebra::vector;

use crate::{physics::physics::DebugPhysicsMessageType, primitive::manual_vertex::ManualVertex};

use super::airfoil::AirFoil;

pub struct Wing {
    pub label: String,
    pub pressure_center: nalgebra::Vector3<f32>,
    pub wing_area: f32,
    pub chord: f32,
    pub air_foil: AirFoil,
    pub normal: nalgebra::Vector3<f32>,
    pub efficiency_factor: f32,
    pub control_input: f32,
    pub is_roll_axis: bool,
    pub last_lift_force: nalgebra::Vector3<f32>,
    pub stable: bool,
    pub incidence_angle: f32,
}

impl Wing {
    pub fn new(label: String, pressure_center: nalgebra::Vector3<f32>, wing_area: f32, chord: f32, air_foil: AirFoil, normal: nalgebra::Vector3<f32>, is_roll_axis: bool, stable: bool, incidence_angle: f32) -> Self {
        Self {
            label,
            wing_area,
            chord,
            air_foil,
            normal,
            pressure_center,
            efficiency_factor: 1.0,
            control_input: 0.1,
            is_roll_axis,
            last_lift_force: nalgebra::Vector3::zeros(),
            stable,
            incidence_angle,
        }
    }

    pub fn physics_force(&mut self, rigidbody: &mut RigidBody) {
        let world_pressure_center = rigidbody.rotation() * self.pressure_center
            + rigidbody.translation();

        let angular_contribution = rigidbody.angvel()
            .cross(&(rigidbody.rotation() * self.pressure_center));
        let world_velocity = rigidbody.linvel() + angular_contribution;
        let local_velocity = rigidbody.rotation().inverse() * world_velocity;

        if local_velocity.magnitude() < 0.01 {
            return;
        }

        let forward_speed = local_velocity.z;
        let vertical_speed = local_velocity.y;

        let max_deflection = if self.is_roll_axis { 25.0 } else { 25.0 };
        let air_density = 1.225f32;
        let speed_sq = local_velocity.magnitude_squared();
        let dynamic_pressure = 0.5 * air_density * speed_sq;

        let velocity_dir_world = (rigidbody.rotation() * local_velocity).normalize();
        let span_axis_world = rigidbody.rotation() * self.normal;
        let lift_dir = span_axis_world.cross(&velocity_dir_world).normalize();
        let velocity_dir_local = local_velocity.normalize();

        let total_force = if self.stable {
            let sideslip_speed = local_velocity.x; // lateral velocity in local space
        
            let yaw_damping = 50000.0; // tune this
            let control_authority = dynamic_pressure * self.wing_area * (max_deflection as f32).to_radians();
            
            // Damping resists sideslip, control_input steers into it
            let side_force_magnitude = (-sideslip_speed * yaw_damping) + (self.control_input * control_authority);
            
            let side_axis_world = rigidbody.rotation() * nalgebra::Vector3::x();
            let side_force = side_axis_world * side_force_magnitude;
            
            self.last_lift_force = side_force;
            side_force
        } else {
            let aoa_deg = vertical_speed.atan2(forward_speed).to_degrees()
                + self.incidence_angle
                + self.control_input * (max_deflection as f32);

            let (lift_coefficient, drag_coefficient) = self.air_foil.sample(aoa_deg);

            let lift_force = lift_dir * (dynamic_pressure * self.wing_area * lift_coefficient);
            let drag_force = rigidbody.rotation() * (-velocity_dir_local * dynamic_pressure * self.wing_area * drag_coefficient);

            self.last_lift_force = lift_force;
            lift_force + drag_force
        };

        // Parasitic drag: opposes forward airspeed only (local Z), not gravity
        let parasitic_cd = 0.02;
        let forward_drag = -local_velocity.z.signum() * local_velocity.z * local_velocity.z * 0.5 * air_density * self.wing_area * parasitic_cd;
        let parasitic_drag_local = nalgebra::Vector3::new(0.0, 0.0, forward_drag);
        let parasitic_drag = rigidbody.rotation() * parasitic_drag_local;
        let total_force = total_force + parasitic_drag;

        let max_force = 500_000.0;
        let mag = total_force.magnitude();
        let clamped = if mag > max_force { total_force * (max_force / mag) } else { total_force };

        rigidbody.add_force_at_point(clamped.into(), world_pressure_center.into(), true);
    }
}