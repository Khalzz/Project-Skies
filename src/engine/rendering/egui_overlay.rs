//! Generic egui-into-wgpu plumbing - deliberately knows nothing about what
//! it's drawing (no game-specific types), just how to run one egui frame
//! and paint its output into an existing render target. Game code supplies
//! the actual UI content via a closure (see `render`'s own doc comment) and
//! this frame's input via `RawInput` (see e.g. play::scene's own
//! aero-debug-overlay code for how that gets built from engine::input's
//! already-tracked mouse state).
//!
//! Exists specifically so a debug overlay can render INSIDE this game's own
//! SDL2/wgpu window - a separate native `eframe` window was tried first and
//! reliably hung during its own startup when opened from a process that had
//! already used and torn down an SDL2 window (see the conversation this
//! came out of); this sidesteps that entirely since there's only ever one
//! window/event loop (SDL2's), egui just contributes draw commands into the
//! same frame the game itself is already rendering.

pub struct EguiOverlay {
    pub ctx: egui::Context,
    renderer: egui_wgpu::Renderer,
}

impl EguiOverlay {
    pub fn new(device: &wgpu::Device, output_color_format: wgpu::TextureFormat) -> Self {
        Self {
            ctx: egui::Context::default(),
            renderer: egui_wgpu::Renderer::new(device, output_color_format, None, 1, false),
        }
    }

    /// Runs one egui frame (`build_ui` is where the actual UI gets built,
    /// e.g. `|ctx| { egui::Window::new(...).show(ctx, |ui| { ... }); }`) and
    /// immediately paints its output into `view` - a Load (not Clear) pass,
    /// same as every other pass drawn on top of the existing frame (see
    /// render_pass.rs's own render_ui_pass for the pattern this matches).
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        // Physical/drawable pixel count (e.g. App::window_manager.pixel_size)
        // - the real size of `view`'s own backing texture, NOT the logical
        // window size raw_input.screen_rect is built from. On a HiDPI/Retina
        // display these differ by pixels_per_point below; passing the wrong
        // one here confines/shrinks the whole UI into a corner instead of
        // filling the actual window (see the conversation this came out of).
        screen_size_px: [u32; 2],
        // physical_px / logical_px - same ratio App::run's own UI macro
        // already computes as `dpi_scale` for the game's own glyphon-based
        // UI. Without this, egui defaults to 1.0 and lays its "points" out
        // 1:1 against physical pixels instead of scaling up to them.
        pixels_per_point: f32,
        raw_input: egui::RawInput,
        build_ui: impl FnMut(&egui::Context),
    ) {
        self.ctx.set_pixels_per_point(pixels_per_point);
        let full_output = self.ctx.run(raw_input, build_ui);
        let clipped_primitives = self.ctx.tessellate(full_output.shapes, full_output.pixels_per_point);

        for (id, image_delta) in &full_output.textures_delta.set {
            self.renderer.update_texture(device, queue, *id, image_delta);
        }

        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: screen_size_px,
            pixels_per_point: full_output.pixels_per_point,
        };

        self.renderer.update_buffers(device, queue, encoder, &clipped_primitives, &screen_descriptor);

        {
            // egui_wgpu::Renderer::render wants a 'static RenderPass (its
            // paint callbacks can stash arbitrary boxed state) -
            // forget_lifetime() converts this one, still only actually used
            // within this same block, into that shape.
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui overlay pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            }).forget_lifetime();
            self.renderer.render(&mut render_pass, &clipped_primitives, &screen_descriptor);
        }

        for id in &full_output.textures_delta.free {
            self.renderer.free_texture(id);
        }
    }
}
