use std::collections::HashMap;

use nalgebra::vector;
use rapier3d::{dynamics::{RigidBody, RigidBodySet}, geometry::ColliderSet, pipeline::QueryPipeline};

use crate::{engine::physics::physics_handler::{PhysicsData, SuspensionDebugData}, game::play::plane::physics::wheels::wheel::WheelData};

use super::wheel::Wheel;

pub struct WheelManager {
    pub wheels: Vec<Wheel>,
    pub renderizable_wheels: HashMap<String, WheelData>,
}

impl WheelManager {
    pub fn new() -> Self {
      let wheels = vec![
        Wheel::new("wheel-f".to_string(), vector![0.0, 0.0, 9.8], 4.2, 100000.0, 50000.0),
        Wheel::new("wheel-lb".to_string(), vector![-1.4, 0.0, 0.0], 4.2, 500000.0, 50000.0),
        Wheel::new("wheel-rb".to_string(), vector![1.4, 0.0, 0.0], 4.2, 500000.0, 50000.0)
      ];

      Self {
        wheels,
        renderizable_wheels: HashMap::new(),
      }
    }

    pub fn update(&mut self, physics_data: &mut PhysicsData, collider_set: &ColliderSet, rigidbody_set: &mut RigidBodySet, query_pipeline: &QueryPipeline) -> Vec<SuspensionDebugData> {
      self.renderizable_wheels.clear();
      let mut suspension_debug_data: Vec<SuspensionDebugData> = Vec::new();
      
      for (index, wheel) in self.wheels.iter_mut().enumerate() {
        if let Some((suspension_force, suspension_origin, wheel_position)) = wheel.update_wheel(&physics_data, &collider_set, rigidbody_set, &query_pipeline) {
            if let Some(rigidbody) = rigidbody_set.get_mut(physics_data.rigidbody_handle) {
                rigidbody.add_force_at_point(suspension_force, suspension_origin.into(), true);
            }
            if let Some(rigidbody) = rigidbody_set.get(physics_data.rigidbody_handle) {
                let rb_pos = rigidbody.translation();
                let rb_rot = rigidbody.rotation();
                let local_position = rb_rot.inverse() * (wheel_position - rb_pos);
                let local_origin = rb_rot.inverse() * (suspension_origin - rb_pos);
                self.renderizable_wheels.insert(wheel.mesh_name.clone(), WheelData { local_position });
                suspension_debug_data.push(SuspensionDebugData {
                    local_origin,
                    local_wheel: local_position,
                });
            }
        }
      }

      return suspension_debug_data;
    }
}