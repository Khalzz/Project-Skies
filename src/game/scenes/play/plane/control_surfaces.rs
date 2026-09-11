use nalgebra::{Quaternion, Unit, UnitQuaternion, Vector3};

use crate::engine::rendering::models::model::Model as LoadedModel;
use crate::engine::utils::lerps::lerp_quaternion;
use crate::transform::Transform;

use super::controls::PlaneControls;

/// Which of `PlaneControls`'s instantaneous inputs a `ControlSurface` reacts
/// to - trim is deliberately not an option here, callers that want it just
/// add it into the surface's own resolved value before it gets used (none do
/// yet - the model's control surfaces currently only animate off raw stick
/// input, trim only affects the invisible aerodynamic forces in
/// `wing_manager.rs`).
pub enum ControlInput {
    Elevator,
    Aileron,
    Rudder,
}

impl ControlInput {
    fn value(&self, controls: &PlaneControls) -> f32 {
        match self {
            // Negated - see PlaneControls.elevator's own assignment (in
            // Plane::update) for why: that value's sign convention flipped
            // (W now +1/S now -1, matching aileron/rudder's own convention)
            // but the elevator control surface's own visual animation
            // direction shouldn't change just because of that, so this
            // undoes the flip locally. TESTING: when
            // debug_simulated_elevator_control_input is available, this uses
            // that instead - it's already in the SAME convention as -elevator
            // (the elevator wing's own control_input is defined as
            // -plane_controls.elevator in wing_manager.rs's non-fly-by-wire
            // path, so this is a like-for-like swap, not an extra negation),
            // so the mesh shows the simulated wing's real, post-solve state
            // rather than raw stick. See that field's own doc comment.
            ControlInput::Elevator => controls.debug_simulated_elevator_control_input.unwrap_or(-controls.elevator),
            ControlInput::Aileron => controls.aileron,
            ControlInput::Rudder => controls.rudder,
        }
    }
}

/// What a `ControlSurface`'s target rotation is composed with, before the
/// control-driven part - three different needs, all seen on the F16's own
/// meshes: `None` (elevators - the control input is the whole rotation),
/// `Fixed` (rudders - each has a constant real-world tilt baked in,
/// authored here rather than in the mesh, composed with the control-driven
/// swing), `Captured` (ailerons - there's no fixed/known rest tilt, so the
/// first frame this surface is ever animated just remembers whatever the
/// mesh's own rotation already was and treats that as neutral).
pub enum SurfaceBase {
    None,
    Fixed(UnitQuaternion<f32>),
    Captured(Option<Quaternion<f32>>),
}

/// One animated part of the plane's model - a named mesh, which axis/how far
/// it swings per unit of control input, how fast it eases toward that target,
/// and what it's relative to (see `SurfaceBase`). `Plane` holds a `Vec` of
/// these (see `Plane::default_control_surfaces`) and drives all of them
/// through the same `apply` each frame, instead of a hand-written block per
/// mesh repeating the same lerp-and-write.
pub struct ControlSurface {
    mesh_list: &'static str,
    mesh_name: &'static str,
    axis: Vector3<f32>,
    scale: f32,
    lerp_speed: f32,
    base: SurfaceBase,
    input: ControlInput,
}

impl ControlSurface {
    pub fn new(mesh_list: &'static str, mesh_name: &'static str, axis: Vector3<f32>, scale: f32, lerp_speed: f32, base: SurfaceBase, input: ControlInput) -> Self {
        Self { mesh_list, mesh_name, axis, scale, lerp_speed, base, input }
    }

    pub fn apply(&mut self, model: &mut LoadedModel, controls: &PlaneControls, delta_time: f32, queue: &wgpu::Queue) {
        let Some(meshes) = model.mesh_lists.get_mut(self.mesh_list) else { return };
        let Some(mesh) = meshes.get_mut(self.mesh_name) else { return };

        let base = match &mut self.base {
            SurfaceBase::None => UnitQuaternion::identity(),
            SurfaceBase::Fixed(rotation) => *rotation,
            SurfaceBase::Captured(captured) => UnitQuaternion::from_quaternion(*captured.get_or_insert(mesh.transform.rotation)),
        };

        let control_value = self.input.value(controls);
        let target = base * UnitQuaternion::from_axis_angle(&Unit::new_normalize(self.axis), self.scale * control_value);
        let new_rotation = lerp_quaternion(mesh.transform.rotation, *target, delta_time * self.lerp_speed);
        let new_transform = Transform::new(mesh.transform.position, new_rotation, mesh.transform.scale);
        mesh.change_transform(queue, new_transform);
    }
}
