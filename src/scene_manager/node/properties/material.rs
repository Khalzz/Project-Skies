use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::rendering::shader_library::ParamValue;

/// A `Node` property choosing the shader its model is drawn with, by name
/// (see engine::rendering::shader_library) - "engine/standard", or one of
/// the game's like "hologram" (`assets/shaders/hologram.wgsl`) - the values
/// it sets for that shader's `Params` (on top of its `.ron`'s), and other
/// materials for some of the model's meshes, by mesh name. A node without
/// one is drawn with "engine/standard".
///
/// In a data file: `(shader: "hologram", params: { "tint": (1, 0.2, 0.2, 0.8) },
/// meshes: { "Canopy": (shader: "gold_canopy") })` - `params` and `meshes`
/// optional.
///
/// A model's mesh materials are the model's - meant for a model only this
/// node draws (each aircraft has its own copy of its plane's).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Material {
    pub shader: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, ParamValue>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub meshes: BTreeMap<String, Material>,
}

impl Material {
    pub fn shader(name: impl Into<String>) -> Self {
        Self { shader: name.into(), params: BTreeMap::new(), meshes: BTreeMap::new() }
    }

    /// Builder: sets its shader's param `name`.
    pub fn param(mut self, name: &str, value: impl Into<ParamValue>) -> Self {
        self.set(name, value);
        self
    }

    /// Sets its shader's param `name` - while it's drawn, too.
    pub fn set(&mut self, name: &str, value: impl Into<ParamValue>) {
        self.params.insert(name.to_owned(), value.into());
    }

    /// Builder: the model's mesh `mesh` drawn with `material` instead.
    pub fn mesh(mut self, mesh: &str, material: Material) -> Self {
        self.meshes.insert(mesh.to_owned(), material);
        self
    }

    /// The material instance it is, on the node `node` (see
    /// ShaderLibrary::set_instance): just its shader's name when it sets no
    /// params (it's the same as every other use of it), the node's own
    /// otherwise.
    pub fn key(&self, node: &str) -> String {
        if self.params.is_empty() { self.shader.clone() } else { format!("{}@{node}", self.shader) }
    }

    /// `key`, for this as the material of the mesh `mesh` of the node `node`.
    pub fn mesh_key(&self, node: &str, mesh: &str) -> String {
        if self.params.is_empty() { self.shader.clone() } else { format!("{}@{node}/{mesh}", self.shader) }
    }

    /// Whether it's just engine/standard - nothing to put on a node.
    pub fn is_standard(&self) -> bool {
        self.shader == crate::rendering::shader_library::STANDARD && self.params.is_empty() && self.meshes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_material_reads_from_data() {
        let material: Material = ron::from_str("(shader: \"hologram\", params: { \"tint\": (1, 0.2, 0.2, 0.8), \"speed\": 4 }, meshes: { \"Canopy\": (shader: \"gold_canopy\") })").unwrap();
        assert_eq!(material.params["tint"].0, [1.0, 0.2, 0.2, 0.8]);
        assert_eq!(material.meshes["Canopy"].shader, "gold_canopy");
        // Its own values make it the node's own material; none, the shader's.
        assert_eq!(material.key("drone"), "hologram@drone");
        assert_eq!(material.meshes["Canopy"].mesh_key("drone", "Canopy"), "gold_canopy");
        assert!(ron::from_str::<Material>("(shader: \"x\", parms: {})").is_err(), "a misspelled field");
        assert!(Material::shader(crate::rendering::shader_library::STANDARD).is_standard());
    }
}
