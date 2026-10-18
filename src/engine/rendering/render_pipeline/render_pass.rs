use std::collections::HashSet;

use wgpu::RenderPassDepthStencilAttachment;

use crate::app::App;
use crate::engine::primitive::manual_vertex::ManualVertex;
use crate::engine::rendering::frame::{FramePlan, FrameStep};
use crate::engine::rendering::render_sets::RenderSets;
use crate::engine::rendering::shader_library::{Blend, Cull, Kind, Overlay, PipelineKey, Placement, BLIT, STANDARD};
use crate::engine::scene_manager::properties::Material;

/// A mesh's name without the `#primN` an object with several materials
/// loads its later ones with (see Model::meshes_named_mut).
fn mesh_base_name(key: &str) -> &str {
    key.split("#prim").next().unwrap_or(key)
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
    // Distinct model refs currently in use, optionally skipping one instance key
    // (e.g. "sun", which has no drawable model of its own).
    fn distinct_model_refs(&self, exclude_key: Option<&str>) -> HashSet<String> {
        let Some(content) = self.scene_manager.content() else { return HashSet::new() };
        content.renderizable_instances.iter()
            .filter(|(key, _)| exclude_key != Some(key.as_str()))
            .map(|(_, renderizable)| renderizable.model_ref.clone())
            .collect()
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
        // - skip every model/skybox draw below, but still just clear to
        // clear_color rather than hardcoding black here: that value is
        // already correct for every phase this can happen in (the active
        // scene's own SceneEnvironment, or - only before any scene has ever
        // reset - run_splash_screen's own configured background, see
        // self.clear_color's own doc comment) - hardcoding black would
        // silently override any of those instead of leaving them alone.
        // App::sync_no_camera_message puts the actual "Add a camera to the
        // scene" text up via the separate UI pass, which still runs normally
        // on top of whatever this clears to (and suppresses itself during
        // splash/loading - see its own comment).
        let has_camera = self.scene_manager.cameras().is_some_and(|c| c.has_active_camera());
        let clear_color = self.scene_manager.environment().map_or(self.clear_color, |e| e.clear_color);
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

        let skybox = self.scene_manager.environment().and_then(|e| e.skybox.as_ref()).or(self.skybox.as_ref());
        if let Some(skybox) = skybox {
            skybox.render(&mut render_pass, &self.camera_resources.bind_group);
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
    /// "transparent" - the play scene's "water", say): over the world drawn
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
    fn place_materials(&mut self, plan: &FramePlan) -> HashSet<String> {
        let passes: HashSet<String> = plan.steps.iter()
            .filter_map(|step| match step {
                FrameStep::Scene { pass } => Some(pass.clone()),
                _ => None,
            })
            .collect();
        // Every node's material instance - its shader and its own values -
        // and its model's mesh materials.
        for model in self.game_models.values_mut() {
            model.mesh_materials.clear();
        }
        if let Some(content) = self.scene_manager.content() {
            for node in content.nodes.iter() {
                let Some(material) = node.get_property::<Material>() else { continue };
                self.shaders.set_instance(&material.key(&node.id), &material.shader, &material.params);
                if material.meshes.is_empty() {
                    continue;
                }
                let Some(model) = content.renderizable_instances.get(&node.id).and_then(|instance| self.game_models.get_mut(&instance.model_ref)) else { continue };
                for (mesh, mesh_material) in &material.meshes {
                    let key = mesh_material.mesh_key(&node.id, mesh);
                    self.shaders.set_instance(&key, &mesh_material.shader, &mesh_material.params);
                    model.mesh_materials.insert(mesh.clone(), key);
                }
            }
        }
        let mut materials: HashSet<String> = self.game_models.values()
            .flat_map(|model| model.material_ranges.iter().map(|(material, _)| material.clone()).chain(model.mesh_materials.values().cloned()))
            .collect();
        materials.insert(STANDARD.to_owned());
        self.material_placements.clear();
        for material in &materials {
            let placement = self.shaders.placement(material, &passes);
            self.material_placements.insert(material.clone(), placement);
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
            self.shaders.set_instance(key, &material.shader, &material.params);
            overlays.push((key.clone(), Kind::Ui));
        }
        for (name, lines) in self.render_sets.all_lines() {
            let key = RenderSets::lines_key(name, lines);
            self.shaders.set_instance(&key, &lines.material.shader, &lines.material.params);
            overlays.push((key, Kind::Lines));
        }
        for (name, text) in self.render_sets.all_text() {
            let key = RenderSets::text_key(name, text);
            self.shaders.set_instance(&key, &text.material.shader, &text.material.params);
            overlays.push((key, Kind::Text));
        }
        materials.extend(overlays.iter().map(|(key, _)| key.clone()));
        materials.extend(["engine/ui_error", "engine/sdf_text_error", "engine/thick_lines_error"].map(str::to_owned));
        self.shaders.retain_instances(&materials);
        let ready: HashSet<String> = fullscreen.into_iter().filter(|key| self.shaders.prepare_fullscreen(key)).collect();
        self.overlay_materials.clear();
        for (key, kind) in overlays {
            if let Some(drawn) = self.shaders.prepare_overlay(&key, kind) {
                self.overlay_materials.insert(key, drawn);
            }
        }
        // Which parts of the tiled meshes draw this frame - the tiles in view
        // (see TiledMeshes), for a model drawn once (its tiles are where its
        // one instance is). Others draw whole.
        self.mesh_ranges.clear();
        self.tiled_meshes.begin_culling_stats();
        let camera_position = self.scene_manager.cameras().map(|cameras| cameras.active().camera.position().coords).unwrap_or_else(nalgebra::Vector3::zeros);
        let frustum = crate::engine::rendering::tiled_meshes::TiledMeshes::world_frustum(&nalgebra::Matrix4::from(self.camera_resources.uniform.view_proj), camera_position);
        if let Some(content) = self.scene_manager.content() {
            for (model_ref, model) in &self.game_models {
                let mut instances = content.renderizable_instances.values().filter(|instance| instance.model_ref == *model_ref);
                let (Some(instance), None) = (instances.next(), instances.next()) else { continue };
                for meshes in model.model.mesh_lists.values() {
                    for (mesh_key, mesh) in meshes {
                        if crate::resources::water_tier_of_mesh(&mesh.name).is_some_and(|tier| tier != self.tiled_meshes.tier()) {
                            continue;
                        }
                        if let Some(ranges) = self.tiled_meshes.visible_ranges(&mesh.name, frustum, instance.instance.transform.position) {
                            self.mesh_ranges.insert((model_ref.clone(), mesh_key.clone()), ranges);
                        }
                    }
                }
            }
        }
        // The bind groups 0 of materials with their own (params, textures) -
        // one for each mesh texture they're drawn with.
        for (model_ref, model) in &self.game_models {
            for meshes in model.model.mesh_lists.values() {
                for (mesh_key, mesh) in meshes {
                    let own = model.mesh_materials.get(mesh_base_name(mesh_key));
                    let keys: Vec<&String> = match own {
                        Some(key) => vec![key],
                        None => model.material_ranges.iter().map(|(key, _)| key).collect(),
                    };
                    for key in keys {
                        let placed = self.material_placements.get(key).is_some_and(|placement| *placement != Placement::Error);
                        if placed && self.shaders.has_group0(key) {
                            self.shaders.prepare_group0(key, model_ref, mesh.material, &model.model.materials[mesh.material].diffuse_texture);
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
        let Some((pipeline, bind_group)) = self.shaders.fullscreen(key) else { return };
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Fullscreen pass"),
            color_attachments: &[color_attachment(view, wgpu::LoadOp::Load)],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        render_pass.set_pipeline(pipeline);
        render_pass.set_bind_group(0, bind_group, &[]);
        render_pass.set_bind_group(1, &self.light.rendering_data.bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }

    /// Draws, into `render_pass`, every model's meshes whose material is
    /// placed in the scene pass `pass` (see `place_materials`): like
    /// engine/standard - solid meshes in "opaque", see-through ones in
    /// "transparent" -, all of them in the pass its shader names, or (with
    /// engine/error) in "opaque". A mesh's material is its model's own for it
    /// (`mesh_materials`), else its instances'.
    fn draw_materials<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>, pass: &str) {
        for model_ref in self.distinct_model_refs(Some("sun")) {
            let Some(model_data) = self.game_models.get(&model_ref) else { continue };
            render_pass.set_vertex_buffer(1, model_data.instance_buffer.slice(..));
            // Before the first frame's instances are written: all standard.
            let first_frame = [(STANDARD.to_owned(), 0..model_data.instance_count)];
            let ranges = if model_data.material_ranges.is_empty() { &first_frame[..] } else { &model_data.material_ranges[..] };
            for (material, instances) in ranges {
                for list in ["opaque", "transparent"] {
                    let Some(meshes) = model_data.model.mesh_lists.get(list) else { continue };
                    for (mesh_key, mesh) in meshes {
                        // A tiered mesh only at its altitude (see TiledMeshes).
                        if crate::resources::water_tier_of_mesh(&mesh.name).is_some_and(|tier| tier != self.tiled_meshes.tier()) {
                            continue;
                        }
                        let key = model_data.mesh_materials.get(mesh_base_name(mesh_key)).unwrap_or(material);
                        let Some(placement) = self.material_placements.get(key) else { continue };
                        let Some((single, double, shell)) = self.mesh_pipelines(key, placement, pass, list) else { continue };
                        let Some(single) = self.shaders.pipeline(key, placement, single) else { continue };
                        let double = double.and_then(|pipeline| self.shaders.pipeline(key, placement, pipeline));
                        let shell = shell.and_then(|pipeline| self.shaders.pipeline(key, placement, pipeline));
                        // Its own bind group 0 (params, textures), or the
                        // mesh's (just its texture).
                        let group0 = if *placement != Placement::Error && self.shaders.has_group0(key) {
                            let Some(group0) = self.shaders.group0(key, &model_ref, mesh.material) else { continue };
                            group0
                        } else {
                            &model_data.model.materials[mesh.material].bind_group
                        };
                        render_pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                        render_pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                        render_pass.set_bind_group(0, group0, &[]);
                        render_pass.set_bind_group(1, &self.camera_resources.bind_group, &[]);
                        render_pass.set_bind_group(2, &mesh.transform_bind_group, &[]);
                        render_pass.set_bind_group(3, &self.light.rendering_data.bind_group, &[]);
                        // All of it - or, tiled, the tiles in view.
                        let whole = [0..mesh.num_elements];
                        let parts = self.mesh_ranges.get(&(model_ref.clone(), mesh_key.clone())).map_or(&whole[..], Vec::as_slice);
                        let mut draw = |pipeline: &'a wgpu::RenderPipeline| {
                            render_pass.set_pipeline(pipeline);
                            for part in parts {
                                render_pass.draw_indexed(part.clone(), 0, instances.clone());
                            }
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
                let settings = self.shaders.settings(key)?;
                Some((pipeline(settings.cull, settings.depth_write, settings.blend()), Some(pipeline(Cull::None, settings.depth_write, settings.blend())), None))
            }
            (Placement::Error, "opaque", _) => Some((pipeline(Cull::Back, true, Blend::None), Some(pipeline(Cull::None, true, Blend::None)), None)),
            _ => None,
        }
    }

    // Particles and trails (see ParticleRenderer) - after everything solid,
    // the water and transparent models, so they blend over all of it. The
    // update (compute) pass runs first, then the scene's depth - now
    // including the water - is snapshotted again into foam_depth_copy for
    // their soft fading (the water pass is done reading its own copy by then).
    fn render_particle_pass(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if !self.particles.has_work() || !self.scene_manager.cameras().is_some_and(|c| c.has_active_camera()) {
            return;
        }
        self.particles.simulate(encoder);
        self.renderer.depth_render.snapshot_for_water(encoder);

        let view = &self.renderer.blur.scene_color.view;
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Particle Render Pass"),
            color_attachments: &[color_attachment(view, wgpu::LoadOp::Load)],
            depth_stencil_attachment: depth_attachment(&self.renderer.depth_render.texture.view, wgpu::LoadOp::Load),
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        self.particles.draw(&mut render_pass, &self.camera_resources.bind_group);
    }

    // Physics debug lines and UI/text share this pass since they're both drawn on top,
    // without a depth buffer, gated on there being UI geometry to draw at all.
    // `which` splits the main layer from the always-on-top one (see UiLayer).
    fn render_ui_pass(&mut self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, which: UiLayer) {
        if self.ui.ui_rendering.num_indices == 0 && self.ui.image_draws.is_empty() {
            return;
        }

        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("UI Render Pass"),
            color_attachments: &[color_attachment(view, wgpu::LoadOp::Load)],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });

        render_pass.set_pipeline(&self.render_physics.render_pipeline);
        render_pass.set_bind_group(0, &self.render_physics.bind_group, &[]);
        render_pass.set_bind_group(1, &self.camera_resources.bind_group, &[]);

        if which != UiLayer::Main && !self.show_depth_map && (self.render_physics.visible || !self.world_lines.is_empty()) {
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
            self.draw_ui_boxes(&mut render_pass, 0, main_index_count);
        }

        // One draw call per distinct image - see Ui::build_image_draws. Not
        // itself split into a main/on-top pair (nothing always_on_top uses
        // Image content today), so it renders as one batch here, same relative
        // position (after the main pass's quads, before the on-top pass) it
        // always has.
        if which != UiLayer::OnTop && !self.ui.image_draws.is_empty() {
            for draw in &self.ui.image_draws {
                let material = self.overlay_materials.get(&draw.material).and_then(|key| self.shaders.overlay(key, Overlay::UiImage));
                if let (Some(image), Some((pipeline, material))) = (self.ui.images.get(&draw.path), material) {
                    render_pass.set_pipeline(pipeline);
                    render_pass.set_bind_group(0, material, &[]);
                    render_pass.set_bind_group(1, &self.camera_resources.bind_group, &[]);
                    render_pass.set_bind_group(2, &image.bind_group, &[]);
                    render_pass.set_bind_group(3, &self.light.rendering_data.bind_group, &[]);
                    render_pass.set_vertex_buffer(0, draw.vertex_buffer.slice(..));
                    render_pass.set_index_buffer(draw.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                    render_pass.draw_indexed(0..draw.num_indices, 0, 0..1);
                }
            }
        }

        // Render text (text renderer handles empty content gracefully)
        if which != UiLayer::OnTop {
            self.ui.text.text_renderer.render(&self.ui.text.text_atlas, &self.renderer.glyphon.viewport, &mut render_pass).unwrap();
        }

        if which != UiLayer::Main && self.ui.ui_rendering.num_indices > main_index_count {
            self.draw_ui_boxes(&mut render_pass, main_index_count, self.ui.ui_rendering.num_indices);
        }

        if which != UiLayer::Main {
            self.ui.text.text_renderer_on_top.render(&self.ui.text.text_atlas, &self.renderer.glyphon.viewport, &mut render_pass).unwrap();
        }
    }

    /// Draws the UI's boxes `from..to` (indices), one draw call for each run
    /// of the same material (see UiRendering::runs).
    fn draw_ui_boxes(&self, render_pass: &mut wgpu::RenderPass<'_>, from: u32, to: u32) {
        render_pass.set_vertex_buffer(0, self.ui.ui_rendering.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.ui.ui_rendering.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        render_pass.set_bind_group(1, &self.camera_resources.bind_group, &[]);
        render_pass.set_bind_group(2, &self.ui.blur_bind_group, &[]);
        render_pass.set_bind_group(3, &self.light.rendering_data.bind_group, &[]);
        for (range, key) in self.ui.ui_rendering.runs_between(from, to) {
            let Some((pipeline, material)) = self.overlay_materials.get(key).and_then(|key| self.shaders.overlay(key, Overlay::UiBox)) else { continue };
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
        let physics_lines: &[[ManualVertex; 2]] = if self.render_physics.visible { &self.render_physics.renderizable_lines } else { &[] };
        let vertices: Vec<ManualVertex> = physics_lines.iter().chain(&self.world_lines)
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

        self.render_physics.vertex_buffer = self.renderer.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Updated ManualVertex Buffer"),
            size: (vertices.len() * std::mem::size_of::<ManualVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        self.render_physics.vertex_buffer.slice(..).get_mapped_range_mut().copy_from_slice(bytemuck::cast_slice(&vertices));
        self.render_physics.vertex_buffer.unmap();

        // Each line has two vertices, one after the other.
        let indices: Vec<u32> = (0..vertices.len() as u32).collect();

        self.render_physics.index_buffer = self.renderer.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Index Buffer"),
            size: (indices.len() * std::mem::size_of::<u32>()) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        self.render_physics.index_buffer.slice(..).get_mapped_range_mut().copy_from_slice(bytemuck::cast_slice(&indices));
        self.render_physics.index_buffer.unmap();

        render_pass.set_vertex_buffer(0, self.render_physics.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.render_physics.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
        render_pass.draw_indexed(0..(indices.len() as u32), 0, 0..1);
    }

    /// Draws this frame's plan (`App::frame_plan` - the scene's) - see
    /// engine::rendering::frame.
    pub(crate) fn render_scene_passes(&mut self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        let mut plan = std::mem::take(&mut self.frame_plan);
        self.run_frame_plan(&mut plan, encoder, view);
        self.frame_plan = plan;
    }

    /// Runs `plan`'s steps in order, onto `screen` (or the target a step
    /// switched to). A step naming something that doesn't exist is skipped,
    /// and said once.
    fn run_frame_plan(&mut self, plan: &mut FramePlan, encoder: &mut wgpu::CommandEncoder, screen: &wgpu::TextureView) {
        // The image the steps draw into - the screen, or a named target's (a
        // fresh view, so it can be borrowed while `self` is in use).
        let mut target: Option<wgpu::TextureView> = None;
        let mut missing = Vec::new();
        let fullscreen_ready = self.place_materials(plan);
        // The scene blurred for the UI's frosted panels - once, before the
        // first UI step, once the world's drawn.
        let mut blurred = false;
        for step in &plan.steps {
            let view = target.as_ref().unwrap_or(screen);
            match step {
                FrameStep::Scene { pass } => match pass.as_str() {
                    "opaque" => self.render_opaque_pass(encoder),
                    "transparent" => self.render_transparent_pass(encoder),
                    _ => self.render_scene_pass(encoder, pass),
                },
                // Before the passes that read it - copy_texture_to_texture
                // isn't valid mid-pass.
                FrameStep::CopyDepth => {
                    if self.scene_manager.cameras().is_some_and(|c| c.has_active_camera()) {
                        self.renderer.depth_render.snapshot_for_water(encoder);
                    }
                }
                FrameStep::Particles => self.render_particle_pass(encoder),
                FrameStep::Target { name, clear } => match name.as_str() {
                    "hud" => {
                        let hud = self.renderer.blur.hud_color.texture.create_view(&wgpu::TextureViewDescriptor::default());
                        if *clear {
                            self.clear_hud_target(encoder, &hud);
                        }
                        target = Some(hud);
                    }
                    _ => missing.push(format!("target \"{name}\" doesn't exist")),
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
                    if !blurred {
                        self.renderer.blur.blur_scene(encoder);
                        blurred = true;
                    }
                    self.render_ui_pass(encoder, view, *layer);
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
        let screen = [self.window_manager.pixel_size.width as f32, self.window_manager.pixel_size.height as f32];
        let Some(lines) = self.render_sets.get_lines_mut(set) else { return false };
        if lines.is_empty() {
            return true;
        }
        let key = RenderSets::lines_key(set, lines);
        let Some(material) = self.overlay_materials.get(&key).and_then(|key| self.shaders.overlay(key, Overlay::Lines)) else { return true };
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
            &self.camera_resources.bind_group,
            &self.light.rendering_data.bind_group,
            [camera_position.x, camera_position.y, camera_position.z],
            screen,
        );
        true
    }

    /// Draws the SDF text set `set` into `view`, over what's there - false
    /// if there's no such set. Nothing when it's empty.
    fn draw_text_set(&mut self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, set: &str) -> bool {
        let camera_position = self.scene_manager.cameras().map(|c| c.active().camera.position()).unwrap_or_else(nalgebra::Point3::origin);
        let screen = [self.window_manager.pixel_size.width as f32, self.window_manager.pixel_size.height as f32];
        let Some(text) = self.render_sets.get_text_mut(set) else { return false };
        if text.is_empty() {
            return true;
        }
        let key = RenderSets::text_key(set, text);
        let Some(material) = self.overlay_materials.get(&key).and_then(|key| self.shaders.overlay(key, Overlay::Text)) else { return true };
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
            &self.camera_resources.bind_group,
            &self.light.rendering_data.bind_group,
            [camera_position.x, camera_position.y, camera_position.z],
            screen,
        );
        true
    }

    /// Clears the flight-HUD target to transparent before this frame's symbols.
    fn clear_hud_target(&self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("HUD clear"),
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
