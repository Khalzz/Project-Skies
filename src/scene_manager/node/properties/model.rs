use nalgebra::Vector3;

use crate::assets::Assets;
use crate::resources::{DoubleSidedMeshes, ModelSource};

/// A `Node` property naming the model it draws - a file in the assets
/// folder (`Model::new("models/f16.glb")`, see `Assets`), a model made in
/// code (see `resources::register_primitive_model`), or a file anywhere
/// (`Model::from_file`) - plus the model's own `scale`:
/// how big the model file is drawn, separate from the node's own
/// `Transform3D::scale`. The drawn size is node scale x model scale, so a
/// model authored at 1 unit long can be `Model::new("F16").scaled(14.0)` on
/// a node whose own scale stays 1 (see `Model::render_scale`). Either way
/// it's loaded with the scene that uses it, before that scene starts (see
/// `App::start_scene_load`).
#[derive(Debug, Clone)]
pub struct Model {
    pub model_ref: String,
    pub scale: Vector3<f32>,
    /// The file to load it from, for `Model::from_file` - None for one found
    /// by name (in the assets folder, or made in code).
    pub source: Option<ModelSource>,
    /// Which meshes skip backface culling, for a model found in the assets
    /// folder - a `source`'s own are on it. See `double_sided`.
    double_sided: DoubleSidedMeshes,
}

impl Model {
    /// `model_ref` at its own size (scale 1) - a path inside the assets
    /// folder ("models/f16.glb"), or the name of a model made in code.
    pub fn new(model_ref: impl Into<String>) -> Self {
        Self { model_ref: model_ref.into(), scale: Vector3::new(1.0, 1.0, 1.0), source: None, double_sided: DoubleSidedMeshes::None }
    }

    /// The model in the file at `path` (a `.glb`/`.gltf` anywhere on disk,
    /// e.g. a plane's own folder) - loaded the first time a node uses it,
    /// shared by every node using the same file.
    pub fn from_file(path: impl Into<std::path::PathBuf>) -> Self {
        Self::from_source(ModelSource::asset(path))
    }

    /// The model `source` describes.
    pub fn from_source(source: ModelSource) -> Self {
        Self { model_ref: source.key(), scale: Vector3::new(1.0, 1.0, 1.0), source: Some(source), double_sided: DoubleSidedMeshes::None }
    }

    /// Which meshes skip backface culling - drawn from both sides, for thin
    /// surfaces like an afterburner's flame:
    /// `Model::new("models/f16.glb").double_sided(DoubleSidedMeshes::Meshes(vec!["Afterburner".into()]))`.
    /// It's part of the loaded model, which every node using the same file
    /// shares - so the node that has it loaded decides (give them all the
    /// same).
    pub fn double_sided(mut self, double_sided: DoubleSidedMeshes) -> Self {
        match self.source.take() {
            Some(source) => self.source = Some(source.double_sided(double_sided)),
            None => self.double_sided = double_sided,
        }
        self
    }

    /// Where it loads from - its own `source`, or `model_ref` in the assets
    /// folder with this model's `double_sided`. None for a model made in
    /// code (see resources::register_primitive_model).
    pub(crate) fn resolve_source(&self, assets: &Assets) -> Option<ModelSource> {
        match &self.source {
            Some(source) => Some(source.clone()),
            None => assets.model_source(&self.model_ref).map(|source| source.double_sided(self.double_sided.clone())),
        }
    }

    /// A separate copy of the model, `name`d - its meshes animated apart
    /// from the shared copy's (see `ModelSource::variant`). Only for
    /// `from_file`/`from_source` models.
    pub fn variant(mut self, name: impl Into<String>) -> Self {
        if let Some(source) = self.source.take() {
            let source = source.variant(name);
            self.model_ref = source.key();
            self.source = Some(source);
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
