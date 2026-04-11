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
    pub max_force: f32,
}

impl Wing {
    pub fn new(label: String, pressure_center: nalgebra::Vector3<f32>, wing_area: f32, chord: f32, air_foil: AirFoil, normal: nalgebra::Vector3<f32>, is_roll_axis: bool, stable: bool, incidence_angle: f32, max_force: f32) -> Self {
        Self {
            label,
            wing_area,
            chord,
            air_foil,
            normal,
            pressure_center,
            efficiency_factor: 1.0,
            control_input: 0.0,
            is_roll_axis,
            last_lift_force: nalgebra::Vector3::zeros(),
            stable,
            incidence_angle,
            max_force,
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
        
            let yaw_damping = 1000.0; // tune this
            let deflection_deg = self.control_input * max_deflection;
            let (cl, _cd) = self.air_foil.sample(deflection_deg);
            let control_authority = dynamic_pressure * self.wing_area * cl.abs();
            
            // Damping resists sideslip, control_input steers into it
            let side_force_magnitude = (-sideslip_speed * yaw_damping) + (self.control_input.signum() * control_authority);
            
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

            // Angular damping: resist pitch/roll oscillation proportional to angular velocity
            // The wing's angular velocity contribution creates a local AoA change that opposes rotation
            let damping_coefficient = 5000.0;
            let angular_vel = rigidbody.angvel();
            let arm = rigidbody.rotation() * self.pressure_center;
            let damping_velocity = angular_vel.cross(&arm);
            let damping_force = -damping_velocity * damping_coefficient * self.wing_area;

            self.last_lift_force = lift_force;
            lift_force + drag_force + damping_force
        };

        // Parasitic drag: opposes forward airspeed only (local Z), not gravity
        let parasitic_cd = 0.02;
        let forward_drag = -local_velocity.z.signum() * local_velocity.z * local_velocity.z * 0.5 * air_density * self.wing_area * parasitic_cd;
        let parasitic_drag_local = nalgebra::Vector3::new(0.0, 0.0, forward_drag);
        let parasitic_drag = rigidbody.rotation() * parasitic_drag_local;
        let total_force = total_force + parasitic_drag;

        // Normal (flat-plate) drag: when air hits the wing perpendicular to its surface
        // (e.g. during stall, falling, or sideslip), the wing acts like a flat plate
        // Scaled by thickness ratio (~4% for NACA 64A204) since thin wings present little frontal area
        let flat_plate_cd = 1.28;
        let thickness_ratio = 0.04; // 4% thick airfoil
        let wing_normal_world = rigidbody.rotation() * nalgebra::Vector3::y();
        let normal_speed = world_velocity.dot(&wing_normal_world);
        let normal_drag_magnitude = 0.5 * air_density * normal_speed * normal_speed.abs() * self.wing_area * thickness_ratio * flat_plate_cd;
        let normal_drag = -wing_normal_world * normal_drag_magnitude;
        let total_force = total_force + normal_drag;

        // Lateral (spanwise) drag: air hitting the wing edge during sideslip
        // Frontal area ~ chord * thickness, approximated as wing_area * 0.1
        let lateral_cd = 1.0;
        let lateral_area = self.wing_area * 0.1;
        let span_axis_world = rigidbody.rotation() * self.normal;
        let lateral_speed = world_velocity.dot(&span_axis_world);
        let lateral_drag_magnitude = 0.5 * air_density * lateral_speed * lateral_speed.abs() * lateral_area * lateral_cd;
        let lateral_drag = -span_axis_world * lateral_drag_magnitude;
        let total_force = total_force + lateral_drag;

        let mag = total_force.magnitude();
        let clamped = if mag > self.max_force { total_force * (self.max_force / mag) } else { total_force };

        rigidbody.add_force_at_point(clamped.into(), world_pressure_center.into(), true);
    }
}