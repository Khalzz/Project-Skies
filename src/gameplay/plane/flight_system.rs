use nalgebra::{Vector3, clamp};
use rapier3d::prelude::RigidBody;
use crate::gameplay::plane::utils;

pub struct AoA {
    pub aoa_pitch: f32,
    pub aoa_yaw: f32,
}

pub struct FlightSystem {
    pub velocity: nalgebra::Vector3<f32>,
    pub local_velocity: nalgebra::Vector3<f32>,
    pub local_angular_velocity: nalgebra::Vector3<f32>,
    pub aoa: AoA,
    pub last_velocity: nalgebra::Vector3<f32>,
    pub g_force: f32,
    pub input: Vector3<f32>, // x = roll, y = pitch, z = yaw
}

impl FlightSystem {

    pub fn new() -> Self {
        Self {
            velocity: nalgebra::Vector3::new(0.0, 0.0, 0.0),
            local_velocity: nalgebra::Vector3::new(0.0, 0.0, 0.0),
            local_angular_velocity: nalgebra::Vector3::new(0.0, 0.0, 0.0),
            aoa: AoA {
                aoa_pitch: 0.0,
                aoa_yaw: 0.0,
            },
            last_velocity: nalgebra::Vector3::new(0.0, 0.0, 0.0),
            g_force: 0.0,
            input: Vector3::new(0.0, 0.0, 0.0),
        }
    }

    pub fn calculate_state(&mut self, rigidbody: &RigidBody, deltaTime: f32) {
        let invRotation = rigidbody.rotation().inverse();
        self.velocity = rigidbody.linvel().clone();
        self.local_velocity = invRotation * self.velocity;
        self.local_angular_velocity = invRotation * rigidbody.angvel();
        
        // Safety check: if any values are NaN/Inf, reset to zero
        if !self.local_velocity.iter().all(|v| v.is_finite()) {
            eprintln!("WARNING: local_velocity had NaN/Inf, resetting");
            self.local_velocity = nalgebra::Vector3::zeros();
        }
        if !self.local_angular_velocity.iter().all(|v| v.is_finite()) {
            eprintln!("WARNING: local_angular_velocity had NaN/Inf, resetting");
            self.local_angular_velocity = nalgebra::Vector3::zeros();
        }

        self.calculate_angle_of_attack();
        self.calculate_g_force(rigidbody, deltaTime, self.velocity);
    }

    pub fn calculate_angle_of_attack(&mut self) {
        if self.velocity.magnitude() > 0.1 {
            // Use atan2 for angle calculation - safe even when z is small/negative
            // But clamp to ±90° to prevent extreme values during vertical flight
            self.aoa.aoa_pitch = self.local_velocity.y.atan2(self.local_velocity.z.abs().max(0.01));
            self.aoa.aoa_yaw = self.local_velocity.x.atan2(self.local_velocity.z.abs().max(0.01));
            
            // Clamp to reasonable range (±85 degrees)
            const MAX_AOA: f32 = std::f32::consts::FRAC_PI_2 * 0.94; // ~85°
            self.aoa.aoa_pitch = self.aoa.aoa_pitch.clamp(-MAX_AOA, MAX_AOA);
            self.aoa.aoa_yaw = self.aoa.aoa_yaw.clamp(-MAX_AOA, MAX_AOA);
        } else {
            self.aoa.aoa_pitch = 0.0;
            self.aoa.aoa_yaw = 0.0;
        }
    }

    pub fn calculate_g_force(&mut self, rigidbody: &rapier3d::prelude::RigidBody, deltaTime: f32, last_velocity: nalgebra::Vector3<f32>) {
        // Skip if deltaTime is too small to avoid division by zero / Inf
        if deltaTime < 0.0001 {
            return;
        }
        let invRotation = rigidbody.rotation().inverse();
        let acceleration = (self.velocity - last_velocity) / deltaTime;
        let local_g_force = invRotation * acceleration;
        self.g_force = local_g_force.magnitude() / 9.81; // Convert to Gs
    }

