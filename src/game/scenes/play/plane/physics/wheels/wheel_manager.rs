use std::collections::HashMap;

use nalgebra::{vector, Vector3};
use rapier3d::{dynamics::{RigidBodyHandle, RigidBodySet}, geometry::ColliderSet, pipeline::QueryPipeline};

use crate::game::scenes::play::plane::physics::wheels::wheel::WheelData;

use super::wheel::{GroundSurface, SuspensionHit, Wheel};

/// Nose-wheel steering authority, schedule on ground speed: full lock at
/// taxi speed, tapering to a few degrees by takeoff-roll speed so a full
/// pedal input at 60 m/s doesn't snap the jet sideways. F-16 NWS is ~±32°.
const MAX_STEER_DEG: f32 = 32.0;
const HIGH_SPEED_STEER_DEG: f32 = 4.0;
const STEER_TAPER_START_MS: f32 = 8.0;
const STEER_TAPER_END_MS: f32 = 40.0;

/// What the pilot is doing to the wheels this step.
pub struct WheelInputs {
    /// -1..1, positive = right - scaled by the speed schedule above.
    pub steering: f32,
    /// 0..1 on every `braked` wheel.
    pub brake: f32,
}

pub struct WheelManager {
    pub wheels: Vec<Wheel>,
    pub renderizable_wheels: HashMap<String, WheelData>,
}

impl WheelManager {
    pub fn new() -> Self {
      // Suspension ray origins, in rigidbody-local WORLD units (the body has
      // no scale). The ray casts DOWN `max_suspension_length` from each; the
      // wheel mesh is placed at whatever point it returns (contact point, or
      // the ray's far end when airborne - see GearMeshes::place).
      //
      // The REARS must sit behind the rigidbody's centre of mass (z = 0.5,
      // see the "player" node's RigidBodyData) and the FRONT well ahead of it,
      // or the upward suspension force tips the airframe onto its tail on the
      // ground. That's why these don't just mirror the wheel meshes' own
      // model positions (rears are at z ~= 2.34 there, which is ahead of the
      // CG) - a small visual offset between the mesh's authored spot and the
      // ray endpoint is the trade for a stable ground stance.
      let wheels = vec![
        Wheel::new("wheel-f".to_string(), vector![0.0, 0.0, 9.8], 4.2, 100000.0, 50000.0).steerable(),
        Wheel::new("wheel-lb".to_string(), vector![-1.4, 0.0, 0.0], 4.2, 500000.0, 50000.0).braked(),
        Wheel::new("wheel-rb".to_string(), vector![1.4, 0.0, 0.0], 4.2, 500000.0, 50000.0).braked()
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
    ///
    /// Every grounded wheel also gets tyre friction (see `Wheel::tyre_force`)
    /// - rolling resistance, `inputs.brake` on the braked wheels, sideways
    /// grip, and `inputs.steering` turning the steerable ones - scaled by the
    /// same load the suspension is carrying.
    pub fn update(&mut self, deploy: f32, inputs: &WheelInputs, body: RigidBodyHandle, collider_set: &ColliderSet, rigidbody_set: &mut RigidBodySet, query_pipeline: &QueryPipeline) {
      self.renderizable_wheels.clear();

      if deploy <= 0.0 {
        return;
      }

      let force_scale = deploy.clamp(0.0, 1.0);
      let ground_speed = rigidbody_set.get(body).map(|rigidbody| rigidbody.linvel().magnitude()).unwrap_or(0.0);
      let steer_angle = inputs.steering.clamp(-1.0, 1.0) * max_steer_angle_deg(ground_speed).to_radians();

      for wheel in self.wheels.iter_mut() {
        wheel.steer_angle = if wheel.steerable { steer_angle } else { 0.0 };
        wheel.last_tyre_force = Vector3::zeros();

        if let Some(SuspensionHit { force, origin, wheel_position, ground }) = wheel.update_wheel(body, collider_set, rigidbody_set, query_pipeline) {
            if let Some(rigidbody) = rigidbody_set.get_mut(body) {
                let suspension_force = force * force_scale;
                rigidbody.add_force_at_point(suspension_force, origin.into(), true);

                if let Some(ground) = ground {
                    let surface = GroundSurface::of_collider(ground, collider_set);
                    let load = suspension_force.y.max(0.0);
                    let tyre_force = wheel.tyre_force(rigidbody, wheel_position, load, inputs.brake, &surface);
                    rigidbody.add_force_at_point(tyre_force, wheel_position.into(), true);
                    wheel.last_tyre_force = tyre_force;
                }
            }
            if let Some(rigidbody) = rigidbody_set.get(body) {
                let local_position = rigidbody.rotation().inverse() * (wheel_position - rigidbody.translation());
                self.renderizable_wheels.insert(wheel.mesh_name.clone(), WheelData { local_position, grounded: ground.is_some() });
            }
        }
      }
    }
}

fn max_steer_angle_deg(ground_speed_ms: f32) -> f32 {
    let t = ((ground_speed_ms - STEER_TAPER_START_MS) / (STEER_TAPER_END_MS - STEER_TAPER_START_MS)).clamp(0.0, 1.0);
    MAX_STEER_DEG + (HIGH_SPEED_STEER_DEG - MAX_STEER_DEG) * t
}
