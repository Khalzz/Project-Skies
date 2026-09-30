use nalgebra::{UnitQuaternion, Vector3};

use crate::resources::{DoubleSidedMeshes, ModelSource};

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
/// (`HashMap<String, ModelDataInstance>`) - plus the model's own `scale`:
/// how big the model file is drawn, separate from the node's own
/// `Transform3D::scale`. The drawn size is node scale x model scale, so a
/// model authored at 1 unit long can be `Model::new("F16").scaled(14.0)` on
/// a node whose own scale stays 1 (see `Model::render_scale`). Build it with
/// `Model::new` (a model named with
/// `resources::declare_model`) or `Model::from_file` (a file on disk) -
/// either way it's loaded the first time a node uses it, not before (see
/// `resources::load_scene_model`).
///
/// Actually connecting this to the renderer - registering a live instance and
/// writing per-frame instance buffers the way `App::renderizable_instances`
/// does for `data.ron`-loaded objects today - is deliberately left as
/// follow-up work, not part of this pass.
#[derive(Debug, Clone)]
pub struct Model {
    pub model_ref: String,
    pub scale: Vector3<f32>,
    /// The file to load it from, for `Model::from_file` - None for a model
    /// named with `resources::declare_model` (or a procedural one).
    pub source: Option<ModelSource>,
}

impl Model {
    /// `model_ref` at its own size (scale 1).
    pub fn new(model_ref: impl Into<String>) -> Self {
        Self { model_ref: model_ref.into(), scale: Vector3::new(1.0, 1.0, 1.0), source: None }
    }

    /// The model in the file at `path` (a `.glb`/`.gltf` anywhere on disk,
    /// e.g. a plane's own folder) - loaded the first time a node uses it,
    /// shared by every node using the same file.
    pub fn from_file(path: impl Into<std::path::PathBuf>) -> Self {
        Self::from_source(ModelSource::asset(path))
    }

    /// The model `source` describes.
    pub fn from_source(source: ModelSource) -> Self {
        Self { model_ref: source.key(), scale: Vector3::new(1.0, 1.0, 1.0), source: Some(source) }
    }

    /// Which meshes skip backface culling - only meaningful for
    /// `from_file`/`from_source` models (a declared one sets its own).
    pub fn double_sided(mut self, double_sided: DoubleSidedMeshes) -> Self {
        if let Some(source) = self.source.take() {
            self.source = Some(source.double_sided(double_sided));
        }
        self
    }

    /// Drawn `scale` times its file size, the same on every axis.
    pub fn scaled(self, scale: f32) -> Self {
        self.with_scale(Vector3::new(scale, scale, scale))
    }

    /// Drawn `scale` times its file size, per axis.
    pub fn with_scale(mut self, scale: Vector3<f32>) -> Self {
        self.scale = scale;
        self
    }

    /// The scale the model is actually drawn at on a node scaled
    /// `node_scale`: the two multiplied, per axis.
    pub fn render_scale(&self, node_scale: Vector3<f32>) -> Vector3<f32> {
        node_scale.component_mul(&self.scale)
    }
}