    pub fn update_thrust(&mut self, rigidbody: &mut RigidBody, deltaTime: f32, throttle: f32) {
        // Thrust in positive Z direction (this was working correctly before)
        rigidbody.add_force(nalgebra::Vector3::new(0.0, 0.0, 1000000.0) * throttle, true);
    }

    pub fn update_drag(&mut self, rigidbody: &mut RigidBody) {
        let lv = self.local_velocity;
        let lv_mag = lv.magnitude();
        
        // Skip drag calculation if velocity is too small to avoid NaN from normalize()
        if lv_mag < 0.001 {
            return;
        }
        
        let lv2 = lv_mag * lv_mag;

        // To the drag forwards its the one we should add drag based on airbrakes or flaps
        const drag_forwards: f32 = 1.0; // here we should evaluate all the values in a curve based on the local velocity of each value for example here on the localvelocity on the z axis
        const drag_back: f32 = 2.0; 
        const drag_left: f32 = 2.0;
        const drag_right: f32 = 2.0;
        const drag_top: f32 = 2.0;
        const drag_bottom: f32 = 2.0;

        let coefficient = utils::scale_6(lv, drag_right, drag_left, drag_top, drag_bottom, drag_forwards, drag_back);

        let drag = coefficient.magnitude() * lv2 * -(lv / lv_mag);

        // Safety check for NaN/Inf before applying drag
        if !drag.x.is_nan() && !drag.y.is_nan() && !drag.z.is_nan() 
            && drag.x.is_finite() && drag.y.is_finite() && drag.z.is_finite() {
            rigidbody.add_force(drag, true);
        }
    }

    pub fn get_lift(&mut self, lift_power: f32, right_axis: Vector3<f32>) -> Vector3<f32> {
        // Angle of attack yaw
        // vector 3 up as right axis
        // lift power as calculated number
        // Project the velocity onto the plane perpendicular to the right_axis
        let right_axis_mag = right_axis.magnitude();
        if right_axis_mag < 0.001 {
            return Vector3::zeros();
        }
        let right_axis_normalized = right_axis / right_axis_mag;
        
        let lv = self.local_velocity;
        let lift_velocity = lv - right_axis_normalized * lv.dot(&right_axis_normalized);
        let lift_velocity_mag = lift_velocity.magnitude();
        
        // Skip lift calculation if velocity projection is too small to avoid NaN from normalize()
        if lift_velocity_mag < 0.001 {
            return Vector3::zeros();
        }
        
        let v2 = lift_velocity_mag * lift_velocity_mag;

        // Clamp angle of attack to reasonable range to prevent extreme values
        let aoa_deg = self.aoa.aoa_pitch.to_degrees().clamp(-90.0, 90.0);
        let lift_coefficient = Self::lift_coefficient(aoa_deg);
        let lift_force = lift_coefficient * v2 * lift_power;
        
        // Clamp lift force to prevent extreme values
        let lift_force_clamped = lift_force.clamp(-1e8, 1e8);

        let lift_direction = Vector3::cross(&(lift_velocity / lift_velocity_mag), &right_axis);
        let lift_direction_mag = lift_direction.magnitude();
        
        // If cross product is near zero (vectors nearly parallel), no lift
        if lift_direction_mag < 0.001 {
            return Vector3::zeros();
        }
        
        let lift = (lift_direction / lift_direction_mag) * lift_force_clamped;

        return lift;
    }

    pub fn calculate_steering(&mut self, deltaTime: f32, angular_velocity: f32, target_velocity: f32, acceleration: f32) -> f32 {
        let error = target_velocity - angular_velocity;
        let accel = deltaTime * acceleration;
        return clamp(error, -accel, accel);
    }

    pub fn lift_coefficient(angle_of_attack_deg: f32) -> f32 {
        // Clamp input to prevent extreme values, then compute lift coefficient
        let clamped_aoa = angle_of_attack_deg.clamp(-90.0, 90.0);
        (2.0 * clamped_aoa.to_radians()).sin()
    }
}