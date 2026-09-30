use std::collections::HashMap;

use nalgebra::Vector3;

use crate::engine::rendering::models::model::Model as LoadedModel;
use crate::engine::utils::lerps::lerp_vector3;
use crate::game::scenes::play::plane::physics::wheels::wheel::WheelData;

// Gear extend/retract speed, as a fraction of the full cycle per second
// (so 0.55 => ~1.8s each way). EMERGENCY is the rate used when ground contact
// forces the gear back down mid-cycle.
const GEAR_RATE_PER_S: f32 = 0.55;
const GEAR_EMERGENCY_RATE_PER_S: f32 = 3.0;
// At/above this `deploy` the gear counts as down-and-locked.
const GEAR_LOCKED_THRESHOLD: f32 = 0.999;

/// Landing-gear state machine - lives on the physics thread (inside
/// `AircraftPhysics`), since `deploy` directly scales the suspension force.
/// `deploy` is the whole state: 0.0 = up and stowed, 1.0 = down and locked,
/// anything between = mid-cycle. Published back in `AircraftState` so the
/// main thread can place the wheel meshes from it (see `GearMeshes`).
///
/// Rules:
/// - starts down (see `Airframe::new`);
/// - can't retract while down-and-locked AND a wheel is on the ground;
/// - freely toggled in the air;
/// - if a wheel ray finds ground while the gear is mid-cycle (not
///   down-and-locked), it's retracted the rest of the way UP, fast - a
///   part-extended strut can't safely carry the airframe.
pub struct LandingGear {
    /// 0..1 gear extension.
    pub deploy: f32,
    /// What the pilot last commanded (`true` = down).
    commanded_down: bool,
    /// Any wheel ray touched ground last physics step - refreshed by
    /// `AircraftPhysics::fixed_update` right after the wheel raycasts.
    pub any_grounded: bool,
}

impl LandingGear {
    pub fn new(starts_down: bool) -> Self {
        Self {
            deploy: if starts_down { 1.0 } else { 0.0 },
            commanded_down: starts_down,
            any_grounded: false,
        }
    }

    fn locked_down(&self) -> bool {
        self.deploy >= GEAR_LOCKED_THRESHOLD
    }

    /// Pilot pressed the gear toggle. No-op only when the gear is already
    /// down-and-locked and a wheel is on the ground (can't retract the gear
    /// you're standing on); otherwise flips the command.
    pub fn request_toggle(&mut self) {
        self.set_commanded_down(!self.commanded_down);
    }

    pub fn set_commanded_down(&mut self, down: bool) {
        if !down && self.locked_down() && self.any_grounded {
            return; // reject "gear up" with weight on wheels
        }
        self.commanded_down = down;
    }

    /// Advance one physics step. Uses `any_grounded` from the previous step's
    /// wheel raycasts.
    pub fn tick(&mut self, delta_time: f32) {
        // A wheel ray found ground while the gear is NOT down-and-locked -
        // i.e. it's mid-cycle. A part-extended strut can't safely carry the
        // airframe, so pull the gear the rest of the way UP (fast) and get it
        // out of the way rather than leaving it half-out under load.
        let forced = self.any_grounded && !self.locked_down();
        if forced {
            self.commanded_down = false;
        }

        let target = if self.commanded_down { 1.0 } else { 0.0 };
        let rate = if forced { GEAR_EMERGENCY_RATE_PER_S } else { GEAR_RATE_PER_S };
        let step = rate * delta_time;
        self.deploy = (self.deploy + (target - self.deploy).clamp(-step, step)).clamp(0.0, 1.0);
    }
}

/// The landing gear's visual half - lives on the main thread (inside
/// `Plane`), placing the wheel meshes from whatever `LandingGear` last
/// published in `AircraftState`.
///
/// Works from each wheel's actual geometry (see Mesh::local_bounds), not
/// its mesh origin - an exporter often leaves a wheel's origin at the
/// model's, with the wheel baked off to the side - so it fits any model:
/// - deployed: on its suspension - the tyre resting on the point the
///   suspension ray returns (its middle one tyre radius above it): the
///   ground where it touches, or the ray's full length (the suspension's
///   max extension) when there's nothing under it;
/// - retracted: tucked straight up to its suspension's mount point;
/// - in between: blended by `deploy`.
pub struct GearMeshes {
    /// Each wheel's suspension mount (model space), by mesh name, from the
    /// last wheel data that had it - the wheels (the plane's data.ron gear)
    /// are still known while the gear is up, when there's no data.
    mounts: HashMap<String, Vector3<f32>>,
}

impl GearMeshes {
    pub fn new() -> Self {
        Self { mounts: HashMap::new() }
    }

    /// Positions every wheel mesh (all its materials), for `deploy` (0 =
    /// up, 1 = down). `instance_scale` maps model space to the world.
    pub fn place(
        &mut self,
        model: &mut LoadedModel,
        wheel_data: &HashMap<String, WheelData>,
        deploy: f32,
        instance_scale: Vector3<f32>,
        queue: &wgpu::Queue,
    ) {
        let to_model = |world: Vector3<f32>| world.component_div(&instance_scale);
        for (name, data) in wheel_data {
            self.mounts.insert(name.clone(), to_model(data.mount));
        }
        for (name, mount) in self.mounts.iter() {
            let mut meshes: Vec<_> = model.meshes_named_mut("opaque", name).collect();
            let Some(first) = meshes.first() else { continue };
            // Where the wheel was modelled (its geometry's middle) - only
            // until the first suspension data arrives - and its radius, the
            // biggest of its materials (the tyre).
            let modelled = first.base_transform.position + first.center_offset();
            // Measured straight down in the model, whichever way the wheel
            // mesh is turned - so the tyre's bottom sits exactly on the
            // ground, with nothing to set by hand.
            let radius = meshes.iter().map(|mesh| mesh.depth_below_center()).fold(0.0f32, f32::max);

            let retracted = *mount;
            let deployed = match wheel_data.get(name) {
                // The tyre's bottom on the suspension's end point - the
                // ground contact, or the ray's full length in the air.
                Some(data) => to_model(data.local_position) + Vector3::y() * radius,
                None => modelled,
            };

            // The wheel's middle goes there - its mesh position is offset by
            // wherever the geometry sits relative to its origin.
            let center = lerp_vector3(retracted, deployed, deploy);
            for mesh in meshes.iter_mut() {
                mesh.transform.position = center - mesh.center_offset();
                mesh.update_transform(queue);
            }
        }
    }
}