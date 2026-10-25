use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};

use wgpu::RenderPassDepthStencilAttachment;

use crate::app::App;
use crate::primitive::manual_vertex::ManualVertex;
use crate::rendering::frame::{FramePlan, FrameStep};
use crate::rendering::instance_management::ModelDataInstance;
use crate::rendering::render_sets::RenderSets;
use crate::rendering::shader_library::{Blend, Cull, Kind, Overlay, PipelineKey, Placement, BLIT, STANDARD};
use crate::scene_manager::node::properties::Material;

/// A mesh's name without the `#primN` an object with several materials
/// loads its later ones with (see Model::meshes_named_mut).
fn mesh_base_name(key: &str) -> &str {
    key.split("#prim").next().unwrap_or(key)
}

/// What a scene pass has to do this frame - see App::pass_work.
#[derive(Default, Clone, Copy)]
struct PassWork {
    draws: bool,
    reads_depth: bool,
}

/// A model's instances, as runs of one material each (see
/// ModelDataInstance::material_ranges) - or, before the first frame's are
/// worked out, all of them with engine/standard.
fn material_runs(model: &ModelDataInstance) -> impl Iterator<Item = (&str, std::ops::Range<u32>)> {
    let before_first_frame = model.material_ranges.is_empty().then(|| (STANDARD, 0..model.instance_count));
    model.material_ranges.iter().map(|(material, instances)| (material.as_str(), instances.clone())).chain(before_first_frame)
}

/// What `hashing` writes into a fresh hasher, as one number.
fn hash_of(hashing: impl FnOnce(&mut DefaultHasher)) -> u64 {
    let mut hasher = DefaultHasher::new();
    hashing(&mut hasher);
    hasher.finish()
}

/// What `step` is - its kind and what it names - into `hasher`.
fn hash_step(step: &FrameStep, hasher: &mut DefaultHasher) {
    std::mem::discriminant(step).hash(hasher);
    match step {
        FrameStep::Scene { pass } => pass.hash(hasher),
        FrameStep::Target { name, clear } => (name, clear).hash(hasher),
        FrameStep::Lines { set } | FrameStep::Text { set } => set.hash(hasher),
        FrameStep::Ui { layer } => (*layer as u8).hash(hasher),
        FrameStep::Fullscreen { shader } => shader.hash(hasher),
        FrameStep::Custom { name, .. } => name.hash(hasher),
        FrameStep::CopyDepth | FrameStep::Particles | FrameStep::Nodes2D | FrameStep::EndTarget => {}
    }
}

/// `material`'s shader, its values (by name, in order - each number exactly)
/// and its meshes' materials, into `hasher`.
fn hash_material(material: &Material, hasher: &mut DefaultHasher) {
    material.shader.hash(hasher);
    for (name, value) in &material.params {
        name.hash(hasher);
        for number in &value.0 {
            number.to_bits().hash(hasher);
        }
    }
    for (mesh, mesh_material) in &material.meshes {
        mesh.hash(hasher);
        hash_material(mesh_material, hasher);
    }
}

pub(crate) fn color_attachment(view: &wgpu::TextureView, load: wgpu::LoadOp<wgpu::Color>) -> Option<wgpu::RenderPassColorAttachment> {
    Some(wgpu::RenderPassColorAttachment {
        view,
        resolve_target: None,
        ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
    })
}

fn depth_attachment(view: &wgpu::TextureView, load: wgpu::LoadOp<f32>) -> Option<RenderPassDepthStencilAttachment> {
    Some(RenderPassDepthStencilAttachment {
        view,
        depth_ops: Some(wgpu::Operations { load, store: wgpu::StoreOp::Store }),
        stencil_ops: None,
    })
}

impl App {
    // Every model with an instance to draw, by its key.
    fn drawn_models(&self) -> impl Iterator<Item = (&String, &ModelDataInstance)> {
        self.assets.game_models.iter().filter(|(_, model)| model.instance_count > 0)
    }

