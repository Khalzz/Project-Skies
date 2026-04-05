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

    pub fn update_thrust(&mut self, rigidbody: &mut RigidBody, deltaTime: f32, throttle: f32) {
        let thrust_local = nalgebra::Vector3::new(0.0, 0.0, 50000.0) * throttle;
        let thrust_world = rigidbody.rotation() * thrust_local;
        rigidbody.add_force(thrust_world, true);
    }
}