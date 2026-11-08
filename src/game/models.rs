//! The models the game's scenes use, by the names they were known by - each
//! an engine `Model` property read from its file in the assets folder (the
//! engine loads it when a scene that uses it is built, and drops it when no
//! scene does).

use nalgebra::Vector3;
use pankarta_software::prelude::*;

pub const F16: &str = "F16";
pub const WATER: &str = "Water";
pub const F14: &str = "F14";
pub const GROUND: &str = "Ground";
pub const RUNWAY: &str = "Runway";
pub const MQ9: &str = "MQ9";

/// The model called `name` - see the constants above. An unknown name is
/// taken as a file path inside the assets folder.
pub fn model(name: &str) -> Model {
    match name {
        // Same file the player's jet uses (assets/planes/f16/data.ron), so
        // both share one load.
        F16 => Model::file("planes/f16/model/f16.glb").double_sided(DoubleSidedMeshes::Meshes(vec!["Afterburner".to_owned()])),
        WATER => Model::file("models/Water/water.gltf"),
        F14 => Model::file("models/F14/F14.gltf"),
        GROUND => Model::file("models/ground/ground.glb"),
        RUNWAY => Model::file("models/Runway/Runway.glb").double_sided(DoubleSidedMeshes::All),
        MQ9 => Model::file("models/MQ-9/mq-9.glb"),
        path => Model::file(asset_relative(path)),
    }
}

/// The file path `name`'s model is read from, inside the assets folder.
pub fn model_path(name: &str) -> Option<String> {
    model(name).file_path().map(str::to_owned)
}

/// `path` (as the game's data files write it: "assets/planes/f16/...",
/// relative to where the game runs) inside the assets folder.
pub fn asset_relative(path: &str) -> String {
    let path = path.replace('\\', "/");
    let path = path.trim_start_matches("./");
    path.strip_prefix("assets/").unwrap_or(path).to_owned()
}

/// The ocean surface's mesh for an eye at `altitude` - see
/// game::ext::mesh_data::register_ocean - drawn with the ocean shader.
pub fn ocean_surface(altitude: f32) -> Mesh {
    let tier = crate::game::ext::mesh_data::ocean_tier_for(crate::OCEAN_TIERS, altitude);
    Mesh::custom(crate::game::ext::mesh_data::ocean_tier_mesh(crate::OCEAN_SURFACE_MODEL, tier))
        .material(crate::game::ocean::Ocean::material())
        .casts_shadows(false)
}

/// Keeps the ocean node `node` on the tier for an eye at `altitude` - its
/// material (and the params the ocean set on it) kept.
pub fn update_ocean_tier(scene: &mut Scene, node: &str, altitude: f32) {
    let tier = crate::game::ext::mesh_data::ocean_tier_for(crate::OCEAN_TIERS, altitude);
    let name = crate::game::ext::mesh_data::ocean_tier_mesh(crate::OCEAN_SURFACE_MODEL, tier);
    let Some(mesh) = scene.get_node_mut(node).and_then(|node| node.get_property_mut::<Mesh>()) else { return };
    if let MeshShape::Custom(current) = &mesh.shape {
        if *current != name {
            mesh.shape = MeshShape::Custom(name);
        }
    }
}

/// The model in the file at `path` (as the game's data files write it -
/// see `asset_relative`), drawn `scale` times its size.
pub fn file_model(path: &str, scale: f32) -> Model {
    Model::file(asset_relative(path)).transform(Transform3D { scale: Vector3::new(scale, scale, scale), ..Default::default() })
}

/// `model`, drawn `scale` times its size.
pub fn scaled(model: Model, scale: f32) -> Model {
    model.transform(Transform3D { scale: Vector3::new(scale, scale, scale), ..Default::default() })
}
