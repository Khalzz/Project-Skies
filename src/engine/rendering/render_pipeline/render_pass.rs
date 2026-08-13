use std::collections::HashSet;

use wgpu::RenderPassDepthStencilAttachment;

use crate::app::App;
use crate::engine::primitive::manual_vertex::ManualVertex;
use crate::engine::rendering::models::model::DrawModel;

fn color_attachment(view: &wgpu::TextureView, load: wgpu::LoadOp<wgpu::Color>) -> Option<wgpu::RenderPassColorAttachment> {
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
        self.renderizable_instances.iter()
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
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Opaque Render Pass"),
            color_attachments: &[color_attachment(view, wgpu::LoadOp::Clear(self.clear_color))],
            // Reversed-Z: clear to 0.0 ("infinitely far") instead of 1.0.
            depth_stencil_attachment: depth_attachment(&self.renderer.depth_render.texture.view, wgpu::LoadOp::Clear(0.0)),
            occlusion_query_set: None,
            timestamp_writes: None,
        });

        if let Some(skybox) = &self.skybox {
            skybox.render(&mut render_pass, &self.camera.bind_group);
        }

        render_pass.set_pipeline(&self.render_pipeline);

        for model_ref in self.distinct_model_refs(Some("sun")) {
            if let Some(model_data) = self.game_models.get(&model_ref) {
                render_pass.set_vertex_buffer(1, model_data.instance_buffer.slice(..));
                render_pass.draw_model_instanced_from_list(&model_data.model, 0..model_data.instance_count as u32, &self.camera.bind_group, &self.light.rendering_data.bind_group, &"opaque".to_string());
            }
        }
    }

    // Same scene_color target as render_opaque_pass above, and for the same reason.
    fn render_transparent_pass(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let view = &self.renderer.blur.scene_color.view;
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Transparent Render Pass"),
            color_attachments: &[color_attachment(view, wgpu::LoadOp::Load)],
            depth_stencil_attachment: depth_attachment(&self.renderer.depth_render.texture.view, wgpu::LoadOp::Load),
            occlusion_query_set: None,
            timestamp_writes: None,
        });

        render_pass.set_pipeline(&self.render_pipeline);

        for model_ref in self.distinct_model_refs(None) {
            if let Some(model_data) = self.game_models.get(&model_ref) {
                render_pass.set_vertex_buffer(1, model_data.instance_buffer.slice(..));
                render_pass.draw_model_instanced_from_list(&model_data.model, 0..model_data.instance_count as u32, &self.camera.bind_group, &self.light.rendering_data.bind_group, &"transparent".to_string());
            }
        }
    }

    // Physics debug lines and UI/text share this pass since they're both drawn on top,
    // without a depth buffer, gated on there being UI geometry to draw at all.
    fn render_ui_pass(&mut self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
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
        render_pass.set_bind_group(1, &self.camera.bind_group, &[]);

        if !self.show_depth_map && self.render_physics.visible {
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
        if main_index_count > 0 {
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
        if !self.ui.image_draws.is_empty() {
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
        self.ui.text.text_renderer.render(&self.ui.text.text_atlas, &self.renderer.glyphon.viewport, &mut render_pass).unwrap();

        if self.ui.ui_rendering.num_indices > main_index_count {
            render_pass.set_pipeline(&self.ui.ui_pipeline);
            render_pass.set_bind_group(0, &self.ui.blur_bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.ui.ui_rendering.vertex_buffer.slice(..));
            render_pass.set_index_buffer(self.ui.ui_rendering.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            render_pass.draw_indexed(main_index_count..self.ui.ui_rendering.num_indices, 0, 0..1);
        }

        self.ui.text.text_renderer_on_top.render(&self.ui.text.text_atlas, &self.renderer.glyphon.viewport, &mut render_pass).unwrap();
    }

    // Debug lines come from the physics thread in absolute world coordinates,
    // so make them camera-relative here to match camera.view_proj.
    fn render_physics_debug_lines<'rp>(&mut self, render_pass: &mut wgpu::RenderPass<'rp>) {
        let camera_position = self.camera.active().camera.position;
        let vertices: Vec<ManualVertex> = self.render_physics.renderizable_lines.iter()
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

        // Each line has two vertices
        let mut indices = Vec::new();
        for i in 0..self.render_physics.renderizable_lines.len() {
            let base_index = (i * 2) as u16;
            indices.push(base_index);
            indices.push(base_index + 1);
        }

        if indices.is_empty() {
            return;
        }

        self.render_physics.index_buffer = self.renderer.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Index Buffer"),
            size: (indices.len() * std::mem::size_of::<u16>()) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        self.render_physics.index_buffer.slice(..).get_mapped_range_mut().copy_from_slice(bytemuck::cast_slice(&indices));
        self.render_physics.index_buffer.unmap();

        render_pass.set_vertex_buffer(0, self.render_physics.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.render_physics.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        render_pass.draw_indexed(0..(indices.len() as u32), 0, 0..1);
    }

    pub(crate) fn render_scene_passes(&mut self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        self.render_opaque_pass(encoder);
        self.render_transparent_pass(encoder);
        // Blits scene_color onto the real swapchain view, so the scene still looks
        // normal everywhere - see BlurRender::render. Has to run after the 3D
        // passes above (nothing to blit yet otherwise) and before the UI pass
        // below, which samples scene_color directly (at whatever per-node radius,
        // see text_shader.wgsl) for any node with background_blur set.
        self.renderer.blur.render(encoder, view);
        self.render_ui_pass(encoder, view);
    }
}
