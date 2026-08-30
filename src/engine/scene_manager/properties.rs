use nalgebra::{UnitQuaternion, Vector3};

/// A `Node` property giving it a 3D position/rotation/scale.
///
/// Deliberately independent of `crate::transform::Transform` and
/// `engine::game_nodes::game_object::Transform` (both tied to the existing
/// `data.ron`-loaded object pipeline, the second via a custom `Deserialize`
/// impl converting Euler angles to a quaternion) - this property system is
/// code-first by design and doesn't go through either's deserialization path.
#[derive(Debug, Clone, Copy)]
pub struct Transform3D {
    pub position: Vector3<f32>,
    pub rotation: UnitQuaternion<f32>,
    pub scale: Vector3<f32>,
}

impl Default for Transform3D {
    fn default() -> Self {
        Transform3D {
            position: Vector3::zeros(),
            rotation: UnitQuaternion::identity(),
            scale: Vector3::new(1.0, 1.0, 1.0),
        }
    }
}

/// A `Node` property pointing at a loaded model by name - matches the
/// `model_ref` keys already used in `App::game_models`
/// (`HashMap<String, ModelDataInstance>`).
///
/// Actually connecting this to the renderer - registering a live instance and
/// writing per-frame instance buffers the way `App::renderizable_instances`
/// does for `data.ron`-loaded objects today - is deliberately left as
/// follow-up work, not part of this pass.
#[derive(Debug, Clone)]
pub struct Model {
    pub model_ref: String,
}
