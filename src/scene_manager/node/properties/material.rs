use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::rendering::shader_library::ParamValue;

/// What a tiled texture's size is measured on - see `Material::tiled`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextureSpace {
    /// On the object, scaled like it - the texture moves and turns with it,
    /// and keeps its size however it's scaled. For things that move.
    Object,
    /// In the world - the texture stays where it is, and lines up across
    /// neighbouring objects. For things that don't: floors, walls.
    World,
}

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
    /// An image (a path inside the assets folder) drawn on the model in
    /// place of its own texture - see `texture`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture: Option<String>,
}

impl Material {
    pub fn shader(name: impl Into<String>) -> Self {
        Self { shader: name.into(), params: BTreeMap::new(), meshes: BTreeMap::new(), texture: None }
    }

    /// engine/standard - the look every model has without a material.
    pub fn standard() -> Self {
        Self::shader(crate::rendering::shader_library::STANDARD)
    }

    /// Builder: draws the image `path` (inside the assets folder) on the
    /// model in place of its own texture. Loaded with the scene, through the
    /// AssetManager (`app.assets.textures`), and shared by every node using
    /// it. It repeats past its edges: texture coordinates past 0..1 tile it.
    pub fn texture(mut self, path: impl Into<String>) -> Self {
        self.texture = Some(path.into());
        self
    }

    /// Builder: its texture (see `texture`) at a fixed size - one copy every
    /// `tile_size` meters, repeated across the surface however big it is,
    /// measured on the object or in the world (`space`). Draws with
    /// engine/tiled.
    pub fn tiled(mut self, tile_size: f32, space: TextureSpace) -> Self {
        self.shader = crate::rendering::shader_library::TILED.to_owned();
        self.param("tile_size", tile_size).param("world", if space == TextureSpace::World { 1.0 } else { 0.0 })
    }

    /// Every texture it draws with - its own, and its meshes' materials'.
    pub fn textures(&self) -> impl Iterator<Item = &String> {
        self.texture.iter().chain(self.meshes.values().flat_map(|mesh| mesh.texture.iter()))
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
        match (self.params.is_empty(), &self.texture) {
            (true, None) => self.shader.clone(),
            // The same texture and shader on two nodes: one material, drawn
            // together.
            (true, Some(texture)) => format!("{}|{texture}", self.shader),
            (false, _) => format!("{}@{node}", self.shader),
        }
    }

    /// `key`, for this as the material of the mesh `mesh` of the node `node`.
    pub fn mesh_key(&self, node: &str, mesh: &str) -> String {
        match (self.params.is_empty(), &self.texture) {
            (true, None) => self.shader.clone(),
            (true, Some(texture)) => format!("{}|{texture}", self.shader),
            (false, _) => format!("{}@{node}/{mesh}", self.shader),
        }
    }

    /// Whether it's just engine/standard - nothing to put on a node.
    pub fn is_standard(&self) -> bool {
        self.shader == crate::rendering::shader_library::STANDARD && self.params.is_empty() && self.meshes.is_empty() && self.texture.is_none()
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
