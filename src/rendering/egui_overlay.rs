//! The game's debug windows - egui, drawn inside the engine's own window.
//!
//! The engine only runs it: the game decides what's in it, from its scene's
//! `SceneBehaviour::debug_ui`:
//!
//!     fn debug_ui(&mut self, ctx: &egui::Context, scene: &mut Scene, app: &mut App) {
//!         egui::Window::new("Player").show(ctx, |ui| {
//!             ui.label(format!("speed: {:.1}", self.speed));
//!         });
//!     }
//!
//! Hidden until the game sets `visible` - from whatever key or button it
//! chooses; the engine binds none. While it's up the mouse is free to click
//! it, and goes back to how it was when it hides.
//!
//! Each frame App::run builds the windows (`run`, after the scene's update)
//! and the render draws them over everything (`paint`).
//!
//! egui draws into this same SDL2/wgpu window rather than a separate native
//! one - a separate `eframe` window reliably hung during its own startup when
//! opened from a process that had already used an SDL2 window.

use sdl2::keyboard::Keycode;

use crate::input::input;
use crate::window::window::CursorIcon;

pub struct DebugUi {
    pub ctx: egui::Context,
    /// Whether the debug windows are drawn (and the scene's `debug_ui` runs).
    pub visible: bool,
    renderer: egui_wgpu::Renderer,
    /// This frame's windows, built by `run`, waiting for `paint`.
    pending: Option<egui::FullOutput>,
    /// Whether the mouse was locked before the debug windows freed it - put
    /// back when they hide.
    cursor_was_locked: Option<bool>,
    /// The mouse cursor egui asked for last (a hand over a button, a text
    /// beam over a text field...) - App::apply_cursor shows it while the
    /// mouse is over a debug window.
    pub(crate) cursor: CursorIcon,
    /// When it started, and when it last ran - egui gets the real time and
    /// frame length from these.
    started: std::time::Instant,
    last_frame: Option<std::time::Instant>,
}

impl DebugUi {
    pub fn new(device: &wgpu::Device, output_color_format: wgpu::TextureFormat) -> Self {
        Self {
            ctx: egui::Context::default(),
            visible: false,
            renderer: egui_wgpu::Renderer::new(device, output_color_format, None, 1, false),
            pending: None,
            cursor_was_locked: None,
            cursor: CursorIcon::Default,
            started: std::time::Instant::now(),
            last_frame: None,
        }
    }

    /// Frees the mouse while the windows are up, and puts it back as it was
    /// once they hide. Returns the lock state the window should have, when it
    /// changes.
    /// `shown`: whether any debug window is up (this one, or the engine's -
    /// see App::debug_windows_shown).
    pub(crate) fn cursor_lock_change(&mut self, shown: bool, locked_now: bool) -> Option<bool> {
        match (shown, self.cursor_was_locked) {
            (true, None) => {
                self.cursor_was_locked = Some(locked_now);
                locked_now.then_some(false)
            }
            (false, Some(was_locked)) => {
                self.cursor_was_locked = None;
                (was_locked != locked_now).then_some(was_locked)
            }
            _ => None,
        }
    }

    /// This frame's input, as egui wants it - `screen_size` is the window
    /// in points (the space the mouse is reported in).
    pub(crate) fn raw_input(&self, screen_size: [u32; 2]) -> egui::RawInput {
        let pointer = egui::pos2(input::mouse_x() as f32, input::mouse_y() as f32);
        let (shift, ctrl) = (input::shift_down(), input::ctrl_down());
        let modifiers = egui::Modifiers { shift, ctrl, command: ctrl, ..Default::default() };

        let mut raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(screen_size[0] as f32, screen_size[1] as f32))),
            modifiers,
            ..Default::default()
        };
        raw_input.events.push(egui::Event::PointerMoved(pointer));
        let scroll = input::mouse_scroll_y();
        if scroll != 0.0 {
            raw_input.events.push(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Line, delta: egui::vec2(0.0, scroll), modifiers });
        }
        // Every press and release as it happened, where it happened - not the
        // button's state sampled once a frame, which misses a click that goes
        // down and up between two frames.
        for event in input::mouse_button_events() {
            let button = match event.button {
                sdl2::mouse::MouseButton::Left => egui::PointerButton::Primary,
                sdl2::mouse::MouseButton::Right => egui::PointerButton::Secondary,
                sdl2::mouse::MouseButton::Middle => egui::PointerButton::Middle,
                _ => continue,
            };
            let pos = egui::pos2(event.x as f32, event.y as f32);
            raw_input.events.push(egui::Event::PointerMoved(pos));
            raw_input.events.push(egui::Event::PointerButton { pos, button, pressed: event.pressed, modifiers });
        }
        raw_input.events.push(egui::Event::PointerMoved(pointer));
        raw_input.events.extend(input::typed_input().iter().filter_map(typing_event));

        // The real time and frame length. Without them egui assumes 1/60 s a
        // frame - at a few hundred frames a second its clock runs several
        // times too fast, and a normal click looks held too long to count.
        let now = std::time::Instant::now();
        raw_input.time = Some(now.duration_since(self.started).as_secs_f64());
        if let Some(last) = self.last_frame {
            raw_input.predicted_dt = now.duration_since(last).as_secs_f32().clamp(1.0 / 1000.0, 0.25);
        }
        raw_input
    }

    /// Keeps this frame's windows (built from `ctx.run`) for `paint`.
    pub(crate) fn set_frame(&mut self, output: egui::FullOutput) {
        self.last_frame = Some(std::time::Instant::now());
        self.cursor = cursor_icon(output.platform_output.cursor_icon);
        // Copying in a debug window (a text field, a Copy button) - onto the
        // system clipboard.
        for command in &output.platform_output.commands {
            if let egui::OutputCommand::CopyText(text) = command {
                crate::input::clipboard::set_clipboard(text);
            }
        }
        self.pending = Some(output);
    }

    /// Draws this frame's windows (if `run` built any) over `view` - a Load
    /// (not Clear) pass, over everything else in the frame.
    pub(crate) fn paint(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        // The real size of `view` in pixels - on a HiDPI/Retina display not
        // the window's size in points (that's what egui lays out in).
        screen_size_px: [u32; 2],
    ) {
        let Some(mut output) = self.pending.take() else { return };
        let shapes = std::mem::take(&mut output.shapes);
        let clipped_primitives = self.ctx.tessellate(shapes, output.pixels_per_point);

        for (id, image_delta) in &output.textures_delta.set {
            self.renderer.update_texture(device, queue, *id, image_delta);
        }

        let screen_descriptor = egui_wgpu::ScreenDescriptor {
            size_in_pixels: screen_size_px,
            pixels_per_point: output.pixels_per_point,
        };
        self.renderer.update_buffers(device, queue, encoder, &clipped_primitives, &screen_descriptor);

        {
            // egui_wgpu::Renderer::render wants a 'static RenderPass (its
            // paint callbacks can stash arbitrary boxed state) -
            // forget_lifetime() converts this one, still only used within
            // this block, into that shape.
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("debug ui pass"),
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

        for id in &output.textures_delta.free {
            self.renderer.free_texture(id);
        }
    }
}

