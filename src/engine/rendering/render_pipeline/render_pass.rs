use std::collections::HashSet;

use wgpu::RenderPassDepthStencilAttachment;

use nalgebra::Vector3;

use crate::app::App;
use crate::engine::input::input;
use crate::engine::primitive::manual_vertex::ManualVertex;
use crate::engine::rendering::models::model::DrawModel;
use crate::game::scenes::play::camera::camera::Camera;
use crate::game::scenes::play::plane::physics::rolling_rate::{max_roll_rate_deg_s, RollRateParams};
use crate::game::scenes::play::plane::plane::Plane;

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

        // draw_model_instanced_from_list sets the pipeline itself, per mesh
        // (single- vs double-sided), so no set_pipeline here.
        for model_ref in self.distinct_model_refs(Some("sun")) {
            // Water-shaded models draw in their own later pass instead (see
            // render_water_pass) - they need a snapshot of this pass's own
            // depth output to sample from, so they can't be part of this
            // pass themselves.
            if self.water_shaded_models.contains(&model_ref) {
                continue;
            }
            if let Some(model_data) = self.game_models.get(&model_ref) {
                render_pass.set_vertex_buffer(1, model_data.instance_buffer.slice(..));
                render_pass.draw_model_instanced_from_list(
                    &model_data.model, 0..model_data.instance_count as u32,
                    &self.camera_resources.bind_group, &self.light.rendering_data.bind_group,
                    &"opaque".to_string(),
                    &self.render_pipeline, Some(&self.render_pipeline_double_sided), None,
                );
            }
        }
    }

    // Runs after render_opaque_pass (needs its depth output already snapshotted,
    // see DepthRender::snapshot_for_water) and before render_transparent_pass -
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
        if self.water_shaded_models.is_empty() {
            return;
        }
        if !self.scene_manager.cameras().is_some_and(|c| c.has_active_camera()) {
            return;
        }

        // Has to happen before begin_render_pass (copy_texture_to_texture isn't
        // valid mid-pass) and before this pass writes any depth of its own.
        self.renderer.depth_render.snapshot_for_water(encoder);

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

        for model_ref in self.distinct_model_refs(Some("sun")) {
            if !self.water_shaded_models.contains(&model_ref) {
                continue;
            }
            // See App::water_debug_hidden_models's own doc comment.
            if self.water_debug_view && self.water_debug_hidden_models.contains(&model_ref) {
                continue;
            }
            let Some(model_data) = self.game_models.get(&model_ref) else { continue };
            let Some(meshes) = model_data.model.mesh_lists.get("opaque") else { continue };

            render_pass.set_vertex_buffer(1, model_data.instance_buffer.slice(..));
            for mesh in meshes.values() {
                render_pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                render_pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                render_pass.set_bind_group(2, &mesh.transform_bind_group, &[]);
                render_pass.draw_indexed(0..mesh.num_elements, 0, 0..model_data.instance_count as u32);
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

        // Pipeline is set per mesh inside draw_model_instanced_from_list.
        for model_ref in self.distinct_model_refs(None) {
            if let Some(model_data) = self.game_models.get(&model_ref) {
                render_pass.set_vertex_buffer(1, model_data.instance_buffer.slice(..));
                render_pass.draw_model_instanced_from_list(
                    &model_data.model, 0..model_data.instance_count as u32,
                    &self.camera_resources.bind_group, &self.light.rendering_data.bind_group,
                    &"transparent".to_string(),
                    &self.render_pipeline_transparent, None, Some(&self.render_pipeline_transparent_front_cull),
                );
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
        render_pass.set_bind_group(1, &self.camera_resources.bind_group, &[]);

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

    // F7 (see App::show_aero_debug_overlay's own doc comment) - two live
    // egui windows (see engine::rendering::egui_overlay for why it's egui
    // rendered into this same wgpu frame rather than a separate native
    // window): the roll-authority-gain chart (rolling_rate's theoretical
    // curve against the player's own recent trail), plus a second "Debug
    // Info" window with everything else Plane::flight_data tracks (speed,
    // altitude, mach, g, AoA, turn rates, ...) - this used to only be
    // readable from the in-HUD text labels (still there, unaffected) or the
    // old F3 debug_panel (FPS/position only) - this consolidates the same
    // numbers into one place alongside the chart they're for tuning
    // against. Input is deliberately minimal (pointer position + primary
    // button + scroll) - just enough for egui_plot's own pan/zoom/hover,
    // not full egui interactivity (no keyboard/text - this is a read-only
    // debug view).
    fn render_aero_debug_overlay_pass(&mut self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        if !self.show_aero_debug_overlay {
            return;
        }

        // screen_rect/pointer input have to be in LOGICAL (window, not
        // drawable) pixels - the same space input::mouse_x()/mouse_y()
        // already report in (see that fn's own doc comment) - while the
        // actual render target (`view`) is sized in PHYSICAL/drawable
        // pixels. Mixing these up is what confines the whole overlay into
        // one corner of the window on a HiDPI/Retina display instead of
        // filling it - see EguiOverlay::render's own doc comment on
        // pixels_per_point for why both are needed.
        let logical_size = [self.window_manager.size.width, self.window_manager.size.height];
        let physical_size = [self.window_manager.pixel_size.width, self.window_manager.pixel_size.height];
        let pixels_per_point = physical_size[0] as f32 / logical_size[0] as f32;
        // Both right-aligned against the actual (logical) screen width -
        // "Debug Info" top-right, chart bottom-right beneath it, each
        // independently right-aligned to its own estimated window width
        // (300 for Debug Info's narrow text column, 580 for the chart's
        // wider plot) rather than a fixed offset from each other, so both
        // actually hug the real right edge on any screen size. 420 for the
        // chart's Y assumes Debug Info's own window (title bar + ~13 rows +
        // 2 separators + padding) is roughly that tall - not exact, just
        // enough clearance that they don't start out overlapping.
        let debug_info_default_x = (logical_size[0] as f32 - 300.0).max(20.0);
        let aero_chart_default_x = (logical_size[0] as f32 - 580.0).max(20.0);
        let pointer_pos = egui::pos2(input::mouse_x() as f32, input::mouse_y() as f32);
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(logical_size[0] as f32, logical_size[1] as f32))),
            events: vec![
                egui::Event::PointerMoved(pointer_pos),
                egui::Event::PointerButton {
                    pos: pointer_pos,
                    button: egui::PointerButton::Primary,
                    pressed: input::mouse_left_button_down(),
                    modifiers: egui::Modifiers::default(),
                },
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: egui::vec2(0.0, input::mouse_scroll_y()),
                    modifiers: egui::Modifiers::default(),
                },
            ],
            ..Default::default()
        };

        // Theoretical curve at AoA=0, sea level, in actual deg/s (not the
        // dimensionless 0..1 roll_authority_gain fraction - that's correct
        // as a multiplier but reads as meaningless "0 to 1" on a chart meant
        // to be compared against real measured roll rate). Recomputed every
        // frame rather than cached - 801 samples through a cheap closed-form
        // function is negligible next to everything else this loop already
        // does per frame.
        let curve: Vec<[f64; 2]> = (0..=800)
            .map(|knots| {
                let speed_ms = knots as f32 * 0.514444;
                let roll_rate = max_roll_rate_deg_s(speed_ms, 0.0, 0.0, &RollRateParams::default());
                [knots as f64, roll_rate as f64]
            })
            .collect();
        let trail: Vec<[f64; 2]> = self.aero_debug_trail.iter()
            // .abs() - the stored value is the real, signed roll rate
            // (positive rolling one way, negative the other, same value
            // Plane::flight_data.roll_rate itself carries), but the
            // theoretical curve is always non-negative (a max achievable
            // MAGNITUDE, not a direction) - comparing a signed measurement
            // against an unsigned curve read as "weird negative outliers"
            // rather than the actual same-magnitude point it is.
            .map(|(speed_kt, _aoa_y_deg, roll_rate)| [*speed_kt as f64, roll_rate.abs() as f64])
            .collect();
        // See App::wing_lift_trail's own doc comment - two separate line
        // series sharing one x (sample index), gathered here same as
        // trail/curve above for the same borrow-checker reason.
        let main_wing_lift_trail: Vec<[f64; 2]> = self.wing_lift_trail.iter()
            .map(|(index, main_lift_y, _elevator_lift_y)| [*index as f64, *main_lift_y as f64])
            .collect();
        let elevator_wing_lift_trail: Vec<[f64; 2]> = self.wing_lift_trail.iter()
            .map(|(index, _main_lift_y, elevator_lift_y)| [*index as f64, *elevator_lift_y as f64])
            .collect();

        // Everything else this debug view shows - gathered up front into
        // plain locals (same reason curve/trail are, above) since the
        // build_ui closure below can't also borrow `self`. `flight_data` is
        // `Plane`'s own struct, already Copy-friendly f32s, so this is just
        // cheap field reads, not a real per-frame cost.
        let fps = self.time.get_fps();
        let player_position = self.scene_manager.content()
            .and_then(|content| content.renderizable_instances.get("player"))
            .map(|instance| instance.instance.transform.position);
        let flight_data = self.scene_manager.content()
            .and_then(|content| content.nodes.get("player"))
            .and_then(|node| node.get_behavior::<Plane>())
            .map(|plane| (plane.controls.throttle, plane.flight_data.speedometer, plane.flight_data.altimeter, plane.flight_data.mach, plane.flight_data.g_meter, plane.flight_data.aoa_x, plane.flight_data.aoa_y, plane.flight_data.aoa, plane.flight_data.roll_rate, plane.flight_data.pitch_rate, plane.flight_data.yaw_rate, plane.stall, plane.controls.aileron, plane.controls.elevator, plane.controls.rudder, plane.controls.trim.roll, plane.controls.trim.pitch, plane.controls.trim.yaw));

        // Play camera (the "camera" node's Camera behavior), read into plain
        // Copy locals for the build_ui closure. name / base offset / editor
        // offset / whether the editor is currently applied.
        let camera_info: Option<(&'static str, Vector3<f32>, Vector3<f32>, bool)> = self.scene_manager.content()
            .and_then(|content| content.nodes.get("camera"))
            .and_then(|node| node.get_behavior::<Camera>())
            .map(|cam| (cam.state_name(), cam.debug_base_offset, cam.debug_offset, cam.debug_mode_active));
        // Filled by the Camera Editor window's buttons; applied after render().
        let mut camera_offset_delta = Vector3::<f32>::zeros();
        let mut camera_set_active: Option<bool> = None;
        let mut camera_reset_offset = false;

        self.egui_overlay.render(
            &self.renderer.device,
            &self.renderer.queue,
            encoder,
            view,
            physical_size,
            pixels_per_point,
            raw_input,
            |ctx| {
                egui::Window::new("Camera Editor (F7)")
                    .default_pos(egui::pos2(20.0, 20.0))
                    .show(ctx, |ui| {
                        let Some((cam_name, base, editor, active)) = camera_info else {
                            ui.label("(no play camera)");
                            return;
                        };
                        // One nudge per click, in the plane's local frame
                        // (+Z forward, +Y up, +X left - matches Camera's own
                        // `target.rotation * debug_offset`).
                        const STEP: f32 = 0.25;

                        let resulting = base + editor;
                        ui.label(format!("Camera: {}", cam_name));
                        ui.label(format!("Base (state) offset: ({:+.2}, {:+.2}, {:+.2})", base.x, base.y, base.z));
                        ui.label(format!("Editor nudge:        ({:+.2}, {:+.2}, {:+.2})", editor.x, editor.y, editor.z));
                        ui.label(format!("Resulting offset:    ({:+.2}, {:+.2}, {:+.2})", resulting.x, resulting.y, resulting.z));
                        ui.separator();

                        ui.horizontal(|ui| {
                            ui.add_space(40.0);
                            if ui.button("  Up  ").clicked() { camera_offset_delta.y += STEP; }
                        });
                        ui.horizontal(|ui| {
                            if ui.button(" Left ").clicked() { camera_offset_delta.x += STEP; }
                            if ui.button("Right ").clicked() { camera_offset_delta.x -= STEP; }
                        });
                        ui.horizontal(|ui| {
                            ui.add_space(40.0);
                            if ui.button(" Down ").clicked() { camera_offset_delta.y -= STEP; }
                        });
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if ui.button("Forward").clicked() { camera_offset_delta.z += STEP; }
                            if ui.button(" Back  ").clicked() { camera_offset_delta.z -= STEP; }
                        });
                        ui.separator();

                        let mut active_mut = active;
                        if ui.checkbox(&mut active_mut, "Editor active (also F5)").changed() {
                            camera_set_active = Some(active_mut);
                        }
                        if ui.button("Reset nudge").clicked() {
                            camera_reset_offset = true;
                        }
                        ui.add_space(2.0);
                        ui.label(format!("position: Vector3::new({:.3}, {:.3}, {:.3})", resulting.x, resulting.y, resulting.z));
                    });

                egui::Window::new("Debug Info (F7)")
                    .default_pos(egui::pos2(debug_info_default_x, 20.0))
                    .show(ctx, |ui| {
                        ui.label(format!("FPS: {:.0}", fps));
                        if let Some(pos) = player_position {
                            ui.label(format!("Position: ({:.1}, {:.1}, {:.1})", pos.x, pos.y, pos.z));
                        } else {
                            ui.label("Position: (no player)");
                        }
                        ui.separator();
                        if let Some((throttle, speedometer, altimeter, mach, g_meter, aoa_x, aoa_y, aoa, roll_rate, pitch_rate, yaw_rate, stall, _aileron, _elevator, _rudder, _trim_roll, _trim_pitch, _trim_yaw)) = flight_data {
                            ui.label(format!("Throttle: {:.0}%", throttle * 100.0));
                            ui.label(format!("Speed: {:.0} kt", speedometer));
                            ui.label(format!("Altitude: {:.0}", altimeter));
                            ui.label(format!("Mach: {:.2}", mach));
                            ui.label(format!("G: {:.1}", g_meter));
                            ui.label(format!("Stall: {}", stall));
                            ui.separator();
                            // Plain "deg" rather than the "°" glyph - egui's
                            // own default embedded font doesn't guarantee
                            // that character renders (showed up blank/
                            // missing in testing), ASCII always will.
                            ui.label(format!("AoA X: {:.1} deg", aoa_x));
                            ui.label(format!("AoA Y: {:.1} deg", aoa_y));
                            ui.label(format!("AoA: {:.1} deg", aoa));
                            ui.separator();
                            ui.label(format!("Roll rate: {:.1} deg/s", roll_rate));
                            ui.label(format!("Pitch rate: {:.1} deg/s", pitch_rate));
                            ui.label(format!("Yaw rate: {:.1} deg/s", yaw_rate));
                        } else {
                            ui.label("Flight data: (no player)");
                        }
                    });

                // Stick position indicator - X = aileron (roll), Y = elevator
                // (pitch), rudder shown separately as a bar (no natural 2nd
                // axis to pair it with here). Drawn with egui's own raw
                // Painter rather than egui_plot - this isn't really a chart
                // (no axes/data series), just a circle + a dot, which
                // egui_plot has no particularly good primitive for.
                egui::Window::new("Stick Input (F7)")
                    .default_pos(egui::pos2(debug_info_default_x, 420.0))
                    .show(ctx, |ui| {
                        if let Some((.., aileron, elevator, rudder, trim_roll, trim_pitch, trim_yaw)) = flight_data {
                            ui.label("X = aileron, Y = elevator. Circle = full deflection.");
                            ui.label("Red = commanded stick. Yellow ring = trim only.");
                            let size = egui::vec2(200.0, 200.0);
                            let (response, painter) = ui.allocate_painter(size, egui::Sense::hover());
                            let rect = response.rect;
                            let center = rect.center();
                            let radius = rect.width().min(rect.height()) * 0.5 - 4.0;

                            painter.circle_stroke(center, radius, egui::Stroke::new(1.5, egui::Color32::GRAY));
                            painter.line_segment([egui::pos2(center.x - radius, center.y), egui::pos2(center.x + radius, center.y)], egui::Stroke::new(1.0, egui::Color32::DARK_GRAY));
                            painter.line_segment([egui::pos2(center.x, center.y - radius), egui::pos2(center.x, center.y + radius)], egui::Stroke::new(1.0, egui::Color32::DARK_GRAY));

                            // Screen Y grows downward, so elevator is negated
                            // here to make "nose up" (positive elevator, by
                            // this game's own convention) plot upward on
                            // screen, matching how a real stick display reads
                            // - flip this back if elevator's own sign turns
                            // out to mean the opposite in this game.
                            let stick_pos = egui::pos2(
                                center.x + aileron.clamp(-1.0, 1.0) * radius,
                                center.y - elevator.clamp(-1.0, 1.0) * radius,
                            );
                            painter.circle_filled(stick_pos, 6.0, egui::Color32::from_rgb(255, 80, 80));

                            // Trim-only marker - where "elevator"/"aileron"
                            // above would sit from trim alone, ignoring
                            // whatever the pilot's stick is doing right now.
                            // Plotted directly from trim_roll/trim_pitch, not
                            // from `elevator`/`aileron` (those already have
                            // pitch trim folded in as of this session - see
                            // Plane::update's own comment - so re-deriving
                            // trim's contribution from them isn't possible
                            // once the stick is off-center; trim_roll/
                            // trim_pitch are the raw Trim values instead).
                            // Same sign-of-elevator caveat as the stick dot
                            // above.
                            let trim_pos = egui::pos2(
                                center.x + trim_roll.clamp(-1.0, 1.0) * radius,
                                center.y - trim_pitch.clamp(-1.0, 1.0) * radius,
                            );
                            painter.circle_stroke(trim_pos, 6.0, egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 220, 60)));

                            ui.add_space(4.0);
                            ui.label(format!("Aileron: {:.2}  Elevator: {:.2}  Rudder: {:.2}", aileron, elevator, rudder));
                            ui.label(format!("Trim roll: {:.2}  Trim pitch: {:.2}", trim_roll, trim_pitch));

                            // Yaw trim has no natural 2nd axis to share the
                            // circle with (same reason rudder itself isn't
                            // plotted there either - see this block's own
                            // top comment), so it gets its own bar here
                            // instead. `flight_data` is a plain snapshot
                            // gathered before this closure (can't borrow
                            // `self` again inside it - see where it's built,
                            // above), not a live handle into Plane's own
                            // Trim, so this can't actually be dragged to set
                            // yaw trim the way a normal Slider would suggest
                            // - disabled to read as the display-only gauge
                            // it is, matching everything else in this
                            // window. Say the word if you want it wired up
                            // to actually set trim.yaw instead.
                            ui.add_space(4.0);
                            let mut trim_yaw_display = trim_yaw;
                            ui.add_enabled(false, egui::Slider::new(&mut trim_yaw_display, -1.0..=1.0).text("Yaw trim"));
                        } else {
                            ui.label("(no player)");
                        }
                    });

                // Positioned clearly right of "Debug Info" (which sits at
                // x=20 above) rather than the other way around, per request.
                egui::Window::new("Aero Debug - Roll Authority Gain (F7)")
                    .default_pos(egui::pos2(aero_chart_default_x, 420.0))
                    .show(ctx, |ui| {
                        ui.label("Line = theoretical max roll rate (AoA=0, sea level). Dots = recent flight trail (actual measured rate).");
                        egui_plot::Plot::new("roll_rate_live_plot")
                            .height(320.0)
                            .width(520.0)
                            .x_axis_label("Airspeed (knots)")
                            .y_axis_label("Roll rate (deg/s)")
                            .x_axis_formatter(|mark, _range| format!("{:.0}", mark.value))
                            .y_axis_formatter(|mark, _range| format!("{:.0}", mark.value))
                            // Fixed spacing rather than egui_plot's own
                            // automatic tick-spacing algorithm on either
                            // axis - that algorithm picks spacing based on
                            // how much pixel width/height is actually
                            // available, and appears to have been collapsing
                            // to just the endpoint ticks when the plot ended
                            // up smaller than expected. This guarantees
                            // ticks at 0/100/.../800 knots and
                            // 0/25/50/.../deg/s regardless of that (25 rather
                            // than a round 50 so RollRateParams::default's
                            // own 260 deg/s peak lands on a tick, not between
                            // two).
                            .x_grid_spacer(egui_plot::uniform_grid_spacer(|_input| [100.0, 100.0, 100.0]))
                            .y_grid_spacer(egui_plot::uniform_grid_spacer(|_input| [25.0, 25.0, 25.0]))
                            .legend(egui_plot::Legend::default())
                            .show(ui, |plot_ui| {
                                plot_ui.line(egui_plot::Line::new("Theoretical (AoA=0)", egui_plot::PlotPoints::from(curve.clone())));
                                plot_ui.points(egui_plot::Points::new("Recent flight", egui_plot::PlotPoints::from(trail.clone())).radius(2.0));
                            });
                    });

                // Live wing lift force - see App::wing_lift_trail's own doc
                // comment. Positioned below "Aero Debug" (same x, height
                // 420+320+~40 padding below that one's own y=420 start).
                egui::Window::new("Wing Lift Forces (F7)")
                    .default_pos(egui::pos2(aero_chart_default_x, 780.0))
                    .show(ctx, |ui| {
                        ui.label("Vertical (Y) lift force, Left wing (main) and Right elevator wing, most recent samples.");
                        egui_plot::Plot::new("wing_lift_live_plot")
                            .height(240.0)
                            .width(520.0)
                            .x_axis_label("Sample")
                            .y_axis_label("Lift force Y (N)")
                            .x_axis_formatter(|mark, _range| format!("{:.0}", mark.value))
                            .y_axis_formatter(|mark, _range| format!("{:.0}", mark.value))
                            .legend(egui_plot::Legend::default())
                            .show(ui, |plot_ui| {
                                plot_ui.line(egui_plot::Line::new("Main wing (Left)", egui_plot::PlotPoints::from(main_wing_lift_trail.clone())));
                                plot_ui.line(egui_plot::Line::new("Elevator wing (Right)", egui_plot::PlotPoints::from(elevator_wing_lift_trail.clone())));
                            });
                    });
            },
        );

        // Apply the Camera Editor window's button presses (couldn't touch
        // `self` from inside the build_ui closure above).
        if camera_offset_delta != Vector3::zeros() || camera_set_active.is_some() || camera_reset_offset {
            if let Some(cam) = self.scene_manager.content_mut()
                .and_then(|content| content.nodes.get_mut("camera"))
                .and_then(|node| node.get_behavior_mut::<Camera>())
            {
                if camera_reset_offset {
                    cam.debug_offset = Vector3::zeros();
                }
                cam.debug_offset += camera_offset_delta;
                if let Some(v) = camera_set_active {
                    cam.debug_mode_active = v;
                }
                // A nudge is meaningless unless the editor offset is actually
                // being applied - turn it on.
                if camera_offset_delta != Vector3::zeros() {
                    cam.debug_mode_active = true;
                }
            }
        }
    }

    // Debug lines come from the physics thread in absolute world coordinates,
    // so make them camera-relative here to match camera.view_proj.
    fn render_physics_debug_lines<'rp>(&mut self, render_pass: &mut wgpu::RenderPass<'rp>) {
        let camera_position = self.scene_manager.cameras().map(|c| c.active().camera.position()).unwrap_or_else(nalgebra::Point3::origin);
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
        self.render_water_pass(encoder);
        self.render_transparent_pass(encoder);
        // Blits scene_color onto the real swapchain view, so the scene still looks
        // normal everywhere - see BlurRender::render. Has to run after the 3D
        // passes above (nothing to blit yet otherwise) and before the UI pass
        // below, which samples scene_color directly (at whatever per-node radius,
        // see text_shader.wgsl) for any node with background_blur set.
        self.renderer.blur.render(encoder, view);
        self.render_ui_pass(encoder, view);
        // Drawn last so it's always on top of the game's own HUD too.
        self.render_aero_debug_overlay_pass(encoder, view);
    }
}
