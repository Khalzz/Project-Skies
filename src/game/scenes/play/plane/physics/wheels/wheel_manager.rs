use std::collections::HashMap;

use nalgebra::Vector3;
use rapier3d::{dynamics::{RigidBodyHandle, RigidBodySet}, geometry::ColliderSet, pipeline::QueryPipeline};

use crate::game::scenes::play::plane::physics::wheels::wheel::WheelData;

use super::wheel::{GroundSurface, SuspensionHit, Wheel};
use crate::game::scenes::play::plane::gear_spec::{GearSpec, SteeringSpec};

/// What the pilot is doing to the wheels this step.
pub struct WheelInputs {
    /// -1..1, positive = right - scaled by the gear's SteeringSpec.
    pub steering: f32,
    /// 0..1 on every `braked` wheel.
    pub brake: f32,
}

pub struct WheelManager {
    pub wheels: Vec<Wheel>,
    pub renderizable_wheels: HashMap<String, WheelData>,
    steering: SteeringSpec,
}

impl WheelManager {
    /// The wheels `gear` (the plane's data.ron) describes.
    pub fn new(gear: &GearSpec) -> Self {
        let mut manager = Self { wheels: Vec::new(), renderizable_wheels: HashMap::new(), steering: gear.steering.clone() };
        manager.set_gear(gear);
        manager
    }

    /// Swaps in new wheels (data.ron edited mid-flight - see
    /// messages::AircraftReload). Each suspension ray casts DOWN
    /// `suspension_length` from its `position` (rigidbody-local meters -
    /// the body has no scale); the wheel mesh rests on whatever point it
    /// returns (see GearMeshes::place). The rear wheels must sit behind the
    /// center of mass and the front one well ahead of it, or the suspension
    /// tips the jet onto its tail on the ground.
    pub fn set_gear(&mut self, gear: &GearSpec) {
        self.steering = gear.steering.clone();
        self.wheels = gear.wheels.iter().map(|spec| {
            let mut wheel = Wheel::new(spec.mesh.clone(), spec.position, spec.suspension_length, spec.stiffness, spec.damping);
            if spec.steerable {
                wheel = wheel.steerable();
            }
            if spec.braked {
                wheel = wheel.braked();
            }
            wheel
        }).collect();
        self.renderizable_wheels.clear();
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
      let steer_angle = inputs.steering.clamp(-1.0, 1.0) * self.steering.max_angle_at(ground_speed).to_radians();

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
                self.renderizable_wheels.insert(wheel.mesh_name.clone(), WheelData { local_position, mount: wheel.offset, grounded: ground.is_some() });
            }
        }
      }
    }
}