/// egui's cursor as the engine's - the closest one.
fn cursor_icon(icon: egui::CursorIcon) -> CursorIcon {
    match icon {
        egui::CursorIcon::PointingHand => CursorIcon::Pointer,
        egui::CursorIcon::Text | egui::CursorIcon::VerticalText => CursorIcon::Text,
        egui::CursorIcon::Move | egui::CursorIcon::Grab | egui::CursorIcon::Grabbing | egui::CursorIcon::AllScroll => CursorIcon::Move,
        egui::CursorIcon::ResizeHorizontal | egui::CursorIcon::ResizeEast | egui::CursorIcon::ResizeWest | egui::CursorIcon::ResizeColumn => CursorIcon::ResizeHorizontal,
        egui::CursorIcon::ResizeVertical | egui::CursorIcon::ResizeNorth | egui::CursorIcon::ResizeSouth | egui::CursorIcon::ResizeRow => CursorIcon::ResizeVertical,
        egui::CursorIcon::ResizeNeSw | egui::CursorIcon::ResizeNorthEast | egui::CursorIcon::ResizeSouthWest => CursorIcon::ResizeNeSw,
        egui::CursorIcon::ResizeNwSe | egui::CursorIcon::ResizeNorthWest | egui::CursorIcon::ResizeSouthEast => CursorIcon::ResizeNwSe,
        egui::CursorIcon::NotAllowed | egui::CursorIcon::NoDrop => CursorIcon::NotAllowed,
        egui::CursorIcon::Wait | egui::CursorIcon::Progress => CursorIcon::Wait,
        _ => CursorIcon::Default,
    }
}

/// SDL typing (see input::TypedInput) as egui input - the characters, and
/// the keys a text field edits with. Anything else is left out.
fn typing_event(typed: &input::TypedInput) -> Option<egui::Event> {
    match typed {
        input::TypedInput::Text(text) => Some(egui::Event::Text(text.clone())),
        input::TypedInput::Key { key, pressed, repeat, ctrl, shift } => {
            let key = match *key {
                Keycode::Backspace => egui::Key::Backspace,
                Keycode::Delete => egui::Key::Delete,
                Keycode::Left => egui::Key::ArrowLeft,
                Keycode::Right => egui::Key::ArrowRight,
                Keycode::Up => egui::Key::ArrowUp,
                Keycode::Down => egui::Key::ArrowDown,
                Keycode::Home => egui::Key::Home,
                Keycode::End => egui::Key::End,
                Keycode::Return | Keycode::KpEnter => egui::Key::Enter,
                Keycode::Tab => egui::Key::Tab,
                Keycode::Escape => egui::Key::Escape,
                Keycode::A => egui::Key::A,
                Keycode::C => egui::Key::C,
                Keycode::V => egui::Key::V,
                Keycode::X => egui::Key::X,
                Keycode::Z => egui::Key::Z,
                _ => return None,
            };
            let modifiers = egui::Modifiers { ctrl: *ctrl, command: *ctrl, shift: *shift, ..Default::default() };
            Some(egui::Event::Key { key, physical_key: None, pressed: *pressed, repeat: *repeat, modifiers })
        }
    }
}