    // Renders into BlurRender's scene_color (an offscreen copy of the swapchain),
    // not the swapchain view itself - the UI pass needs something already-rendered
    // it can sample from for background_blur, and you can't read a texture that's
    // also the render target currently being written to. BlurRender::render blits
    // scene_color onto the real swapchain view right before the UI pass runs, so
    // this indirection is invisible for every scene not using background_blur.
    fn render_opaque_pass(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let view = &self.renderer.blur.scene_color.view;
        // No real camera to render from (see SceneCameras::has_active_camera)
        // - skip every model/skybox draw below, but still clear to the
        // environment's clear color rather than hardcoding black.
        // App::sync_no_camera_message puts the actual "Add a camera to the
        // scene" text up via the separate UI pass, which still runs normally
        // on top of whatever this clears to (and suppresses itself while a
        // scene loads - see its own comment).
        let has_camera = self.scene_manager.cameras().is_some_and(|c| c.has_active_camera());
        let clear_color = self.current_environment().clear_color;
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Opaque Render Pass"),
            color_attachments: &[color_attachment(view, wgpu::LoadOp::Clear(clear_color))],
            // Reversed-Z: clear to 0.0 ("infinitely far") instead of 1.0.
            depth_stencil_attachment: depth_attachment(&self.renderer.depth_render.texture.view, wgpu::LoadOp::Clear(0.0)),
            occlusion_query_set: None,
            timestamp_writes: None,
        });

        if !has_camera {
            return;
        }

        if let Some(skybox) = self.current_environment().skybox.as_ref() {
            skybox.render(&mut render_pass, &self.renderer.camera_resources.bind_group);
        }

        // A shader that reads this pass's depth (the ocean) draws in a later
        // pass instead, after the frame plan's CopyDepth - it can't read the
        // depth it's writing.
        self.draw_materials(&mut render_pass, "opaque");
    }

    // Same scene_color target as render_opaque_pass above, and for the same reason.
    fn render_transparent_pass(&mut self, encoder: &mut wgpu::CommandEncoder) {
        // render_opaque_pass already cleared to black and drew nothing - see
        // its own comment on has_active_camera - nothing for this pass to add.
        if !self.scene_manager.cameras().is_some_and(|c| c.has_active_camera()) {
            return;
        }

        let view = &self.renderer.blur.scene_color.view;
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Transparent Render Pass"),
            color_attachments: &[color_attachment(view, wgpu::LoadOp::Load)],
            depth_stencil_attachment: depth_attachment(&self.renderer.depth_render.texture.view, wgpu::LoadOp::Load),
            occlusion_query_set: None,
            timestamp_writes: None,
        });

        self.draw_materials(&mut render_pass, "transparent");
    }

    /// A scene pass of the game's own (any name but "opaque" and
    /// "transparent" - a "water" pass, say): over the world drawn
    /// so far, against its depth - every model whose material's shader draws
    /// in `pass`.
    fn render_scene_pass(&mut self, encoder: &mut wgpu::CommandEncoder, pass: &str) {
        if !self.scene_manager.cameras().is_some_and(|c| c.has_active_camera()) {
            return;
        }
        let view = &self.renderer.blur.scene_color.view;
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Scene pass"),
            color_attachments: &[color_attachment(view, wgpu::LoadOp::Load)],
            depth_stencil_attachment: depth_attachment(&self.renderer.depth_render.texture.view, wgpu::LoadOp::Load),
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        self.draw_materials(&mut render_pass, pass);
    }

    /// Where each material in use is drawn this frame
    /// (`App::material_placements`), for `plan`'s scene passes - building
    /// any shader that isn't yet (see ShaderLibrary::placement) - and the
    /// plan's Fullscreen steps' materials made ready: the ones that can be
    /// used (the others draw engine/blit).
    ///
    /// Done again only when something it reads changed since the last time
    /// (see `materials_fingerprint`) - otherwise what it set up then is still
    /// right, and kept.
    fn place_materials(&mut self, plan: &FramePlan) -> HashSet<String> {
        let fingerprint = self.materials_fingerprint(plan);
        if let Some((last, ready)) = &self.renderer.placed {
            if *last == fingerprint {
                return ready.clone();
            }
        }
        let ready = self.set_up_materials(plan);
        // Building shaders just now counted as a change: what's built after
        // it, not before, is what the next frame compares against.
        let fingerprint = self.materials_fingerprint(plan);
        self.renderer.placed = Some((fingerprint, ready.clone()));
        ready
    }

    /// Everything `place_materials` reads, as one number: the frame plan's
    /// steps, every node's material (and the model it's drawn with), every
    /// drawn model's materials and meshes, the UI's and the line and text
    /// sets' materials, and the shader library's changes. Anything added to
    /// what `set_up_materials` reads has to be added here too - or a change
    /// to it won't show until something else changes.
    ///
    /// The same whatever order maps hand their entries out in: each entry is
    /// hashed on its own, and the hashes added up.
    fn materials_fingerprint(&self, plan: &FramePlan) -> u64 {
        let mut total = hash_of(|hasher| {
            for step in &plan.steps {
                hash_step(step, hasher);
            }
            self.renderer.shaders.changes().hash(hasher);
        });
        if let Some(content) = self.scene_manager.content() {
            for node in content.nodes.iter() {
                let Some(material) = node.get_property::<Material>() else { continue };
                total = total.wrapping_add(hash_of(|hasher| {
                    node.id.hash(hasher);
                    hash_material(material, hasher);
                    content.renderizable_instances.get(&node.id).map(|instance| &instance.model_ref).hash(hasher);
                }));
            }
        }
        for (model_ref, model) in &self.assets.game_models {
            let meshes = model.model.mesh_lists.iter()
                .flat_map(|(list, meshes)| meshes.iter().map(move |(key, mesh)| hash_of(|hasher| (list, key, mesh.material).hash(hasher))))
                .fold(0u64, u64::wrapping_add);
            total = total.wrapping_add(hash_of(|hasher| {
                (model_ref, model.stamp, meshes).hash(hasher);
                for (material, _) in &model.material_ranges {
                    material.hash(hasher);
                }
            }));
        }
        for (key, material) in &self.ui.ui_rendering.materials {
            total = total.wrapping_add(hash_of(|hasher| {
                ("ui", key).hash(hasher);
                hash_material(material, hasher);
            }));
        }
        for (name, lines) in self.renderer.render_sets.all_lines() {
            total = total.wrapping_add(hash_of(|hasher| {
                ("lines", name).hash(hasher);
                hash_material(&lines.material, hasher);
            }));
        }
        for (name, text) in self.renderer.render_sets.all_text() {
            total = total.wrapping_add(hash_of(|hasher| {
                ("text", name).hash(hasher);
                hash_material(&text.material, hasher);
            }));
        }
        total
    }

    /// Sets up every material in use - see `place_materials`.
    fn set_up_materials(&mut self, plan: &FramePlan) -> HashSet<String> {
        let passes: HashSet<String> = plan.steps.iter()
            .filter_map(|step| match step {
                FrameStep::Scene { pass } => Some(pass.clone()),
                _ => None,
            })
            .collect();
        // Every node's material instance - its shader and its own values -
        // and its model's mesh materials.
        for model in self.assets.game_models.values_mut() {
            model.mesh_materials.clear();
        }
        if let Some(content) = self.scene_manager.content() {
            for node in content.nodes.iter() {
                let Some(material) = node.get_property::<Material>() else { continue };
                self.renderer.shaders.set_instance(&material.key(&node.id), &material.shader, &material.params);
                if material.meshes.is_empty() {
                    continue;
                }
                let Some(model) = content.renderizable_instances.get(&node.id).and_then(|instance| self.assets.game_models.get_mut(&instance.model_ref)) else { continue };
                for (mesh, mesh_material) in &material.meshes {
                    let key = mesh_material.mesh_key(&node.id, mesh);
                    self.renderer.shaders.set_instance(&key, &mesh_material.shader, &mesh_material.params);
                    model.mesh_materials.insert(mesh.clone(), key);
                }
            }
        }
        let mut materials: HashSet<String> = self.assets.game_models.values()
            .flat_map(|model| model.material_ranges.iter().map(|(material, _)| material.clone()).chain(model.mesh_materials.values().cloned()))
            .collect();
        materials.insert(STANDARD.to_owned());
        self.renderer.material_placements.clear();
        for material in &materials {
            let placement = self.renderer.shaders.placement(material, &passes);
            self.renderer.material_placements.insert(material.clone(), placement);
        }
        // The Fullscreen steps' materials - and engine/blit, what they fall
        // back to.
        let mut fullscreen: HashSet<String> = plan.steps.iter()
            .filter_map(|step| match step {
                FrameStep::Fullscreen { shader } => Some(shader.clone()),
                _ => None,
            })
            .collect();
        fullscreen.insert(BLIT.to_owned());
        materials.extend(fullscreen.iter().cloned());
        // The 2D things' materials - the UI's (see UiRendering::materials),
        // each line and text set's - and the flashing magenta ones they fall
        // back to.
        let mut overlays: Vec<(String, Kind)> = Vec::new();
        for (key, material) in &self.ui.ui_rendering.materials {
            self.renderer.shaders.set_instance(key, &material.shader, &material.params);
            overlays.push((key.clone(), Kind::Ui));
        }
        for (name, lines) in self.renderer.render_sets.all_lines() {
            let key = RenderSets::lines_key(name, lines);
            self.renderer.shaders.set_instance(&key, &lines.material.shader, &lines.material.params);
            overlays.push((key, Kind::Lines));
        }
        for (name, text) in self.renderer.render_sets.all_text() {
            let key = RenderSets::text_key(name, text);
            self.renderer.shaders.set_instance(&key, &text.material.shader, &text.material.params);
            overlays.push((key, Kind::Text));
        }
        materials.extend(overlays.iter().map(|(key, _)| key.clone()));
        materials.extend(["engine/ui_error", "engine/sdf_text_error", "engine/thick_lines_error"].map(str::to_owned));
        self.renderer.shaders.retain_instances(&materials);
        let ready: HashSet<String> = fullscreen.into_iter().filter(|key| self.renderer.shaders.prepare_fullscreen(key)).collect();
        self.renderer.overlay_materials.clear();
        for (key, kind) in overlays {
            if let Some(drawn) = self.renderer.shaders.prepare_overlay(&key, kind) {
                self.renderer.overlay_materials.insert(key, drawn);
            }
        }
        // The bind groups 0 of materials with their own (params, textures) -
        // one for each mesh texture they're drawn with.
        for (model_ref, model) in &self.assets.game_models {
            for meshes in model.model.mesh_lists.values() {
                for (mesh_key, mesh) in meshes {
                    let own = model.mesh_materials.get(mesh_base_name(mesh_key));
                    let keys: Vec<&String> = match own {
                        Some(key) => vec![key],
                        None => model.material_ranges.iter().map(|(key, _)| key).collect(),
                    };
                    for key in keys {
                        let placed = self.renderer.material_placements.get(key).is_some_and(|placement| *placement != Placement::Error);
                        if placed && self.renderer.shaders.has_group0(key) {
                            self.renderer.shaders.prepare_group0(key, model_ref, mesh.material, &model.model.materials[mesh.material].diffuse_texture);
                        }
                    }
                }
            }
        }
        ready
    }

    /// Runs the Fullscreen material `key` (see ShaderLibrary::prepare_fullscreen)
    /// over all of `view`.
    fn draw_fullscreen(&self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, key: &str) {
        let Some((pipeline, bind_group)) = self.renderer.shaders.fullscreen(key) else { return };
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Fullscreen pass"),
            color_attachments: &[color_attachment(view, wgpu::LoadOp::Load)],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        render_pass.set_pipeline(pipeline);
        render_pass.set_bind_group(0, bind_group, &[]);
        render_pass.set_bind_group(1, &self.renderer.light.rendering_data.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }

    /// Draws, into `render_pass`, every model's meshes whose material is
    /// placed in the scene pass `pass` (see `place_materials`): like
    /// engine/standard - solid meshes in "opaque", see-through ones in
    /// "transparent" -, all of them in the pass its shader names, or (with
    /// engine/error) in "opaque". A mesh's material is its model's own for it
    /// (`mesh_materials`), else its instances'.
    fn draw_materials<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>, pass: &str) {
        // The same for every mesh this pass - set once. (wgpu keeps them bound
        // across pipeline changes, rebinding them itself where a pipeline's
        // layout needs it.)
        render_pass.set_bind_group(1, &self.renderer.camera_resources.bind_group, &[]);
        render_pass.set_bind_group(3, &self.renderer.light.rendering_data.bind_group, &[]);
        // What's bound now - set again only when a mesh needs another.
        let mut bound_pipeline: Option<&wgpu::RenderPipeline> = None;
        let mut bound_group0: Option<&wgpu::BindGroup> = None;
        for (model_ref, model_data) in self.drawn_models() {
            render_pass.set_vertex_buffer(1, model_data.instance_buffer.slice(..));
            // Every mesh of the model draws from these - once per model.
            render_pass.set_vertex_buffer(0, model_data.model.vertex_buffer.slice(..));
            render_pass.set_index_buffer(model_data.model.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            for (material, instances) in material_runs(model_data) {
                for list in ["opaque", "transparent"] {
                    let Some(meshes) = model_data.model.mesh_lists.get(list) else { continue };
                    for (mesh_key, mesh) in meshes {
                        let key = model_data.mesh_materials.get(mesh_base_name(mesh_key)).map_or(material, String::as_str);
                        let Some(placement) = self.renderer.material_placements.get(key) else { continue };
                        let Some((single, double, shell)) = self.mesh_pipelines(key, placement, pass, list) else { continue };
                        let Some(single) = self.renderer.shaders.pipeline(key, placement, single) else { continue };
                        let double = double.and_then(|pipeline| self.renderer.shaders.pipeline(key, placement, pipeline));
                        let shell = shell.and_then(|pipeline| self.renderer.shaders.pipeline(key, placement, pipeline));
                        // Its own bind group 0 (params, textures), or the
                        // mesh's (just its texture).
                        let group0 = if *placement != Placement::Error && self.renderer.shaders.has_group0(key) {
                            let Some(group0) = self.renderer.shaders.group0(key, model_ref, mesh.material) else { continue };
                            group0
                        } else {
                            &model_data.model.materials[mesh.material].bind_group
                        };
                        if !bound_group0.is_some_and(|bound| std::ptr::eq(bound, group0)) {
                            render_pass.set_bind_group(0, group0, &[]);
                            bound_group0 = Some(group0);
                        }
                        render_pass.set_bind_group(2, &mesh.transform_bind_group, &[]);
                        let mut draw = |pipeline: &'a wgpu::RenderPipeline| {
                            if !bound_pipeline.is_some_and(|bound| std::ptr::eq(bound, pipeline)) {
                                render_pass.set_pipeline(pipeline);
                                bound_pipeline = Some(pipeline);
                            }
                            render_pass.draw_indexed(mesh.first_index..mesh.first_index + mesh.num_elements, mesh.base_vertex, instances.clone());
                        };
                        match (mesh.double_sided, shell, double) {
                            // A see-through shell: its far side, then its near.
                            (true, Some(shell), _) => {
                                draw(shell);
                                draw(single);
                            }
                            (true, None, Some(double)) => draw(double),
                            _ => draw(single),
                        }
                    }
                }
            }
        }
    }

    /// What the scene pass `pass` has this frame: whether `draw_materials`
    /// would draw anything in it, and whether anything it draws reads the
    /// depth so far (see ShaderLibrary::reads_depth) - the same walk, stopping
    /// once both are known. A pass with nothing in it is skipped (see
    /// run_frame_plan): starting and ending a render pass costs something on
    /// every backend even when empty. And the depth is only copied for a pass
    /// that reads it.
    fn pass_work(&self, pass: &str) -> PassWork {
        let mut work = PassWork::default();
        if !self.scene_manager.cameras().is_some_and(|c| c.has_active_camera()) {
            return work;
        }
        for (_, model_data) in self.drawn_models() {
            for (material, instances) in material_runs(model_data) {
                if instances.is_empty() {
                    continue;
                }
                for list in ["opaque", "transparent"] {
                    let Some(meshes) = model_data.model.mesh_lists.get(list) else { continue };
                    for mesh_key in meshes.keys() {
                        let key = model_data.mesh_materials.get(mesh_base_name(mesh_key)).map_or(material, String::as_str);
                        let Some(placement) = self.renderer.material_placements.get(key) else { continue };
                        if self.mesh_pipelines(key, placement, pass, list).is_some() {
                            work.draws = true;
                            work.reads_depth |= *placement != Placement::Error && self.renderer.shaders.reads_depth(key);
                            if work.reads_depth {
                                return work;
                            }
                        }
                    }
                }
            }
        }
        work
    }

    /// Which pipelines the material `key`, placed like `placement`, draws a
    /// mesh of the list `list` with in the pass `pass` - single-sided,
    /// double-sided, and a double-sided see-through shell's far side - or
    /// None when it doesn't draw it here.
    fn mesh_pipelines(&self, key: &str, placement: &Placement, pass: &str, list: &str) -> Option<(PipelineKey, Option<PipelineKey>, Option<PipelineKey>)> {
        let pipeline = |cull, depth_write, blend| PipelineKey { cull, depth_write, blend };
        match (placement, pass, list) {
            (Placement::ByMesh, "opaque", "opaque") => Some((pipeline(Cull::Back, true, Blend::Alpha), Some(pipeline(Cull::None, true, Blend::Alpha)), None)),
            (Placement::ByMesh, "transparent", "transparent") => Some((pipeline(Cull::Back, false, Blend::Alpha), None, Some(pipeline(Cull::Front, false, Blend::Alpha)))),
            (Placement::Pass(own), _, _) if own == pass => {
                let settings = self.renderer.shaders.settings(key)?;
                Some((pipeline(settings.cull, settings.depth_write, settings.blend()), Some(pipeline(Cull::None, settings.depth_write, settings.blend())), None))
            }
            (Placement::Error, "opaque", _) => Some((pipeline(Cull::Back, true, Blend::None), Some(pipeline(Cull::None, true, Blend::None)), None)),
            _ => None,
        }
    }

    // Particles and trails (see ParticleRenderer) - after everything solid
    // and see-through, so they blend over all of it. The update (compute)
    // pass runs first, then the scene's depth - everything drawn so far - is
    // snapshotted for their soft fading (any pass before reading the depth is
    // done with its own copy by then).
    fn render_particle_pass(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if !self.renderer.particles.has_work() || !self.scene_manager.cameras().is_some_and(|c| c.has_active_camera()) {
            return;
        }
        self.renderer.particles.simulate(encoder);
        self.renderer.depth_render.snapshot_depth(encoder);

        let view = &self.renderer.blur.scene_color.view;
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Particle Render Pass"),
            color_attachments: &[color_attachment(view, wgpu::LoadOp::Load)],
            depth_stencil_attachment: depth_attachment(&self.renderer.depth_render.texture.view, wgpu::LoadOp::Load),
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        self.renderer.particles.draw(&mut render_pass, &self.renderer.camera_resources.bind_group);
    }

    // Physics debug lines and UI/text share this pass since they're both drawn on top,
    // without a depth buffer, gated on there being UI geometry to draw at all.
    // `which` splits the main layer from the always-on-top one (see UiLayer).
    // `frame` is the texture behind `view`, when it's the screen: with an
    // always-on-top layer to draw (a modal), what's been drawn so far - world
    // and HUD - is copied and blurred first, so that layer's blur shows the
    // HUD behind it instead of painting the bare world over it.
    fn render_ui_pass(&mut self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, which: UiLayer, frame: Option<&wgpu::Texture>) {
        if self.ui.ui_rendering.num_indices == 0 && self.ui.image_draws.is_empty() {
            return;
        }
        // ui_rendering.uses_blur is only the on-top layer's by now (see
        // App::prepare_ui_content) - no blurred node there, nothing to copy.
        let has_on_top = self.ui.ui_rendering.num_indices > self.ui.ui_rendering.main_index_count;
        let on_top_blurs = has_on_top && self.ui.ui_rendering.uses_blur;
        let Some(frame) = frame.filter(|_| which != UiLayer::Main && on_top_blurs) else {
            self.draw_ui_layers(encoder, view, which, which != UiLayer::Main, false);
            return;
        };
        if which == UiLayer::All {
            self.draw_ui_layers(encoder, view, UiLayer::Main, true, false);
        }
        let captured = self.renderer.blur.blur_backdrop(encoder, frame);
        if !captured {
            // Can't copy the screen - the scene's blur, as before.
            self.renderer.blur.blur_scene(encoder);
        }
        self.draw_ui_layers(encoder, view, UiLayer::OnTop, which == UiLayer::OnTop, captured);
    }

    /// One render pass of the UI's `which` layer(s) - with the physics debug
    /// lines under it when `lines`, and the on-top layer's blur sampling
    /// BlurRender::backdrop when `on_top_backdrop`.
    fn draw_ui_layers(&mut self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, which: UiLayer, lines: bool, on_top_backdrop: bool) {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("UI Render Pass"),
            color_attachments: &[color_attachment(view, wgpu::LoadOp::Load)],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });

        render_pass.set_pipeline(&self.renderer.render_physics.render_pipeline);
        render_pass.set_bind_group(0, &self.renderer.render_physics.bind_group, &[]);
        render_pass.set_bind_group(1, &self.renderer.camera_resources.bind_group, &[]);

        if lines && !self.renderer.show_depth_map && (self.renderer.render_physics.visible || !self.renderer.world_lines.is_empty()) {
            self.render_physics_debug_lines(&mut render_pass);
        }

        // Main-pass quads, then main-pass text, THEN always_on_top's quads and
        // text (see UiRendering::main_index_count / TextRendering::
        // text_renderer_on_top's own doc comments) - text is a wholly separate
        // draw pass from these quads (glyphon, not the Ui shaders below), so
        // without this split every node's text ended up in one shared final
        // pass with no z-order relative to a quad drawn after it: a modal's
        // opaque background quad could correctly land above a panel behind it,
        // while that same panel's *text* still rendered on top of the modal
        // regardless, since all text - modal's and panel's alike - drew after
        // all quads unconditionally.
        let main_index_count = self.ui.ui_rendering.main_index_count;
        if which != UiLayer::OnTop && main_index_count > 0 {
            self.draw_ui_boxes(&mut render_pass, 0, main_index_count, false);
        }

        // One draw call per distinct image - see Ui::build_image_draws. Not
        // itself split into a main/on-top pair (nothing always_on_top uses
        // Image content today), so it renders as one batch here, same relative
        // position (after the main pass's quads, before the on-top pass) it
        // always has.
        if which != UiLayer::OnTop && !self.ui.image_draws.is_empty() {
            render_pass.set_vertex_buffer(0, self.ui.image_vertex_buffer.slice(..));
            render_pass.set_index_buffer(self.ui.image_index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            for draw in &self.ui.image_draws {
                let material = self.renderer.overlay_materials.get(&draw.material).and_then(|key| self.renderer.shaders.overlay(key, Overlay::UiImage));
                if let (Some(image), Some((pipeline, material))) = (self.assets.image(&draw.path), material) {
                    render_pass.set_pipeline(pipeline);
                    render_pass.set_bind_group(0, material, &[]);
                    render_pass.set_bind_group(1, &self.renderer.camera_resources.bind_group, &[]);
                    render_pass.set_bind_group(2, &image.bind_group, &[]);
                    render_pass.set_bind_group(3, &self.renderer.light.rendering_data.bind_group, &[]);
                    render_pass.draw_indexed(draw.indices.clone(), draw.base_vertex, 0..1);
                }
            }
        }

        // Render text (text renderer handles empty content gracefully)
        if which != UiLayer::OnTop {
            self.ui.text.text_renderer.render(&self.ui.text.text_atlas, &self.renderer.glyphon.viewport, &mut render_pass).unwrap();
        }

        if which != UiLayer::Main && self.ui.ui_rendering.num_indices > main_index_count {
            self.draw_ui_boxes(&mut render_pass, main_index_count, self.ui.ui_rendering.num_indices, on_top_backdrop);
        }

        if which != UiLayer::Main {
            self.ui.text.text_renderer_on_top.render(&self.ui.text.text_atlas, &self.renderer.glyphon.viewport, &mut render_pass).unwrap();
        }
    }

    /// Draws the UI's boxes `from..to` (indices), one draw call for each run
    /// of the same material (see UiRendering::runs). `backdrop`: blurred
    /// boxes sample BlurRender::backdrop instead of the bare world.
    fn draw_ui_boxes(&self, render_pass: &mut wgpu::RenderPass<'_>, from: u32, to: u32, backdrop: bool) {
        render_pass.set_vertex_buffer(0, self.ui.ui_rendering.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.ui.ui_rendering.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        render_pass.set_bind_group(1, &self.renderer.camera_resources.bind_group, &[]);
        let blur = if backdrop { &self.ui.blur_bind_group_on_top } else { &self.ui.blur_bind_group };
        render_pass.set_bind_group(2, blur, &[]);
        render_pass.set_bind_group(3, &self.renderer.light.rendering_data.bind_group, &[]);
        for (range, key) in self.ui.ui_rendering.runs_between(from, to) {
            let Some((pipeline, material)) = self.renderer.overlay_materials.get(key).and_then(|key| self.renderer.shaders.overlay(key, Overlay::UiBox)) else { continue };
            render_pass.set_pipeline(pipeline);
            render_pass.set_bind_group(0, material, &[]);
            render_pass.draw_indexed(range, 0, 0..1);
        }
    }

    // Debug lines come from the physics thread in absolute world coordinates,
    // so make them camera-relative here to match camera.view_proj. The
    // physics thread's only while F5 is on; the game's own (App::world_lines)
    // always.
    fn render_physics_debug_lines<'rp>(&mut self, render_pass: &mut wgpu::RenderPass<'rp>) {
        let camera_position = self.scene_manager.cameras().map(|c| c.active().camera.position()).unwrap_or_else(nalgebra::Point3::origin);
        let physics_lines: &[[ManualVertex; 2]] = if self.renderer.render_physics.visible { &self.renderer.render_physics.renderizable_lines } else { &[] };
        let vertices: Vec<ManualVertex> = physics_lines.iter().chain(&self.renderer.world_lines)
            .flat_map(|line| line.to_vec())
            .map(|mut vertex| {
                vertex.position[0] -= camera_position.x;
                vertex.position[1] -= camera_position.y;
                vertex.position[2] -= camera_position.z;
                vertex
            })
            .collect();

        if vertices.is_empty() {
            return;
        }

        // Kept from frame to frame - a new one only when the lines outgrow it.
        let bytes: &[u8] = bytemuck::cast_slice(&vertices);
        let buffer = &mut self.renderer.render_physics.vertex_buffer;
        crate::rendering::ui::ui::grow_buffer(buffer, &self.renderer.device, bytes.len() as u64, wgpu::BufferUsages::VERTEX);
        self.renderer.queue.write_buffer(buffer, 0, bytes);

        // Each line is two vertices, one after the other - drawn in order,
        // no index buffer needed.
        render_pass.set_vertex_buffer(0, self.renderer.render_physics.vertex_buffer.slice(..bytes.len() as u64));
        render_pass.draw(0..vertices.len() as u32, 0..1);
    }

    /// Draws this frame's plan (`App::frame_plan` - the scene's) - see
    /// engine::rendering::frame.
    pub(crate) fn render_scene_passes(&mut self, encoder: &mut wgpu::CommandEncoder, screen_texture: &wgpu::Texture, view: &wgpu::TextureView) {
        let mut plan = std::mem::take(&mut self.renderer.frame_plan);
        self.run_frame_plan(&mut plan, encoder, screen_texture, view);
        self.renderer.frame_plan = plan;
    }

    /// Runs `plan`'s steps in order, onto `screen` (or the target a step
    /// switched to). A step naming something that doesn't exist is skipped,
    /// and said once.
    fn run_frame_plan(&mut self, plan: &mut FramePlan, encoder: &mut wgpu::CommandEncoder, screen_texture: &wgpu::Texture, screen: &wgpu::TextureView) {
        // The image the steps draw into - the screen, or a named target's (a
        // fresh view, so it can be borrowed while `self` is in use).
        let mut target: Option<wgpu::TextureView> = None;
        let mut missing = Vec::new();
        let fullscreen_ready = self.place_materials(plan);
        // The scene blurred for the UI's frosted panels - once, before the
        // first UI step, once the world's drawn. Only when something reads
        // it: a blurred main-layer node, or a Fullscreen shader reading
        // "blurred" (last frame's, so it's kept up every frame).
        let fullscreen_reads_blur = plan.steps.iter().any(|step| matches!(step, FrameStep::Fullscreen { shader }
            if fullscreen_ready.contains(shader) && self.renderer.shaders.settings(shader).is_some_and(|settings| settings.inputs.iter().any(|input| input == "blurred"))));
        let mut blurred = false;
        // Which scene passes have something to draw this frame - the others
        // are skipped, and so is a depth copy no pass after it needs. "opaque"
        // always runs: it clears the frame and draws the sky.
        let busy: Vec<PassWork> = plan.steps.iter().map(|step| match step {
            FrameStep::Scene { pass } if pass != "opaque" => self.pass_work(pass),
            _ => PassWork::default(),
        }).collect();
        for (index, step) in plan.steps.iter().enumerate() {
            let view = target.as_ref().unwrap_or(screen);
            match step {
                FrameStep::Scene { pass } => match pass.as_str() {
                    "opaque" => self.render_opaque_pass(encoder),
                    _ if !busy[index].draws => {}
                    "transparent" => self.render_transparent_pass(encoder),
                    _ => self.render_scene_pass(encoder, pass),
                },
                // Before the passes that read it - copy_texture_to_texture
                // isn't valid mid-pass. Only when a scene pass after it draws
                // something that reads it (`reads: [Depth]`) - a full-screen
                // copy isn't free.
                FrameStep::CopyDepth => {
                    if busy[index + 1..].iter().any(|work| work.draws && work.reads_depth) {
                        self.renderer.depth_render.snapshot_depth(encoder);
                    }
                }
                FrameStep::Particles => self.render_particle_pass(encoder),
                FrameStep::Nodes2D => self.renderer.shapes_2d.draw(encoder, view),
                FrameStep::Target { name, clear } => match self.renderer.targets.get(name) {
                    Some(image) => {
                        let view = image.texture.create_view(&wgpu::TextureViewDescriptor::default());
                        if *clear {
                            self.clear_target(encoder, &view);
                        }
                        target = Some(view);
                    }
                    None => missing.push(format!("target \"{name}\" doesn't exist (see Renderer::add_target)")),
                },
                FrameStep::EndTarget => target = None,
                FrameStep::Lines { set } => {
                    if !self.draw_line_set(encoder, view, set) {
                        missing.push(format!("line set \"{set}\" doesn't exist"));
                    }
                }
                FrameStep::Text { set } => {
                    if !self.draw_text_set(encoder, view, set) {
                        missing.push(format!("text set \"{set}\" doesn't exist"));
                    }
                }
                FrameStep::Ui { layer } => {
                    // The always-on-top layer blurs the screen as drawn so
                    // far - not while drawing into another target, where it
                    // falls back to the scene's blur.
                    let frame = if target.is_none() { Some(screen_texture) } else { None };
                    let rendering = &self.ui.ui_rendering;
                    let needs_blur = fullscreen_reads_blur
                        || (*layer != UiLayer::OnTop && rendering.main_uses_blur)
                        || (*layer != UiLayer::Main && rendering.uses_blur && frame.is_none());
                    if needs_blur && !blurred {
                        self.renderer.blur.blur_scene(encoder);
                        blurred = true;
                    }
                    self.render_ui_pass(encoder, view, *layer, frame);
                }
                // A shader over the whole target - or, if it can't be used,
                // the scene plain (engine/blit; why is said once).
                FrameStep::Fullscreen { shader } => {
                    let key = if fullscreen_ready.contains(shader) { shader.as_str() } else { BLIT };
                    self.draw_fullscreen(encoder, view, key);
                }
                FrameStep::Custom { run, .. } => run(self, encoder, view),
            }
        }
        for what in missing {
            plan.report(what);
        }
    }

    /// Draws the line set `set` into `view`, over what's there - false if
    /// there's no such set. Nothing when it's empty.
    fn draw_line_set(&mut self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, set: &str) -> bool {
        let camera_position = self.scene_manager.cameras().map(|c| c.active().camera.position()).unwrap_or_else(nalgebra::Point3::origin);
        let screen = [self.window.pixel_size.width as f32, self.window.pixel_size.height as f32];
        let Some(lines) = self.renderer.render_sets.get_lines_mut(set) else { return false };
        if lines.is_empty() {
            return true;
        }
        let key = RenderSets::lines_key(set, lines);
        let Some(material) = self.renderer.overlay_materials.get(&key).and_then(|key| self.renderer.shaders.overlay(key, Overlay::Lines)) else { return true };
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Line set pass"),
            color_attachments: &[color_attachment(view, wgpu::LoadOp::Load)],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        lines.draw(
            &self.renderer.device,
            &self.renderer.queue,
            &mut render_pass,
            material,
            &self.renderer.camera_resources.bind_group,
            &self.renderer.light.rendering_data.bind_group,
            [camera_position.x, camera_position.y, camera_position.z],
            screen,
        );
        true
    }

    /// Draws the SDF text set `set` into `view`, over what's there - false
    /// if there's no such set. Nothing when it's empty.
    fn draw_text_set(&mut self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, set: &str) -> bool {
        let camera_position = self.scene_manager.cameras().map(|c| c.active().camera.position()).unwrap_or_else(nalgebra::Point3::origin);
        let screen = [self.window.pixel_size.width as f32, self.window.pixel_size.height as f32];
        let Some(text) = self.renderer.render_sets.get_text_mut(set) else { return false };
        if text.is_empty() {
            return true;
        }
        let key = RenderSets::text_key(set, text);
        let Some(material) = self.renderer.overlay_materials.get(&key).and_then(|key| self.renderer.shaders.overlay(key, Overlay::Text)) else { return true };
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Text set pass"),
            color_attachments: &[color_attachment(view, wgpu::LoadOp::Load)],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        text.draw(
            &self.renderer.device,
            &self.renderer.queue,
            &mut render_pass,
            material,
            &self.renderer.camera_resources.bind_group,
            &self.renderer.light.rendering_data.bind_group,
            [camera_position.x, camera_position.y, camera_position.z],
            screen,
        );
        true
    }

    /// Clears one of the game's targets (see Renderer::add_target) to
    /// transparent, before this frame's steps draw into it.
    fn clear_target(&self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Target clear"),
            color_attachments: &[color_attachment(view, wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT))],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });
    }
}

