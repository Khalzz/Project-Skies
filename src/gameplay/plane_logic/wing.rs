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
    pub control_input: f32,
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
            control_input: 0.0,
        }
    }

    pub fn physics_force(&mut self, rigidbody: &mut RigidBody, renderizable_lines: &mut Vec<[ManualVertex; 2]>) {    
        
    }
}