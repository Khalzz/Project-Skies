use std::collections::HashMap;

use nalgebra::{UnitQuaternion, Vector3};

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
/// - starts down or up, as the spawn says (see `Airframe::new`);
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
        self.deploy = super::airframe::approach(self.deploy, target, rate, delta_time);
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
/// - retracted: in its bay - the wheel's `retracted_position`, turned by
///   its `retracted_rotation` (data.ron), or with those left out tucked
///   straight up to its suspension's mount point;
/// - in between: blended by `deploy` - moved along the line between the
///   two, and turned a matching part of the way about its own middle.
pub struct GearMeshes {
    /// Each wheel's suspension mount (model space), by mesh name, from the
    /// last wheel data that had it - the wheels (the plane's data.ron gear)
    /// are still known while the gear is up, when there's no data.
    mounts: HashMap<String, Vector3<f32>>,
    /// Strut mounts in the jet's frame, and the deploy to show, until the
    /// physics thread has reported the gear. An air start is up on the
    /// first frame instead of sitting in the model's gear-down pose.
    armed: Option<(Vec<(String, Vector3<f32>)>, f32)>,
    /// Each wheel's pose with the gear up, by mesh name: where its middle
    /// goes (the jet's frame; None - its mount) and how it's turned - from
    /// the plane's data.ron gear (see `set_gear`).
    stowed: HashMap<String, (Option<Vector3<f32>>, UnitQuaternion<f32>)>,
}

impl GearMeshes {
    pub fn new() -> Self {
        Self { mounts: HashMap::new(), armed: None, stowed: HashMap::new() }
    }

    /// Where `gear`'s wheels go with the gear up - set at spawn, and again
    /// when the data.ron is saved.
    pub fn set_gear(&mut self, gear: &super::gear_spec::GearSpec) {
        self.stowed = gear.wheels.iter()
            .map(|wheel| (wheel.mesh.clone(), (wheel.retracted_position, wheel.retracted_turn())))
            .collect();
    }

    /// Show `gear` at `down` until physics reports. `down`: extended.
    pub fn arm(&mut self, gear: &super::gear_spec::GearSpec, down: bool) {
        self.set_gear(gear);
        self.armed = Some((
            gear.wheels.iter().map(|wheel| (wheel.mesh.clone(), wheel.position)).collect(),
            if down { 1.0 } else { 0.0 },
        ));
    }

    /// The deploy `arm` asked for, before physics has reported.
    pub fn armed_deploy(&self) -> Option<f32> {
        self.armed.as_ref().map(|(_, deploy)| *deploy)
    }

    /// Physics has reported - its pose takes over.
    pub fn disarm(&mut self) {
        self.armed = None;
    }

    /// Puts the armed pose on the wheels. Physics replaces it.
    pub fn show_armed(&mut self, model: &mut LoadedModel, instance_scale: Vector3<f32>, queue: &wgpu::Queue) {
        let Some((wheels, deploy)) = self.armed.clone() else { return };
        let to_model = |point: Vector3<f32>| point.component_div(&instance_scale);
        for (name, position) in wheels {
            self.mounts.insert(name, to_model(position));
        }
        self.place(model, &HashMap::new(), deploy, instance_scale, queue);
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

            let (stowed_at, stowed_turn) = self.stowed.get(name).copied().unwrap_or((None, UnitQuaternion::identity()));
            let retracted = stowed_at.map_or(*mount, to_model);
            let deployed = match wheel_data.get(name) {
                // The tyre's bottom on the suspension's end point - the
                // ground contact, or the ray's full length in the air.
                Some(data) => to_model(data.local_position) + Vector3::y() * radius,
                None => modelled,
            };

            // The wheel's middle goes there, turned the part of the way to
            // its stowed angle the gear is up - about that middle: its mesh
            // position is offset by wherever the (turned) geometry sits
            // relative to its origin.
            let center = lerp_vector3(retracted, deployed, deploy);
            let turn = UnitQuaternion::identity().slerp(&stowed_turn, (1.0 - deploy).clamp(0.0, 1.0));
            for mesh in meshes.iter_mut() {
                let modelled_rotation = UnitQuaternion::from_quaternion(mesh.base_transform.rotation);
                mesh.transform.rotation = (turn * modelled_rotation).into_inner();
                mesh.transform.position = center - turn * mesh.center_offset();
                mesh.update_transform(queue);
            }
        }
    }
}