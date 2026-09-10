use std::collections::HashMap;

use nalgebra::vector;
use rapier3d::{dynamics::{RigidBody, RigidBodySet}, geometry::ColliderSet, pipeline::QueryPipeline};

use crate::{engine::physics::physics_handler::{PhysicsData, SuspensionDebugData}, game::scenes::play::plane::physics::wheels::wheel::WheelData};

use super::wheel::Wheel;

pub struct WheelManager {
    pub wheels: Vec<Wheel>,
    pub renderizable_wheels: HashMap<String, WheelData>,
}

impl WheelManager {
    pub fn new() -> Self {
      // Suspension ray origins, in rigidbody-local WORLD units (the body has
      // no scale). The ray casts DOWN `max_suspension_length` from each; the
      // wheel mesh is placed at whatever point it returns (contact point, or
      // the ray's far end when airborne - see Plane::apply_physics_feedback).
      //
      // The REARS must sit behind the rigidbody's centre of mass (z = 0.5,
      // see the "player" node's RigidBodyData) and the FRONT well ahead of it,
      // or the upward suspension force tips the airframe onto its tail on the
      // ground. That's why these don't just mirror the wheel meshes' own
      // model positions (rears are at z ~= 2.34 there, which is ahead of the
      // CG) - a small visual offset between the mesh's authored spot and the
      // ray endpoint is the trade for a stable ground stance.
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

    /// `deploy` is the landing gear's 0..1 extension (0 = fully retracted,
    /// 1 = down and locked). The raycast ALWAYS runs while `deploy > 0` - the
    /// gear state machine needs the ground-contact result to decide whether
    /// to slam the gear back down mid-retraction. The suspension FORCE,
    /// though, is scaled by `deploy`: a half-extended strut can't hold the
    /// airframe's full weight ("not enough force until fully open"). At
    /// `deploy == 0` nothing runs at all.
    pub fn update(&mut self, deploy: f32, physics_data: &mut PhysicsData, collider_set: &ColliderSet, rigidbody_set: &mut RigidBodySet, query_pipeline: &QueryPipeline) -> Vec<SuspensionDebugData> {
      self.renderizable_wheels.clear();
      let mut suspension_debug_data: Vec<SuspensionDebugData> = Vec::new();

      if deploy <= 0.0 {
        return suspension_debug_data;
      }

      let force_scale = deploy.clamp(0.0, 1.0);

      for wheel in self.wheels.iter_mut() {
        if let Some((suspension_force, suspension_origin, wheel_position, grounded)) = wheel.update_wheel(&physics_data, &collider_set, rigidbody_set, &query_pipeline) {
            if let Some(rigidbody) = rigidbody_set.get_mut(physics_data.rigidbody_handle) {
                rigidbody.add_force_at_point(suspension_force * force_scale, suspension_origin.into(), true);
            }
            if let Some(rigidbody) = rigidbody_set.get(physics_data.rigidbody_handle) {
                let rb_pos = rigidbody.translation();
                let rb_rot = rigidbody.rotation();
                let local_position = rb_rot.inverse() * (wheel_position - rb_pos);
                let local_origin = rb_rot.inverse() * (suspension_origin - rb_pos);
                self.renderizable_wheels.insert(wheel.mesh_name.clone(), WheelData { local_position, grounded });
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