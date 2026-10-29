use nalgebra::Vector3;

use crate::asset_manager::AssetManager;
use crate::resources::{self, DoubleSidedMeshes, ModelLoad, ModelSource};

/// A `Node` property: the model it draws, and that model's own `scale`.
///
/// Where the model comes from is its `source` - a file in the assets folder
/// (`Model::new(ModelSource::file("models/f16.glb"))`), a mesh the game made
/// (`ModelSource::mesh`, see `AssetManager::add_mesh`), or one of the
/// engine's shapes (`ModelSource::Cube`, `ModelSource::Plane`). Whichever it
/// is, it's loaded with the scene that uses it, before that scene starts
/// (see `Scene::assets_to_load`), and shared by every node using the same.
///
/// `scale` is how big the model is drawn, separate from the node's own
/// `Transform3D::scale`. The drawn size is node scale x model scale, so a
/// model authored at 1 unit long can be `.scaled(14.0)` on a node whose own
/// scale stays 1 (see `Model::render_scale`).
#[derive(Debug, Clone)]
pub struct Model {
    pub source: ModelSource,
    pub scale: Vector3<f32>,
    /// See `double_sided`.
    double_sided: DoubleSidedMeshes,
    /// See `variant`.
    variant: Option<String>,
}

impl Model {
    /// The model `source` gives, at its own size (scale 1).
    pub fn new(source: ModelSource) -> Self {
        Self { source, scale: Vector3::new(1.0, 1.0, 1.0), double_sided: DoubleSidedMeshes::None, variant: None }
    }

    /// Which meshes skip backface culling - drawn from both sides, for thin
    /// surfaces like an afterburner's flame:
    /// `.double_sided(DoubleSidedMeshes::Meshes(vec!["Afterburner".into()]))`.
    /// It's part of the loaded model, which every node using the same one
    /// shares - so the node that has it loaded decides (give them all the
    /// same).
    pub fn double_sided(mut self, double_sided: DoubleSidedMeshes) -> Self {
        self.double_sided = double_sided;
        self
    }

    /// A separate copy of the model, `name`d - for when two uses of one
    /// model need its meshes animated differently (mesh transforms are the
    /// model's, shared by every node drawing it): a missile on a pylon
    /// (plume off) and one in flight (plume lit). A light copy - its own mesh
    /// transforms, the shared copy's geometry and textures.
    pub fn variant(mut self, name: impl Into<String>) -> Self {
        self.variant = Some(name.into());
        self
    }

    /// Drawn `scale` times its own size, the same on every axis.
    pub fn scaled(self, scale: f32) -> Self {
        self.with_scale(Vector3::new(scale, scale, scale))
    }

    /// Drawn `scale` times its own size, per axis.
    pub fn with_scale(mut self, scale: Vector3<f32>) -> Self {
        self.scale = scale;
        self
    }

    /// The scale the model is actually drawn at on a node scaled
    /// `node_scale`: the two multiplied, per axis.
    pub fn render_scale(&self, node_scale: Vector3<f32>) -> Vector3<f32> {
        node_scale.component_mul(&self.scale)
    }

    /// What its loaded copy is kept under - its source's, and its variant's
    /// name if it's one.
    pub(crate) fn key(&self) -> String {
        resources::variant_key(&self.source.key(), self.variant.as_deref())
    }

    pub(crate) fn variant_name(&self) -> Option<&str> {
        self.variant.as_deref()
    }

    /// What loading its shared copy takes - or why it can't be (no such file
    /// or mesh).
    pub(crate) fn load(&self, assets: &AssetManager) -> Result<ModelLoad, String> {
        assets.model_load(&self.source, &self.double_sided)
    }
}
