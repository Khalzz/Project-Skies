use nalgebra::vector;
use rapier3d::prelude::RigidBody;
use crate::gameplay::plane_logic::{airfoil::AirFoil, wing::Wing};
use crate::rendering::{manual_vertex::ManualVertex, rendering_utils::RenderizableLines};
/**
 * # Plane Logic
 * This module will be mainly dedicated to handle everything physics related for the plane, but
 * with focus on flight dynamics.
 */

pub struct PlaneLogic {
    pub wings: Vec<Wing>,
}

impl PlaneLogic {
    pub fn new() -> Self {

        // load airfoil:
        let naca_2412 = AirFoil::new("assets/aero_data/f16.ron".to_owned());
        let naca_0012 = AirFoil::new("assets/aero_data/f16-elevators.ron".to_owned());

        // i have to also add left and right ailerons
        let wings = vec![
            Wing::new(vector![8.5, 0.0, 1.0], 6.96, 2.50, 0.0, naca_2412.clone(), vector![0.0, 1.0, 0.0], 0.5), // left wing
            Wing::new(vector![-8.5, 0.0, 1.0], 6.96, 2.50, 0.0, naca_2412.clone(), vector![0.0, 1.0, 0.0], 0.5), // right wing
            Wing::new(vector![0.0, 0.0, -6.0], 6.54, 2.70, 0.0, naca_0012.clone(), vector![0.0, 1.0, 0.0], 1.0), // elevator wing
            Wing::new(vector![0.0, 5.0, -7.0], 6.96, 2.50, 0.0, naca_0012.clone(), vector![1.0, 0.0, 0.0], 0.15) // rudder wing
        ];

        Self {
            wings,
        }
    }

    pub fn update(&mut self, rigidbody: &mut RigidBody, renderizable_lines: &mut Vec<[ManualVertex; 2]>) {
        for wing in self.wings.iter_mut() {
            wing.physics_force(rigidbody, renderizable_lines);
        }
    }
}