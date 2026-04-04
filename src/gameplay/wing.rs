use rapier3d::prelude::RigidBody;
use std::f32::consts::PI;
use nalgebra::vector;

use crate::{physics::physics::DebugPhysicsMessageType, primitive::manual_vertex::ManualVertex};

use super::airfoil::AirFoil;

pub struct Wing {
    pub label: String,
    pub pressure_center: nalgebra::Vector3<f32>,
    pub wing_area: f32,
    pub wing_span: f32,
    pub aspect_ratio: f32,
    pub chord: f32,
    pub air_foil: AirFoil,
    pub normal: nalgebra::Vector3<f32>,
    pub flap_ratio: f32,
    pub efficiency_factor: f32,
    pub control_input: f32,
    pub is_roll_axis: bool,
    pub last_lift_force: nalgebra::Vector3<f32>,
    pub stable: bool,
}

impl Wing {
    pub fn new(label: String, pressure_center: nalgebra::Vector3<f32>, wing_span: f32, wing_area: f32, chord: f32, air_foil: AirFoil, normal: nalgebra::Vector3<f32>, flap_ratio: f32, is_roll_axis: bool, stable: bool) -> Self {
        Self {
            label,
            wing_area,
            wing_span,
            chord,
            air_foil,
            normal,
            flap_ratio,
            pressure_center,
            aspect_ratio: wing_span.powi(2) / wing_area,
            efficiency_factor: 1.0,
            control_input: 0.05,
            is_roll_axis,
            last_lift_force: nalgebra::Vector3::zeros(),
            stable,
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
            + self.control_input * (max_deflection as f32);

        let (lift_coefficient, drag_coefficient) = self.air_foil.sample(aoa_deg);

        let lift_force = lift_dir * (dynamic_pressure * self.wing_area * lift_coefficient) * 2.0;
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

    let max_force = 5_000_000.0;
    let mag = total_force.magnitude();
    let clamped = if mag > max_force { total_force * (max_force / mag) } else { total_force };

    rigidbody.add_force_at_point(clamped.into(), world_pressure_center.into(), true);
}

    pub fn render(&self, rigidbody: &RigidBody, renderizable_lines: &mut Vec<DebugPhysicsMessageType>) {
        let world_pressure_center = rigidbody.rotation() * self.pressure_center + rigidbody.translation();
        let axis_length = 0.2;

        // X axis (red)
        let x_dir = rigidbody.rotation() * vector![axis_length, 0.0, 0.0];
        let x_origin = ManualVertex {
            position: world_pressure_center.into(),
            color: [1.0, 0.0, 0.0]
        };
        let x_end = ManualVertex {
            position: (world_pressure_center + x_dir).into(),
            color: [1.0, 0.0, 0.0]
        };
        renderizable_lines.push(DebugPhysicsMessageType::RenderizableLines([x_origin, x_end]));

        // Y axis (green)
        let y_dir = rigidbody.rotation() * vector![0.0, axis_length, 0.0];
        let y_origin = ManualVertex {
            position: world_pressure_center.into(),
            color: [0.0, 1.0, 0.0]
        };
        let y_end = ManualVertex {
            position: (world_pressure_center + y_dir).into(),
            color: [0.0, 1.0, 0.0]
        };
        renderizable_lines.push(DebugPhysicsMessageType::RenderizableLines([y_origin, y_end]));

        // Z axis (blue)
        let z_dir = rigidbody.rotation() * vector![0.0, 0.0, axis_length];
        let z_origin = ManualVertex {
            position: world_pressure_center.into(),
            color: [0.0, 0.0, 1.0]
        };
        let z_end = ManualVertex {
            position: (world_pressure_center + z_dir).into(),
            color: [0.0, 0.0, 1.0]
        };
        renderizable_lines.push(DebugPhysicsMessageType::RenderizableLines([z_origin, z_end]));

        // Lift force visualization (yellow/orange) - scaled down for visibility
        let lift_scale = 0.01; // Scale factor to make the line visible
        let lift_dir = (self.last_lift_force * lift_scale);
        let lift_origin = ManualVertex {
            position: world_pressure_center.into(),
            color: [1.0, 0.8, 0.0]
        };
        let lift_end = ManualVertex {
            position: (world_pressure_center + lift_dir).into(),
            color: [1.0, 0.5, 0.0]
        };
        renderizable_lines.push(DebugPhysicsMessageType::RenderizableLines([lift_origin, lift_end]));
    }
}