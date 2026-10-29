use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::rendering::instance_management::ModelDataInstance;
use crate::rendering::models::model::Model;
use crate::resources::MeshData;

/// Every model in memory, by key (see ModelSource::key).
#[derive(Default)]
pub struct ModelCache {
    /// Drawn by at least one node - each with its GPU instance buffer.
    pub(crate) drawn: HashMap<String, ModelDataInstance>,
    /// Loaded, drawn by nothing yet - a model moves to `drawn` with its first
    /// node (see render_bridge::register_static_model): an instance buffer
    /// can't be made empty.
    pub(crate) idle: HashMap<String, Model>,
    /// The ones the open scene loaded - dropped with it (see
    /// resources::unload_level_models).
    pub(crate) scene_owned: HashSet<String>,
    /// The game's own meshes, by name (see AssetManager::add_mesh) - kept
    /// while the game runs, built into a model by each scene using one.
    pub(crate) meshes: HashMap<String, Arc<MeshData>>,
}

impl ModelCache {
    pub(crate) fn is_loaded(&self, key: &str) -> bool {
        self.drawn.contains_key(key) || self.idle.contains_key(key)
    }
}
