use std::collections::HashSet;

use wgpu::RenderPassDepthStencilAttachment;

use crate::app::App;
use crate::engine::primitive::manual_vertex::ManualVertex;
use crate::engine::rendering::models::model::DrawModel;
use crate::engine::rendering::frame::{FramePlan, FrameStep};
use crate::engine::rendering::shader_library::{Blend, Cull, PipelineKey, Placement, STANDARD};

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

        // Water-shaded models draw in their own later pass instead (see
        // render_water_pass) - they need a snapshot of this pass's own depth
        // output to sample from, so they can't be part of this pass.
        self.draw_materials(&mut render_pass, "opaque");
    }

    // Runs after render_opaque_pass - with the frame plan's CopyDepth step
    // between them, which snapshots its depth output (DepthRender::
    // snapshot_for_water) for this to read - and before render_transparent_pass -
    // its own render pass rather than folded into render_opaque_pass's loop,
    // since a water-shaded model needs to sample a copy of the depth buffer
    // render_opaque_pass just finished writing, and wgpu doesn't allow reading a
    // texture that's also this same pass's own depth-stencil attachment (same
    // "can't read what you're writing" constraint BlurRender's scene_color
    // already works around for color). Draws with raw wgpu calls instead of the
    // shared draw_model_instanced_from_list/draw_mesh_instanced helpers other
    // models use, since those unconditionally bind each mesh's own *material* at
    // group 0 - here group 0 is the depth snapshot instead (see
    // WaterRenderData's own doc comment), constant for the whole pass rather
    // than rebound per mesh.
    fn render_water_pass(&mut self, encoder: &mut wgpu::CommandEncoder) {
        // The models drawn with "engine/water".
        let water_models: HashSet<String> = self.game_models.iter()
            .filter(|(_, model)| model.material_ranges.iter().any(|(material, _)| self.material_placements.get(material) == Some(&Placement::Water)))
            .map(|(model_ref, _)| model_ref.clone())
            .collect();
        if water_models.is_empty() {
            return;
        }
        if !self.scene_manager.cameras().is_some_and(|c| c.has_active_camera()) {
            return;
        }

        let view = &self.renderer.blur.scene_color.view;
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Water Render Pass"),
            color_attachments: &[color_attachment(view, wgpu::LoadOp::Load)],
            depth_stencil_attachment: depth_attachment(&self.renderer.depth_render.texture.view, wgpu::LoadOp::Load),
            occlusion_query_set: None,
            timestamp_writes: None,
        });

        render_pass.set_pipeline(&self.water.render_pipeline);
        render_pass.set_bind_group(0, self.water.bind_group(), &[]);
        render_pass.set_bind_group(1, &self.camera_resources.bind_group, &[]);
        render_pass.set_bind_group(3, &self.light.rendering_data.bind_group, &[]);

        // Ocean culling (see WaterRenderData::visible_ranges): the camera's
        // view volume in world space, and fresh counters for the F3 panel.
        let camera_position = self.scene_manager.cameras().map(|cameras| cameras.active().camera.position().coords).unwrap_or_else(nalgebra::Vector3::zeros);
        let frustum = crate::engine::rendering::render_pipeline::water_renderer::WaterRenderData::world_frustum(&nalgebra::Matrix4::from(self.camera_resources.uniform.view_proj), camera_position);
        self.water.begin_culling_stats();

        for model_ref in water_models {
            // Where this model's (single) instance sits in the world - tile
            // boxes are relative to it. With several instances there's no one
            // place to cull against, so those draw whole.
            let mut instances = self.scene_manager.content().into_iter()
                .flat_map(|content| content.renderizable_instances.values())
                .filter(|instance| instance.model_ref == model_ref);
            let mesh_position = match (instances.next(), instances.next()) {
                (Some(instance), None) => Some(instance.instance.transform.position),
                _ => None,
            };
            // See App::water_debug_hidden_models's own doc comment.
            if self.water_debug_view && self.water_debug_hidden_models.contains(&model_ref) {
                continue;
            }
            let Some(model_data) = self.game_models.get(&model_ref) else { continue };
            let Some(meshes) = model_data.model.mesh_lists.get("opaque") else { continue };

            render_pass.set_vertex_buffer(1, model_data.instance_buffer.slice(..));
            for mesh in meshes.values() {
                // Ocean meshes come in altitude tiers - only the current
                // one draws (see WaterRenderData::tier).
                if crate::resources::water_tier_of_mesh(&mesh.name).is_some_and(|tier| tier != self.water.tier()) {
                    continue;
                }
                render_pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                render_pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                render_pass.set_bind_group(2, &mesh.transform_bind_group, &[]);
                let visible = mesh_position.and_then(|position| self.water.visible_ranges(&mesh.name, frustum, position));
                match visible {
                    Some(ranges) => {
                        for range in ranges {
                            render_pass.draw_indexed(range, 0, 0..model_data.instance_count as u32);
                        }
                    }
                    None => render_pass.draw_indexed(0..mesh.num_elements, 0, 0..model_data.instance_count as u32),
                }
            }
        }
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

    /// A scene pass of the game's own (any name but "opaque", "water" and
    /// "transparent"): over the world drawn so far, against its depth -
    /// every model whose material's shader draws in `pass`.
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
    /// any shader that isn't yet (see ShaderLibrary::placement).
    fn place_materials(&mut self, plan: &FramePlan) {
        let passes: HashSet<String> = plan.steps.iter()
            .filter_map(|step| match step {
                FrameStep::Scene { pass } => Some(pass.clone()),
                _ => None,
            })
            .collect();
        let mut materials: HashSet<String> = self.game_models.values()
            .flat_map(|model| model.material_ranges.iter().map(|(material, _)| material.clone()))
            .collect();
        materials.insert(STANDARD.to_owned());
        self.material_placements.clear();
        for material in materials {
            let placement = self.shaders.placement(&material, &passes);
            self.material_placements.insert(material, placement);
        }
    }

    /// Draws, into `render_pass`, every model's instances whose material is
    /// placed in the scene pass `pass` (see `place_materials`): like
    /// engine/standard - solid meshes in "opaque", see-through ones in
    /// "transparent" -, all of a model's meshes in the pass its shader names,
    /// or (with engine/error) in "opaque".
    fn draw_materials<'a>(&'a self, render_pass: &mut wgpu::RenderPass<'a>, pass: &str) {
        let solid = "opaque".to_string();
        let see_through = "transparent".to_string();
        for model_ref in self.distinct_model_refs(Some("sun")) {
            let Some(model_data) = self.game_models.get(&model_ref) else { continue };
            render_pass.set_vertex_buffer(1, model_data.instance_buffer.slice(..));
            // Before the first frame's instances are written: all standard.
            let first_frame = [(STANDARD.to_owned(), 0..model_data.instance_count)];
            let ranges = if model_data.material_ranges.is_empty() { &first_frame[..] } else { &model_data.material_ranges[..] };
            for (material, instances) in ranges {
                let Some(placement) = self.material_placements.get(material) else { continue };
                let key = |cull, depth_write, blend| PipelineKey { cull, depth_write, blend };
                // Which mesh lists, with which pipelines: (list, single-sided,
                // double-sided, double-sided shell drawn far side first).
                let draws: Vec<(&String, PipelineKey, Option<PipelineKey>, Option<PipelineKey>)> = match (placement, pass) {
                    (Placement::ByMesh, "opaque") => vec![(&solid, key(Cull::Back, true, Blend::Alpha), Some(key(Cull::None, true, Blend::Alpha)), None)],
                    (Placement::ByMesh, "transparent") => vec![(&see_through, key(Cull::Back, false, Blend::Alpha), None, Some(key(Cull::Front, false, Blend::Alpha)))],
                    (Placement::Pass(own), _) if own == pass => {
                        let Some(settings) = self.shaders.settings(material) else { continue };
                        let single = key(settings.cull, settings.depth_write, settings.blend);
                        let double = Some(key(Cull::None, settings.depth_write, settings.blend));
                        vec![(&solid, single, double, None), (&see_through, single, double, None)]
                    }
                    (Placement::Error, "opaque") => {
                        let (single, double) = (key(Cull::Back, true, Blend::None), Some(key(Cull::None, true, Blend::None)));
                        vec![(&solid, single, double, None), (&see_through, single, double, None)]
                    }
                    _ => continue,
                };
                for (list, single, double, shell) in draws {
                    let Some(single) = self.shaders.pipeline(material, placement, single) else { continue };
                    let double = double.and_then(|key| self.shaders.pipeline(material, placement, key));
                    let shell = shell.and_then(|key| self.shaders.pipeline(material, placement, key));
                    render_pass.draw_model_instanced_from_list(
                        &model_data.model, instances.clone(),
                        &self.camera_resources.bind_group, &self.light.rendering_data.bind_group,
                        list,
                        single, double, shell,
                    );
                }
            }
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
        // draw pass from these quads (glyphon, not the ui_pipeline below), so
        // without this split every node's text ended up in one shared final
        // pass with no z-order relative to a quad drawn after it: a modal's
        // opaque background quad could correctly land above a panel behind it,
        // while that same panel's *text* still rendered on top of the modal
        // regardless, since all text - modal's and panel's alike - drew after
        // all quads unconditionally.
        let main_index_count = self.ui.ui_rendering.main_index_count;
        if which != UiLayer::OnTop && main_index_count > 0 {
            render_pass.set_pipeline(&self.ui.ui_pipeline);
            render_pass.set_bind_group(0, &self.ui.blur_bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.ui.ui_rendering.vertex_buffer.slice(..));
            render_pass.set_index_buffer(self.ui.ui_rendering.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            render_pass.draw_indexed(0..main_index_count, 0, 0..1);
        }

        // One draw call per distinct image - see Ui::build_image_draws. Not
        // itself split into a main/on-top pair (nothing always_on_top uses
        // Image content today), so it renders as one batch here, same relative
        // position (after the main pass's quads, before the on-top pass) it
        // always has.
        if which != UiLayer::OnTop && !self.ui.image_draws.is_empty() {
            render_pass.set_pipeline(&self.ui.image_pipeline);
            for draw in &self.ui.image_draws {
                if let Some(image) = self.ui.images.get(&draw.path) {
                    render_pass.set_bind_group(0, &image.bind_group, &[]);
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
            render_pass.set_pipeline(&self.ui.ui_pipeline);
            render_pass.set_bind_group(0, &self.ui.blur_bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.ui.ui_rendering.vertex_buffer.slice(..));
            render_pass.set_index_buffer(self.ui.ui_rendering.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            render_pass.draw_indexed(main_index_count..self.ui.ui_rendering.num_indices, 0, 0..1);
        }

        if which != UiLayer::Main {
            self.ui.text.text_renderer_on_top.render(&self.ui.text.text_atlas, &self.renderer.glyphon.viewport, &mut render_pass).unwrap();
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
        self.place_materials(plan);
        for step in &plan.steps {
            let view = target.as_ref().unwrap_or(screen);
            match step {
                FrameStep::Scene { pass } => match pass.as_str() {
                    "opaque" => self.render_opaque_pass(encoder),
                    "water" => self.render_water_pass(encoder),
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
                FrameStep::Ui { layer } => self.render_ui_pass(encoder, view, *layer),
                FrameStep::Fullscreen { shader } => match shader.as_str() {
                    // The world (scene color) and the "hud" target onto this
                    // target, under the G-force effects.
                    "engine/scene_blit" => {
                        self.renderer.blur.write_effects(&self.renderer.queue);
                        self.renderer.blur.render(encoder, view);
                    }
                    _ => missing.push(format!("full-screen shader \"{shader}\" doesn't exist")),
                },
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
            &self.camera_resources.bind_group,
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
            &self.camera_resources.bind_group,
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
