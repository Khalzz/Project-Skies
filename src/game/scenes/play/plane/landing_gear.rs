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

struct LandingGearWheel {
    // Same mesh names `WheelManager` keys its wheel data by.
    mesh_name: &'static str,
    // Fully retracted - tucked in near the fuselage centerline. Hand-picked;
    // eyeball against the model.
    retracted_position: Vector3<f32>,
    // Fully deployed pose, used ONLY as a fallback when there's no live
    // suspension raycast data yet (first frames / physics not running).
    // Normally the deployed pose is the raycast point, so the wheels track
    // the ground / bumps.
    deployed_fallback: Vector3<f32>,
}

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
/// published in `AircraftState`. Holds no gear state of its own.
pub struct GearMeshes {
    wheels: Vec<LandingGearWheel>,
}

impl GearMeshes {
    pub fn new() -> Self {
        Self {
            wheels: vec![
                LandingGearWheel { mesh_name: "wheel-f",  retracted_position: Vector3::new(0.0, 0.00, 0.456),  deployed_fallback: Vector3::new(0.0, -0.279, 0.756) },
                LandingGearWheel { mesh_name: "wheel-lb", retracted_position: Vector3::new(-0.08, 0.02, 0.367), deployed_fallback: Vector3::new(-0.2, -0.266, 0.167) },
                LandingGearWheel { mesh_name: "wheel-rb", retracted_position: Vector3::new(0.08, 0.02, 0.367),  deployed_fallback: Vector3::new(0.2, -0.266, 0.167) },
            ],
        }
    }

    /// Positions every wheel mesh, blending each between its retracted pose
    /// and its live deployed pose (raycast point, or the fallback) by
    /// `deploy`.
    pub fn place(
        &self,
        model: &mut LoadedModel,
        wheel_data: &HashMap<String, WheelData>,
        deploy: f32,
        instance_scale: Vector3<f32>,
        queue: &wgpu::Queue,
    ) {
        let Some(meshes) = model.mesh_lists.get_mut("opaque") else { return };
        for lg in &self.wheels {
            let Some(mesh) = meshes.get_mut(lg.mesh_name) else { continue };
            let deployed = match wheel_data.get(lg.mesh_name) {
                Some(d) => Vector3::new(
                    d.local_position.x / instance_scale.x,
                    d.local_position.y / instance_scale.y,
                    d.local_position.z / instance_scale.z,
                ),
                None => lg.deployed_fallback,
            };
            mesh.transform.position = lerp_vector3(lg.retracted_position, deployed, deploy);
            mesh.update_transform(queue);
        }
    }
}