/// Which slice of the UI a step draws: all of it, the main layer (every
/// node but the always-on-top ones), or the always-on-top layer (with the
/// physics debug lines) - so a frame plan can put something between them,
/// like the play scene's G-force effects over its HUD but under its menus.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UiLayer {
    All,
    Main,
    OnTop,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn material_hash(material: &Material) -> u64 {
        hash_of(|hasher| hash_material(material, hasher))
    }

    #[test]
    fn a_material_hashes_differently_once_anything_in_it_changes() {
        let base = Material::shader("hologram").param("tint", [1.0, 0.2, 0.2, 0.8]);
        assert_eq!(material_hash(&base), material_hash(&base.clone()));
        assert_ne!(material_hash(&base), material_hash(&base.clone().param("tint", [1.0, 0.2, 0.2, 0.9])), "a value");
        assert_ne!(material_hash(&base), material_hash(&base.clone().param("speed", 2.0)), "a value added");
        assert_ne!(material_hash(&base), material_hash(&Material::shader("glow").param("tint", [1.0, 0.2, 0.2, 0.8])), "the shader");
        assert_ne!(material_hash(&base), material_hash(&base.clone().mesh("Canopy", Material::shader("glass"))), "a mesh's material");
    }

    #[test]
    fn a_plan_step_hashes_by_what_it_is_and_names() {
        let step_hash = |step: &FrameStep| hash_of(|hasher| hash_step(step, hasher));
        let water = FrameStep::Scene { pass: "water".into() };
        assert_eq!(step_hash(&water), step_hash(&FrameStep::Scene { pass: "water".into() }));
        assert_ne!(step_hash(&water), step_hash(&FrameStep::Scene { pass: "lava".into() }));
        assert_ne!(step_hash(&FrameStep::Particles), step_hash(&FrameStep::Nodes2D));
        assert_ne!(step_hash(&FrameStep::Target { name: "hud".into(), clear: true }), step_hash(&FrameStep::Target { name: "hud".into(), clear: false }));
    }
}
