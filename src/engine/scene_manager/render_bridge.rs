use crate::app::App;
use crate::engine::game_nodes::game_object::{GameObject, MetaData, Transform as GameObjectTransform};
use crate::engine::rendering::instance_management::{InstanceData, ModelDataInstance};
use crate::resources;

use super::properties::{Model, Transform3D};
use super::scene::Scene;

/// Bridges a spawned `Node`'s `Transform3D`+`Model` properties into the
/// existing renderer, which only walks `App::renderizable_instances`/
/// `App::game_models` today - see `properties.rs`'s own doc comment on
/// `Model`, this is exactly the "follow-up work" flagged there.
///
/// Expects `model_ref` to already be loaded - either via
/// `resources::register_model` (into `app.loaded_models`) or already resident
/// in `app.game_models` (the `data.ron` pipeline, or a previous call to this
/// fn for another instance of the same model) - and errors instead of
/// loading one itself; see that function's own doc comment for why loading is
/// meant to happen up front, not lazily here.
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
    let transform = *node.get_property::<Transform3D>().ok_or_else(|| format!("node '{id}' has no Transform3D property"))?;
    let model_ref = node.get_property::<Model>().ok_or_else(|| format!("node '{id}' has no Model property"))?.model_ref.clone();

    let game_object = GameObject {
        id: id.to_owned(),
        model: model_ref.clone(),
        transform: GameObjectTransform {
            position: transform.position,
            rotation: transform.rotation,
            scale: transform.scale,
        },
        children: vec![],
        metadata: MetaData { physics: None, cameras: None, lighting: None },
    };

    // Reuse the mesh already loaded - either previously instanced
    // (app.game_models) or preloaded but not yet instanced
    // (app.loaded_models, see resources::register_model) - never loads one
    // itself; a model_ref with neither is a "you forgot to preload this" bug.
    let loaded_model = match app.game_models.remove(&model_ref) {
        Some(existing) => existing.model,
        None => app.loaded_models.remove(&model_ref)
            .ok_or_else(|| format!("model '{model_ref}' isn't loaded - call resources::register_model first"))?,
    };

    scene.content.renderizable_instances.insert(id.to_owned(), InstanceData {
        renderizable_transform: game_object.transform,
        instance: game_object,
        model_ref: model_ref.clone(),
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

    app.game_models.insert(model_ref, ModelDataInstance {
        model: loaded_model,
        instance_buffer,
        instance_count,
    });

    Ok(())
}
