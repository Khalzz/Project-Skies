use crate::app::App;
use crate::game_nodes::game_object::{GameObject, Transform as GameObjectTransform};
use crate::rendering::instance_management::{InstanceData, ModelDataInstance};
use crate::resources;

use super::node::properties::{Material, Model};
use crate::rendering::shader_library::STANDARD;
use super::scene::Scene;

/// Takes the node `id`'s instance off the renderer - the reverse of
/// `register_static_model`. Its model stays loaded; the shared instance
/// buffer keeps its size (the per-frame write just fills fewer slots), only
/// the instance count drops.
pub fn unregister_model(scene: &mut Scene, app: &mut App, id: &str) {
    let Some(removed) = scene.content.renderizable_instances.remove(id) else { return };
    let remaining = scene.content.renderizable_instances.values()
        .filter(|instance| instance.model_ref == removed.model_ref)
        .count() as u32;
    if let Some(model) = app.assets.models.drawn.get_mut(&removed.model_ref) {
        model.instance_count = remaining;
    }
}

/// Bridges a spawned `Node`'s `Transform3D`+`Model` properties into the
/// existing renderer, which only walks `SceneContent::renderizable_instances`/
/// `ModelCache::drawn` today, not `Node`s (see `Model` in
/// `node/properties/model.rs`).
///
/// The model is normally loaded already - by the scene's loading step (see
/// `App::start_scene_load`), or by another node using it. One spawned while
/// the scene runs, using a model nothing loaded yet, is loaded here, on the
/// spot (see `Scene::will_use` to have it loaded with the scene instead).
///
/// What this always rebuilds is that model's shared instance buffer: it's a
/// single fixed-size GPU buffer covering every instance of that model at once
/// (see `create_instance_buffer`), so adding one more instance needs a fresh,
/// larger buffer, not just an in-place write - this recomputes it from every
/// `renderizable_instances` entry currently sharing `model_ref` (this node
/// included) each time. That makes this call O(current instance count of
/// that model), fine for "a new object appears" - it's not what keeps
/// existing instances' positions updated frame to frame, `App::run`'s own
/// per-frame `write_buffer` loop already does that cheaply, without
/// reallocating, and keeps working unchanged once this has sized the buffer
/// correctly.
///
/// One real gap, not handled here: removing a node doesn't shrink/rebuild the
/// buffer back down - only growth (a node newly referencing a model) is
/// covered, matching how far this system's been built out so far.
pub fn register_static_model(scene: &mut Scene, app: &mut App, id: &str) -> Result<(), String> {
    let node = scene.content.nodes.get(id).ok_or_else(|| format!("no node named '{id}' to register"))?;
    let transform = *node.transform_3d().ok_or_else(|| format!("node '{id}' has no Transform3D property"))?;
    let model = node.get_property::<Model>().ok_or_else(|| format!("node '{id}' has no Model property"))?;
    let material = node.get_property::<Material>().map_or_else(|| STANDARD.to_owned(), |material| material.key(id));
    // Node scale x the model's own - see Model::render_scale.
    let render_scale = model.render_scale(transform.scale);
    // Kept under its source's key (and variant's) - see Model::key.
    let model_ref = model.key();
    let variant = model.variant_name().map(str::to_owned);
    let load = model.load(&app.assets);

    let game_object = GameObject {
        id: id.to_owned(),
        model: model_ref.clone(),
        transform: GameObjectTransform {
            position: transform.position,
            rotation: transform.rotation,
            scale: render_scale,
        },
        children: vec![],
    };

    // Reuse the mesh already loaded - either drawn already
    // (assets.models.drawn) or loaded but not drawn yet (assets.models.idle).
    let loaded_model = match app.assets.models.drawn.remove(&model_ref) {
        Some(existing) => existing.model,
        // First node in this scene using it - loaded already by the scene's
        // loading step, or now.
        None => resources::load_scene_model(app, &load.map_err(|error| format!("node '{id}': {error}"))?, variant.as_deref())?,
    };

    scene.content.renderizable_instances.insert(id.to_owned(), InstanceData {
        renderizable_transform: game_object.transform,
        instance: game_object,
        model_ref: model_ref.clone(),
        material,
    });

    // Rebuild the shared instance buffer from every renderizable instance
    // that now references this model - includes the node just inserted above.
    let instances: Vec<&GameObject> = scene.content.renderizable_instances.values()
        .filter(|instance| instance.model_ref == model_ref)
        .map(|instance| &instance.instance)
        .collect();
    let instance_count = instances.len() as u32;
    let camera_position = scene.cameras.active().camera.position().coords;
    let instance_buffer = resources::create_instance_buffer(&instances, &app.renderer.device, camera_position);

    app.assets.models.drawn.insert(model_ref, ModelDataInstance {
        model: loaded_model,
        instance_buffer,
        instance_count,
        material_ranges: Vec::new(),
        mesh_materials: Default::default(),
        stamp: ModelDataInstance::new_stamp(),
    });

    Ok(())
}
