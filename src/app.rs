use std::collections::HashMap;
use std::env;
use std::rc::Rc;
use std::time::Instant;

use glyphon::TextArea;
use nalgebra::Vector3;
use sdl2::EventPump;

use crate::assets::Assets;
use crate::audio::audio::Audio;
use crate::game_nodes::timing::Timing;
use crate::input::input;
use crate::particles::ParticleEmitters;
use crate::particles::renderer::{EmitterSource, ParticleFrameInfo};
use crate::physics::physics::{DebugPhysicsMessageType, PhysicsState};
use crate::physics::physics_handler::RenderMessage;
use crate::rendering::egui_overlay::DebugUi;
use crate::rendering::graphics_backend::GraphicsBackend;
use crate::rendering::enviroment::environment::PreparedEnvironment;
use crate::rendering::instance_management::InstanceRaw;
use crate::rendering::renderer::Renderer;
use crate::rendering::ui::ui::Ui;
use crate::scene_manager::node::properties::{Shape2D, ShapeKind};
use crate::rendering::shapes_2d::ShapeInstance;
use crate::scene_manager::scene_access::SceneChanges;
use crate::scene_manager::scene::{FrameContext, SceneManager};
use crate::settings::ProjectSettings;
use crate::ui::color::UiColor;
use crate::ui::ui_node::UiNode;
use crate::ui::ui_transform::PositionValue;
use crate::window::window::Window;

#[derive(Clone)]
pub struct AppState {
    pub is_running: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub width: u32,
    pub height: u32
}

/*
    # App

    The engine: one window, the systems that run for as long as the app does,
    and the scene currently open. Set it up, then hand control to `run`:

        let mut app = App::new().await?;
        app.window.set_title("My Game");
        app.settings.physics.gravity = Vector3::new(0.0, -9.81, 0.0);
        input::load_bindings("settings/input.ron")?;
        app.scene_manager.create_scene("level", Level::new);
        app.scene_manager.open_scene("level");
        app.run();

    - window: title, size, window mode, screen, mouse lock.
    - settings: the project's defaults (gravity, fixed timestep, environment,
      lighting, particles' wind/sea) - a scene overrides any of them for
      itself (see crate::settings).
    - assets: the assets folder - models are named by their path in it.
    - renderer: everything that draws.
    - scene_manager: the scenes, and the one open now.
    - physics: the open scene's physics thread, and what it last sent.
    - ui, audio, time (delta time, time scale), debug_ui (the game's egui
      debug windows).
    - Input is global, not here - see crate::input::input.
*/
pub struct App {
    /// The window - see `Window`.
    pub window: Window,
    /// Everything that draws - see `Renderer`.
    pub renderer: Renderer,
    /// The project's defaults - every scene uses them, unless it sets its own
    /// (see crate::settings).
    pub settings: ProjectSettings,
    /// The assets folder (see `Assets`).
    pub assets: Assets,
    pub scene_manager: SceneManager,
    pub ui: Ui,
    pub audio: Audio,
    /// Delta time and time scale - see `Timing`.
    pub time: Timing,
    /// The game's debug windows - see `DebugUi`.
    pub debug_ui: DebugUi,
    /// What a scene's loading is on right now (see LoadReporter) - shown by
    /// the loading screen, None when nothing's loading.
    pub loading_step: Option<String>,
    /// The open scene's physics thread, and what it last sent - see
    /// `PhysicsState`.
    pub physics: PhysicsState,
    /// Set true to quit from code that only has `&mut App` (a UI button's
    /// click handler, say) - `run` stops at the start of the next frame, the
    /// same way as closing the window (physics shut down, `run` returns).
    pub should_quit: bool,
    event_pump: Option<EventPump>,
}

impl App {
    /// The engine, with its window created but not shown yet: 800x600,
    /// windowed, resizable, titled "Pankarta Software" - change any of it
    /// through `app.window` before `run`, which shows it. Draws with whichever
    /// graphics API suits this machine best - see `new_with_backend`.
    pub async fn new() -> Result<App, String> {
        Self::new_with_backend(GraphicsBackend::Auto).await
    }

    /// `new`, drawing with `backend` (see GraphicsBackend) - an error when
    /// this machine can't use it. It's fixed for the app's life: changing it
    /// means starting again (`GraphicsBackend::available` lists the choices).
    pub async fn new_with_backend(backend: GraphicsBackend) -> Result<App, String> {
        // Read when SDL starts - a fullscreen window stays up when it loses focus.
        env::set_var("SDL_VIDEO_MINIMIZE_ON_FOCUS_LOSS", "0");
        let sdl = sdl2::init()?;
        let window = Window::new(sdl.video()?)?;
        input::init(sdl.game_controller()?, sdl.joystick()?);
        // SDL stays up as long as its subsystems do - nothing else keeps `sdl`.
        let event_pump = sdl.event_pump()?;

        // Asked now, before the device exists, and kept - see
        // GraphicsBackend::available for why not later.
        GraphicsBackend::available();
        let renderer = Renderer::new(&window, backend).await?;

        // Its boxes and images are drawn with Ui shaders from the library.
        let ui = Ui::new(
            &renderer.device,
            &renderer.queue,
            &renderer.config,
            &renderer.glyphon.cache,
            &renderer.blur,
            renderer.shaders.overlay_layout(crate::rendering::shader_library::Overlay::UiBox),
        );
        let assets = Assets::new(renderer.shaders.overlay_layout(crate::rendering::shader_library::Overlay::UiImage).clone());
        let debug_ui = DebugUi::new(&renderer.device, renderer.config.format);

        Ok(App {
            window,
            renderer,
            settings: ProjectSettings::default(),
            assets,
            scene_manager: SceneManager::new(),
            ui,
            audio: Audio::new(),
            time: Timing::new(),
            debug_ui,
            loading_step: None,
            physics: PhysicsState::new(),
            should_quit: false,
            event_pump: Some(event_pump),
        })
    }

    /// Resizes the renderer and the UI to the window when its size has
    /// changed - by the game (`app.window.set_size`, `set_window_mode`...) or
    /// the user (dragging it, maximizing it). Every frame, before anything's
    /// drawn.
    fn sync_window_size(&mut self) {
        let (width, height) = self.window.sdl_window().drawable_size();
        if width != self.renderer.config.width || height != self.renderer.config.height {
            self.resize();
        } else if self.ui.update_scale(self.window.size.width, self.window.size.height) {
            // The UI's scaling changed (app.ui.scaling / scale), the window didn't.
            self.relayout_ui();
        }
    }

    /// Lays the UI out again in its current room (Ui::size) - every node's
    /// Percent sizes and Center/End positions were worked out against the
    /// old one (see UiNode::resolve).
    fn relayout_ui(&mut self) {
        let (ui_width, ui_height) = self.ui.size();
        for node in self.ui.renderizable_elements.values_mut() {
            node.resolve(ui_width, ui_height);
        }
        self.ui.has_changed = true;
    }

    /// The UI's room as a `Size` (UI units, rounded) - what its layout pass
    /// turns positions into screen space with.
    fn ui_layout_size(&self) -> Size {
        let (width, height) = self.ui.size();
        Size { width: width.round().max(1.0) as u32, height: height.round().max(1.0) as u32 }
    }

    pub(crate) fn resize(&mut self) {
        self.window.refresh_size();
        // Real backing pixels (see Window::pixel_size) - these size actual GPU
        // render targets/aspect ratio, which have to match the surface's real
        // resolution, not the window's points size. UI layout/hit-testing
        // stays on window.size (points) - see that field's own doc comment.
        let width = self.window.pixel_size.width;
        let height = self.window.pixel_size.height;
        // Minimized - a 0-sized surface can't be configured. Restoring it
        // changes the size again, which lands back here.
        if width == 0 || height == 0 {
            return;
        }

        self.renderer.resize(width, height);
        if let Some(cameras) = self.scene_manager.cameras_mut() {
            cameras.resize(width, height);
        }
        // A scene waiting for its assets has cameras too.
        if let Some(pending) = self.scene_manager.loading.as_mut() {
            pending.scene.cameras.resize(width, height);
        }
        self.ui.resize_blur_binding(&self.renderer.device, &self.renderer.queue, &self.renderer.blur, width, height);

        // The UI's scale follows the window's size (see UiScaling), and its
        // layout the new room.
        self.ui.update_scale(self.window.size.width, self.window.size.height);
        self.relayout_ui();
    }

    /// The environment drawn now - the active scene's own, or the project's.
    pub(crate) fn current_environment(&self) -> &PreparedEnvironment {
        self.scene_manager.active_scene.as_ref()
            .and_then(|scene| scene.environment.as_ref())
            .map(|(_, prepared)| prepared)
            .unwrap_or(&self.renderer.project_environment)
    }

    fn current_environment_mut(&mut self) -> &mut PreparedEnvironment {
        match self.scene_manager.active_scene.as_mut().and_then(|scene| scene.environment.as_mut()) {
            Some((_, prepared)) => prepared,
            None => &mut self.renderer.project_environment,
        }
    }

    /// Applies the settings the active scene runs with now (its own, or the
    /// project's) wherever they changed since last frame: builds a changed
    /// environment again, sends a changed gravity or time scale to the
    /// physics thread, and fills the light buffer from the lighting.
    fn apply_settings(&mut self, camera_position: Vector3<f32>) {
        if self.settings.environment != self.renderer.project_environment_source {
            self.renderer.project_environment = self.renderer.prepare_environment(self.settings.environment.clone());
            self.renderer.project_environment_source = self.settings.environment.clone();
        }
        if let Some(scene) = self.scene_manager.active_scene.as_mut() {
            let current = scene.environment.as_ref().map(|(source, _)| source);
            if scene.settings.environment.as_ref() != current {
                scene.environment = scene.settings.environment.clone()
                    .map(|environment| (environment.clone(), self.renderer.prepare_environment(environment)));
            }
        }

        let scene_settings = self.scene_manager.active_scene.as_ref().map(|scene| &scene.settings);
        let gravity = scene_settings.map_or(self.settings.physics.gravity, |settings| settings.gravity(&self.settings));
        let lighting = scene_settings.map_or(self.settings.lighting, |settings| settings.lighting(&self.settings));

        self.physics.sync(gravity, self.time.time_scale);

        // The sun is infinitely far away - its direction is placed that far
        // along in the shaders' camera-relative space, so it's one fixed
        // direction for every surface on screen. The sky's sun disc follows
        // the same direction, so disc, lighting and glint all agree.
        const SUN_DISTANCE: f32 = 1.0e9;
        let sun_direction = lighting.sun_direction.try_normalize(1e-6).unwrap_or_else(Vector3::y);
        let light_position = sun_direction * SUN_DISTANCE;
        self.renderer.light.uniform.position = [light_position.x, light_position.y, light_position.z];
        self.renderer.light.uniform.color = lighting.sun_color;
        self.renderer.light.uniform.time = self.time.shader_time();
        self.renderer.light.uniform.camera_position = camera_position.into();
        self.renderer.queue.write_buffer(&self.renderer.light.rendering_data.buffer, 0, bytemuck::cast_slice(&[self.renderer.light.uniform]));
        let queue = self.renderer.queue.clone();
        if let Some(skybox) = self.current_environment_mut().skybox.as_mut() {
            skybox.set_sun_direction(&queue, sun_direction);
        }
    }

    /// Feeds every node's `ParticleEmitters` (at wherever the node is being
    /// drawn this frame) to the particle renderer - see ParticleRenderer::
    /// update. Once per frame, after the camera has moved.
    fn update_particles(&mut self) {
        let Some(cameras) = self.scene_manager.cameras() else { return };
        if !cameras.has_active_camera() {
            return;
        }
        let active = cameras.active();
        let camera_position = active.camera.position().coords;
        let particle_settings = self.scene_manager.active_scene.as_ref()
            .map_or(self.settings.particles, |scene| scene.settings.particles(&self.settings));
        let light = &self.renderer.light.uniform;
        let frame = ParticleFrameInfo {
            camera_position,
            camera_view: active.camera.calc_matrix(),
            near: active.projection.znear,
            delta_time: self.time.delta_time,
            // No sea: far below anything a particle reaches.
            sea_level: particle_settings.sea_level.unwrap_or(-1.0e9),
            wind: particle_settings.wind,
            sun_direction: Vector3::from(light.position),
            sun_color: Vector3::from(light.color),
        };

        let Some(content) = self.scene_manager.content() else { return };
        let sources = content.nodes.iter().flat_map(|node| {
            let emitters = node.get_property::<ParticleEmitters>();
            // Where the node is being drawn - its render instance if it has
            // one (moved by physics), otherwise its own Transform3D.
            let pose = content.renderizable_instances.get(&node.id)
                .map(|instance| (instance.instance.transform.position, instance.instance.transform.rotation))
                .or_else(|| node.transform_3d().map(|transform| (transform.position, transform.rotation)));
            emitters.zip(pose).into_iter().flat_map(move |(emitters, (position, rotation))| {
                emitters.iter().map(move |(name, emitter)| EmitterSource { node_id: &node.id, name, emitter, position, rotation })
            })
        });
        self.renderer.particles.update(&self.renderer.device, &self.renderer.queue, &frame, sources);
    }

    /// Hands the 2D nodes' shapes (see `Shape2D`) to the renderer, in the
    /// order they're drawn: by z index (the shape's own, or its node's),
    /// then by when each was put on - the later in front. Their units are
    /// the UI's (see `Transform2D`), so they're turned into pixels the way
    /// the UI's are.
    fn prepare_nodes_2d(&mut self) {
        let mut shapes: Vec<((i32, u64), ShapeInstance)> = Vec::new();
        if let Some(content) = self.scene_manager.content() {
            let pixels_per_unit = self.window.pixel_size.width as f32 / self.ui_layout_size().width as f32;
            for node in content.nodes.iter() {
                let (Some(transform), Some(shape)) = (node.transform_2d(), node.get_property::<Shape2D>()) else { continue };
                let (center, rotation) = shape.offset.place(transform);
                let (half_size, kind) = match shape.kind {
                    ShapeKind::Rect { width, height } => ([width / 2.0 * transform.scale.x, height / 2.0 * transform.scale.y], ShapeInstance::RECT),
                    ShapeKind::Circle { radius } => {
                        let radius = radius * transform.scale.x.max(transform.scale.y);
                        ([radius, radius], ShapeInstance::CIRCLE)
                    }
                };
                let (sin, cos) = rotation.sin_cos();
                let order = (shape.z_index.unwrap_or(transform.z_index), node.property_order::<Shape2D>().unwrap_or(0));
                shapes.push((order, ShapeInstance {
                    center: [center.x * pixels_per_unit, center.y * pixels_per_unit],
                    half_size: half_size.map(|half| half * pixels_per_unit),
                    rotation: [cos, sin],
                    kind: [kind, 0.0],
                    color: shape.color.to_rgba_f32(),
                }));
            }
        }
        shapes.sort_by_key(|(order, _)| *order);
        let shapes: Vec<ShapeInstance> = shapes.into_iter().map(|(_, shape)| shape).collect();
        let screen_px = (self.renderer.config.width, self.renderer.config.height);
        self.renderer.shapes_2d.set(&self.renderer.device, &self.renderer.queue, &shapes, screen_px);
    }

    /// Registers a line set for the frame plan's `Lines` steps - see
    /// engine::rendering::render_sets. `screen_space`: in screen pixels, not
    /// the world.
    pub fn add_line_set(&mut self, name: &str, screen_space: bool) {
        let mut lines = crate::rendering::thick_lines::ThickLines::new(&self.renderer.device, self.renderer.shaders.overlay_layout(crate::rendering::shader_library::Overlay::Lines));
        lines.screen_space = screen_space;
        self.renderer.render_sets.insert_lines(name, lines);
    }

    /// Registers an SDF text set for the frame plan's `Text` steps (see
    /// `add_line_set`).
    pub fn add_text_set(&mut self, name: &str, screen_space: bool) {
        let mut text = crate::rendering::sdf_text::SdfText::new(&self.renderer.device, self.renderer.config.format, self.renderer.shaders.overlay_layout(crate::rendering::shader_library::Overlay::Text));
        text.screen_space = screen_space;
        self.renderer.render_sets.insert_text(name, text);
    }

    pub(crate) fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        self.sync_no_camera_message();
        let ui_height = self.ui.size().1;
        crate::ui::widgets::toast::update_toasts(&mut self.ui, self.time.unscaled_delta_time, ui_height);
        self.prepare_ui_content();
        self.prepare_nodes_2d();
        self.apply_cursor();
        self.fire_ui_click_handlers();

        // WGPU - getting the screen's next image can wait on the GPU or vsync.
        let output = self.renderer.surface.get_current_texture()?;

        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.renderer.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });

        self.render_scene_passes(&mut encoder, &output.texture, &view);
        // The game's debug windows, over everything (see DebugUi) - only
        // built on the frames they're up.
        let screen_size_px = [self.renderer.config.width, self.renderer.config.height];
        self.debug_ui.paint(&self.renderer.device, &self.renderer.queue, &mut encoder, &view, screen_size_px);

        self.renderer.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }

    // Reserved UI key for the message below - never something a scene should
    // add/remove itself.
    const NO_CAMERA_MESSAGE_KEY: &str = "__no_camera_message";

    // Ensures/removes the "Add a camera to the scene" label to match the
    // active scene's SceneCameras::has_active_camera() each frame -
    // render_pass.rs's own render_opaque_pass already clears to black and
    // skips every draw in that case (see its own comment); this is the other
    // half, the actual visible text, via the ordinary UI pass, which runs
    // unconditionally on top regardless of what the 3D passes did.
    fn sync_no_camera_message(&mut self) {
        // Suppressed while: a real camera exists; a heavy scene's background
        // load hasn't finished yet (LoadingScreenScene is legitimately
        // camera-less the whole time); or no scene
        // has ever been constructed yet at all (the very first frame, before
        // the first scene opens) - same "nothing to blame a scene for"
        // reasoning as the loading case.
        if self.scene_manager.cameras().is_some_and(|c| c.has_active_camera()) || self.scene_manager.loading.is_some() || self.scene_manager.active_scene.is_none() {
            if self.ui.renderizable_elements.remove(Self::NO_CAMERA_MESSAGE_KEY).is_some() {
                self.ui.has_changed = true;
            }
            return;
        }

        if self.ui.renderizable_elements.contains_key(Self::NO_CAMERA_MESSAGE_KEY) {
            return;
        }

        let (screen_width, screen_height) = self.ui.size();
        let mut label = UiNode::label(&mut self.ui.text.font_system, "Add a camera to the scene", Some(500.0), Some(40.0))
            .set_position(PositionValue::Center(0.0), PositionValue::Center(0.0))
            .set_text_color(UiColor::Rgb(255, 255, 255))
            // set_position alone only centers the label's own box on screen -
            // text inside it defaults to left-aligned (see ui_node.rs's
            // effective_style.align.unwrap_or(Align::Left)), which reads as
            // visibly off-center since the box (500px) is wider than the text.
            .set_align(glyphon::cosmic_text::Align::Center);
        label.resolve(screen_width, screen_height);
        self.ui.add_to_ui(Self::NO_CAMERA_MESSAGE_KEY.to_owned(), label);
        self.ui.has_changed = true;
    }

    // Rebuilds UI vertex/index/text buffers from the current node tree - every
    // frame while there's any UI: this pass is also where hover, press, clicks,
    // scrolling and style transitions are worked out, so it has to run for
    // them to respond. With no UI, only when something marked it changed (the
    // last node removed, say) - then it's cleared once and skipped after.
    fn prepare_ui_content(&mut self) {
        if !self.ui.has_changed && self.ui.renderizable_elements.is_empty() {
            self.ui.ui_rendering.hover_cursor = None;
            return;
        }

        // Two separate batches (see TextRendering::text_renderer_on_top's own doc
        // comment for why one shared Vec/one glyphon TextRenderer can't do this) -
        // main pass's text, and always_on_top nodes' text, prepared+rendered as
        // two independent draw calls so the latter actually lands above the
        // former instead of every node's text piling into one shared final pass.
        let mut text_areas: Vec<TextArea> = Vec::new();
        let mut text_areas_on_top: Vec<TextArea> = Vec::new();

        self.ui.ui_rendering.clear();

        // Taken out up front (rather than looked up in render order via repeated
        // get_mut calls) because this function's TextAreas borrow from inside
        // renderizable_elements for its own entire remaining duration (see the
        // comment near the bottom) - a second borrow of the map partway through
        // (which get_mut would be) conflicts with that. Same two-pass take/
        // restore shape App::fire_ui_click_handlers uses, for the same
        // underlying reason (something here needs to outlive a single map
        // borrow) - restored right before the map is touched again, after
        // text_areas has been fully consumed.
        let mut on_top_nodes: Vec<(String, UiNode)> = Vec::new();
        for key in &self.ui.always_on_top {
            if let Some(node) = self.ui.renderizable_elements.remove(key) {
                on_top_nodes.push((key.clone(), node));
            }
        }
        // Same take-out, for the opposite end - see Ui::always_on_bottom's own
        // doc comment.
        let mut on_bottom_nodes: Vec<(String, UiNode)> = Vec::new();
        for key in &self.ui.always_on_bottom {
            if let Some(node) = self.ui.renderizable_elements.remove(key) {
                on_bottom_nodes.push((key.clone(), node));
            }
        }
        // And Ui::overlay's (toasts) - drawn after the on-top ones.
        let mut overlay_nodes: Vec<(String, UiNode)> = Vec::new();
        for key in &self.ui.overlay {
            if let Some(node) = self.ui.renderizable_elements.remove(key) {
                overlay_nodes.push((key.clone(), node));
            }
        }

        // An active always_on_top node (a modal, currently) should block input to
        // literally everything else while it's up, the same way a real dialog
        // does - otherwise it's only visually on top (see Ui::always_on_top's own
        // doc comment for the rendering half of this), and clicks still fall
        // through to whatever button happens to sit at the same screen position
        // underneath. Among the always_on_top nodes themselves, the same goes
        // for each over the ones before it: only the last active one takes
        // input - a dropdown's open list (see widgets::dropdown) over the modal
        // it was opened from blocks that modal, until it closes.
        let blocks_input = on_top_nodes.iter().any(|(_, n)| n.is_active);

        // UI units -> real backing pixels: the display's own (see
        // Window::pixel_size's own doc comment) times the UI's scale (see
        // UiScaling) - node_content_preparation needs this to place glyphon
        // TextAreas and draw every box, since those work in real pixels
        // regardless of what space everything else here (UiTransform, mouse
        // position) is in. See Label::text_area's own doc comment. Worked out
        // from the same rounded room the layout uses, so the two always agree.
        let ui_size = self.ui_layout_size();
        let dpi_scale = self.window.pixel_size.width as f32 / ui_size.width as f32;

        // Debug bounds overlay (Ui::debug_bounds, no key) - has to run before node_content_preparation
        // below, not after: that call's returned TextAreas keep *ui_node mutably
        // borrowed for as long as they're alive (all the way to text_areas being
        // consumed further down), so nothing else can borrow *ui_node again
        // afterward. Reading the rect from the start of this frame (one frame
        // stale for a node whose layout is still actively changing) instead of
        // after this frame's update is an invisible tradeoff for a debug-only
        // overlay. Shared by both loops below rather than a closure, since a
        // closure capturing &mut self.ui.ui_rendering would itself conflict with
        // the &mut self.ui.renderizable_elements/on_top_nodes iteration around it.
        macro_rules! prepare_node {
            ($ui_node:expr, $hit_testable:expr, $text_areas:expr) => {
                if self.ui.debug_bounds {
                    $ui_node.debug_bounds_preparation(&ui_size, dpi_scale, &mut self.ui.ui_rendering);
                }
                $ui_node.measure_fit();
                // None - top-level nodes start unclipped; a scrollable
                // container establishes its own clip for its descendants
                // further down the recursion (see node_content_preparation's
                // Container branch/clip_rect's own doc comment).
                let (textareas_to_merge, _vertices_to_add, _indices_to_add) = $ui_node.node_content_preparation(&ui_size, dpi_scale, &mut self.ui.ui_rendering, &mut self.ui.text.font_system, &mut self.ui.text.text_cache, self.time.unscaled_delta_time, $hit_testable, None, None, false);
                $text_areas.extend(textareas_to_merge);
            };
        }

        // always_on_bottom nodes first (see their own doc comment) - still subject
        // to the same input-blocking as the regular pool below, not the always-
        // hit-testable treatment on_top_nodes gets, since they're conceptually
        // part of the same "everything except the modal" layer, just ordered to
        // render first/behind within it.
        for (_key, ui_node) in &mut on_bottom_nodes {
            prepare_node!(ui_node, !blocks_input, text_areas);
        }
        for (_key, ui_node) in &mut self.ui.renderizable_elements {
            prepare_node!(ui_node, !blocks_input, text_areas);
        }
        // Everything up to here is the main pass - see UiRendering::
        // main_index_count's own doc comment for why this boundary is recorded,
        // not just the two loops' existence.
        self.ui.ui_rendering.main_index_count = self.ui.ui_rendering.num_indices;
        self.ui.ui_rendering.main_uses_blur = std::mem::take(&mut self.ui.ui_rendering.uses_blur);
        let topmost_active = on_top_nodes.iter().rposition(|(_, node)| node.is_active);
        for (index, (_key, ui_node)) in on_top_nodes.iter_mut().enumerate() {
            let hit_testable = topmost_active.map_or(true, |topmost| index >= topmost);
            prepare_node!(ui_node, hit_testable, text_areas_on_top);
        }
        for (_key, ui_node) in &mut overlay_nodes {
            prepare_node!(ui_node, true, text_areas_on_top);
        }

        // Only update buffers if we have data
        if !self.ui.ui_rendering.vertices.is_empty() {
            self.ui.ui_rendering.ensure_vertex_capacity(&self.renderer.device);
            self.renderer.queue.write_buffer(&self.ui.ui_rendering.vertex_buffer, 0, bytemuck::cast_slice(self.ui.ui_rendering.vertices.as_slice()));
        }
        if !self.ui.ui_rendering.indices.is_empty() {
            self.ui.ui_rendering.ensure_index_capacity(&self.renderer.device);
            self.renderer.queue.write_buffer(&self.ui.ui_rendering.index_buffer, 0, bytemuck::cast_slice(&self.ui.ui_rendering.indices));
        }

        // Always prepare, even with zero text areas - glyphon's prepare() is what
        // clears its *previous* batch (it's cheap/safe to call empty: it just
        // clears and returns, see glyphon::TextRenderer::render's own early-out
        // for an empty batch). Skipping this call when there's currently nothing
        // to show - e.g. the frame a modal closes, going from "had text" to
        // "none" - left whatever was prepared last frame sitting in the
        // renderer's buffer with nothing ever clearing it, so render() (called
        // unconditionally every frame) kept drawing stale text that should've
        // disappeared.
        self.ui.text.text_renderer.prepare(&self.renderer.device, &self.renderer.queue, &mut self.ui.text.font_system, &mut self.ui.text.text_atlas, &self.renderer.glyphon.viewport, text_areas, &mut self.ui.text.text_cache).unwrap();
        self.ui.text.text_renderer_on_top.prepare(&self.renderer.device, &self.renderer.queue, &mut self.ui.text.font_system, &mut self.ui.text.text_atlas, &self.renderer.glyphon.viewport, text_areas_on_top, &mut self.ui.text.text_cache).unwrap();

        // Puts always_on_top/always_on_bottom nodes back now that text_areas (the
        // last thing that held any borrow into them) is fully consumed - see
        // where they were taken out, above.
        for (key, node) in on_top_nodes {
            self.ui.renderizable_elements.insert(key, node);
        }
        for (key, node) in on_bottom_nodes {
            self.ui.renderizable_elements.insert(key, node);
        }
        for (key, node) in overlay_nodes {
            self.ui.renderizable_elements.insert(key, node);
        }

        // An image that turned up while the scene runs loads now. Not while
        // a scene loads - its images are on the loading thread, and its UI is
        // under the loading screen.
        if self.scene_manager.loading.is_none() {
            let paths = self.ui.ui_rendering.image_quads.keys().map(|(_, path)| path.as_str());
            self.assets.load_missing_images(&self.renderer.device, &self.renderer.queue, paths);
        }

        // Rebuilt last: glyphon's TextArea values above still borrow from inside
        // renderizable_elements (each label's Buffer), so nothing else can take a
        // &mut of self.ui until text_areas is fully consumed.
        self.ui.build_image_draws(&self.renderer.device, &self.renderer.queue);

        self.ui.has_changed = false;
    }

    // The mouse cursor for this frame: a debug window's while the mouse is over
    // one, otherwise whatever the UI node under it asks for (UiNode::set_cursor),
    // otherwise the arrow. Left alone while the mouse is locked - it's hidden.
    fn apply_cursor(&mut self) {
        if self.window.is_cursor_locked() {
            return;
        }
        let icon = if self.debug_windows_shown() && self.debug_ui.ctx.is_pointer_over_area() {
            self.debug_ui.cursor
        } else {
            self.ui.ui_rendering.hover_cursor.unwrap_or_default()
        };
        self.window.set_cursor_icon(icon);
    }

    // Runs every node's .on_click(...) callback that was clicked this frame (see
    // UiNode::is_clicked, set in node_content_preparation above). Two passes: first
    // just walk the tree collecting (and removing, see Option::take) each clicked
    // node's on_click closure paired with its path - a plain &mut borrow of
    // self.ui.renderizable_elements, no &mut App needed yet. Only then, with that
    // borrow fully released, call each closure with &mut self - since
    // renderizable_elements was never emptied or taken out of self.ui (unlike an
    // earlier version of this that did exactly that), a handler can freely look up
    // and mutate ANY UI node via Ui::get_ui_node while it runs, including its own
    // node or an ancestor (e.g. a button hiding the panel it's inside of).
    fn fire_ui_click_handlers(&mut self) {
        // A node is only clicked the frame the left button goes down (see
        // UiNode::is_clicked) - every other frame there's nothing to look for.
        if !input::mouse_left_button_just_pressed() {
            return;
        }
        // A debug window sits on top of the game's own UI. A click on it must
        // not also hit the button underneath.
        if self.debug_windows_shown() && self.debug_ui.ctx.is_pointer_over_area() {
            return;
        }
        // Walked in the order they're drawn (see prepare_ui_content):
        // always_on_bottom, the rest, always_on_top, overlay - the map itself
        // has no order, and a node drawn under another must not take a click
        // meant for the one over it (a dropdown's box under its open list).
        let ui = &self.ui;
        let layered = |key: &String| ui.always_on_bottom.contains(key) || ui.always_on_top.contains(key) || ui.overlay.contains(key);
        let mut order: Vec<String> = ui.always_on_bottom.clone();
        order.extend(ui.renderizable_elements.keys().filter(|key| !layered(key)).cloned());
        order.extend(ui.always_on_top.iter().cloned());
        order.extend(ui.overlay.iter().cloned());

        let mut pending: Vec<(String, Box<dyn FnMut(&mut App)>)> = Vec::new();
        for id in &order {
            if let Some(node) = self.ui.renderizable_elements.get_mut(id) {
                node.take_click_handlers(id, &mut pending);
            }
        }
        // A click can land on several overlapping nodes (a dropdown drawn over
        // the row under it, and that row). The walk is in drawing order, and
        // parent-then-children within each node, so the last one is the one
        // painted on top. Only that one runs; the rest still get their
        // closures put back.
        let topmost = pending.len().saturating_sub(1);
        for (index, (path, mut on_click)) in pending.into_iter().enumerate() {
            if index == topmost {
                on_click(self);
            }
            if let Some(node) = Ui::get_ui_node(&mut self.ui.renderizable_elements, &path) {
                node.restore_click_handler(on_click);
            }
        }
    }

    /// Shows the window and runs the game until it's closed (or
    /// `should_quit` is set). `scene_manager` is set up by the caller before
    /// this - registering a scene and opening it only take effect here, after
    /// the splash scene (if any - see `SceneManager::set_splash_scene`).
    pub fn run(mut self) {
        let mut app_state = AppState { is_running: true };
        let mut event_pump = self.event_pump.take().expect("run consumes the App, so the event pump is still here");

        self.window.show();
        // Whatever was set on the window before run, the renderer follows.
        self.sync_window_size();

        self.scene_manager.begin_splash();

        loop {
            self.time.update();
            // true if the OS/window manager requested a close (SDL2's own
            // Event::Quit - clicking the window's close button, Cmd+Q, etc.)
            let quit_requested = input::update(&mut event_pump, self.time.unscaled_delta_time, false);
            if quit_requested {
                app_state.is_running = false;
            }
            self.sync_window_size();

            if !app_state.is_running || self.should_quit {
                self.physics.stop();
                break
            }

            if self.scene_manager.reset {
                SceneManager::apply_open(&mut self);
            } else {
                SceneManager::poll_load(&mut self);
                let physics_data = self.receive_physics();

                // The UI bounds overlay (no key - set Ui::debug_bounds) -
                // rebuilt every frame while it's on, since it has to track
                // wherever nodes actually are right now.
                if self.ui.debug_bounds {
                    self.ui.has_changed = true;
                }

                // Tick the active scene - taken out of its slot first so there's no
                // conflicting borrow with the &mut self it needs, put back once done.
                // Its nodes tick right alongside it, in the same taken-out window -
                // their behaviors' update, then after_physics (once a frame, not at
                // the physics thread's rate - see Behavior::after_physics).
                if let Some(mut scene) = self.scene_manager.active_scene.take() {
                    // Taken out of self.physics (the Rc shared, not the lines
                    // copied) - the scene gets &mut self too.
                    let physics_tx = self.physics.command_tx().cloned();
                    let debug_physics = Rc::clone(&self.physics.debug_lines);
                    let mut ctx = FrameContext {
                        app_state: &mut app_state,
                        event_pump: &mut event_pump,
                        physics_command_tx: physics_tx.as_ref(),
                        physics_data: &physics_data,
                        debug_physics: &debug_physics,
                    };
                    // Whatever node physics halves published since last frame,
                    // in place before any behavior's update gets to read it.
                    for states in self.physics.take_node_states() {
                        scene.content.nodes.apply_physics_states(states);
                    }

                    scene.run_update(&mut self, &mut ctx);

                    let mut changes = SceneChanges::default();
                    scene.content.nodes.update(&mut scene.cameras, &mut self, &mut changes);
                    // Only with a physics thread - without one there are no
                    // results to react to (see Behavior::after_physics).
                    if self.physics.is_running() {
                        scene.content.nodes.after_physics(&mut scene.cameras, &mut self, &physics_data, &mut changes);
                    }
                    // What the behaviors spawned and despawned, now they've all run.
                    scene.apply_changes(&mut self, changes, physics_tx.as_ref());

                    // Everything behaviors wrote for their physics halves this
                    // frame, sent as one batch. Always taken, even with no
                    // physics running, so it can't pile up on the nodes.
                    self.physics.send_inputs(scene.content.nodes.take_physics_inputs());

                    // The animation's turn, after the behaviors - then the
                    // hierarchy (children onto their parents, LookAt nodes
                    // turned), and from that every renderer instance and
                    // camera node's camera.
                    scene.run_late_update(&mut self, &mut ctx);
                    scene.content.nodes.resolve_hierarchy();
                    scene.sync_render_instances();
                    scene.sync_cameras();

                    // Re-aims every look_at-configured camera at its current
                    // target - last, so it wins over whatever a Behavior/
                    // scene's own update already did to a camera's
                    // position/orientation this frame (see SceneCameras::
                    // update's own doc comment).
                    scene.cameras.update(&scene.content.renderizable_instances);
                    scene.run_after_cameras(&mut self);

                    self.scene_manager.active_scene = Some(scene);
                }

                SceneManager::advance_load(&mut self);
                self.scene_manager.advance_splash(self.time.unscaled_delta_time);

                let camera_position = self.scene_manager.cameras().map(|c| c.active().camera.position().coords).unwrap_or_else(Vector3::zeros);
                self.write_instance_buffers(camera_position);
                self.apply_settings(camera_position);

                // No-op unless something called SceneCameras::transition_to(...) -
                // advances the blend (if any) before this frame's view_proj upload.
                if let Some(cameras) = self.scene_manager.cameras_mut() {
                    cameras.update_transition(self.time.delta_time);
                    self.renderer.camera_resources.update_buffer(&self.renderer.queue, cameras.active());
                }
                self.update_particles();
                // A game shader's file saved: built again (see ShaderLibrary::watch).
                // (The depth's near/far buffer isn't written here: it's made
                // with those values, and they never change.)
                self.renderer.shaders.watch();

                // TEMP: debug_text!/F3 messages have nowhere on-screen to render yet
                // - dump to stdout for now so debug_text! is actually visible somewhere.
                let messages = crate::tooling::debug_console::drain_messages();
                if crate::tooling::debug_console::is_console_visible() {
                    for message in messages {
                        println!("{message}");
                    }
                }

                self.run_debug_ui();
            }

            match self.render() {
                Ok(_) => {},
                Err(wgpu::SurfaceError::Outdated) => {
                    self.resize()
                }
                Err(wgpu::SurfaceError::Lost) => {
                    eprintln!("Device lost! You need to recreate the device and all resources.");
                    break;
                }
                Err(e) => eprintln!("Error: {}", e),
            }
        }
    }

    /// The physics thread's latest - every body's pose (applied to its node
    /// and renderer instance here) and its debug lines. Returns the newest
    /// batch of poses, for FrameContext.
    fn receive_physics(&mut self) -> Rc<HashMap<String, RenderMessage>> {
        if !self.physics.is_running() {
            return Rc::default();
        }
        let physics_data = self.physics.receive(self.renderer.render_physics.visible);

        // Paused, the poses aren't applied - the physics thread may still be
        // a step or two ahead when the pause reaches it, and drawing those
        // reads as the body coasting on after everything else stopped.
        let paused = self.time.is_paused();
        let now = Instant::now();

        self.renderer.render_physics.renderizable_lines.clear();
        for message in self.physics.debug_lines.iter() {
            match message {
                DebugPhysicsMessageType::RenderizableLines { owner, line } => {
                    let mut line = *line;
                    // The mesh is drawn ahead of the last physics sample (see
                    // physics_pose_now). Carry this body's lines by the same
                    // amount, or they lag behind it.
                    if let (false, Some(pose)) = (paused, owner.as_ref().and_then(|id| self.physics.poses.get(id))) {
                        let kinematic = owner.as_ref().is_some_and(|id| self.scene_manager.content().is_some_and(|content| content.kinematic.contains(id)));
                        if let (false, Some((position, rotation))) = (kinematic, physics_pose_now(pose, now)) {
                            let sample_rotation = nalgebra::UnitQuaternion::new_normalize(pose.rotation);
                            let shift = |point: [f32; 3]| {
                                let local = sample_rotation.inverse() * (Vector3::new(point[0], point[1], point[2]) - pose.translation);
                                let world = position + rotation * local;
                                [world.x, world.y, world.z]
                            };
                            line[0].position = shift(line[0].position);
                            line[1].position = shift(line[1].position);
                        }
                    }
                    self.renderer.render_physics.renderizable_lines.push(line);
                },
                DebugPhysicsMessageType::RenderizablePoint(_) => {},
            }
        }

        // Each body's pose onto its node's Transform3D - where every node's
        // pose is read from (the hierarchy, the animation) and where the
        // renderer instance is synced from at the end of the frame - and its
        // renderer instance. Not onto a node an animation is moving
        // (SceneContent::kinematic) - its body is held still meanwhile.
        if !paused {
            if let Some(content) = self.scene_manager.content_mut() {
                for (key, pose) in self.physics.poses.iter() {
                    if content.kinematic.contains(key) {
                        continue;
                    }
                    let Some((position, rotation)) = physics_pose_now(pose, now) else { continue };
                    if let Some(renderizable) = content.renderizable_instances.get_mut(key) {
                        renderizable.instance.transform.position = position;
                        renderizable.instance.transform.rotation = rotation;
                    }
                    if let Some(transform) = content.nodes.get_mut(key).and_then(|node| node.transform_3d_mut()) {
                        transform.position = position;
                        transform.rotation = rotation;
                    }
                }
            }
        }

        physics_data
    }

    /// Every model's instance buffer from its nodes' poses - grouped by
    /// model, each model's sorted by material, so every material draws one
    /// run of them (ModelDataInstance::material_ranges).
    fn write_instance_buffers(&mut self, camera_position: Vector3<f32>) {
        let mut model_instances: HashMap<&str, Vec<(&str, InstanceRaw)>> = HashMap::new();
        if let Some(content) = self.scene_manager.content() {
            for renderizable in content.renderizable_instances.values() {
                model_instances
                    .entry(renderizable.model_ref.as_str())
                    .or_default()
                    .push((renderizable.material.as_str(), renderizable.instance.transform.to_raw(camera_position)));
            }
        }

        for (model_ref, model) in self.assets.game_models.iter_mut() {
            model.material_ranges.clear();
            let Some(instances) = model_instances.get_mut(model_ref.as_str()) else { continue };
            instances.sort_by(|a, b| a.0.cmp(b.0));
            let mut start = 0;
            for index in 1..=instances.len() {
                if index == instances.len() || instances[index].0 != instances[start].0 {
                    model.material_ranges.push((instances[start].0.to_owned(), start as u32..index as u32));
                    start = index;
                }
            }
            let raws: Vec<InstanceRaw> = instances.iter().map(|(_, raw)| *raw).collect();
            if !raws.is_empty() {
                self.renderer.queue.write_buffer(&model.instance_buffer, 0, bytemuck::cast_slice(&raws));
            }
        }
    }

    /// The game's debug windows (see DebugUi): its toggle key, the mouse
    /// freed while they're up, and - while they are - the active scene's
    /// `debug_ui` run to build them - drawn by `render`.
    fn run_debug_ui(&mut self) {
        self.debug_ui.handle_toggle_key();
        // The debug windows free the mouse to click them.
        let shown = self.debug_windows_shown();
        if let Some(locked) = self.debug_ui.cursor_lock_change(shown, self.window.is_cursor_locked()) {
            self.window.set_cursor_locked(locked);
        }
        if !self.debug_windows_shown() {
            return;
        }

        let raw_input = self.debug_ui.raw_input([self.window.size.width, self.window.size.height]);
        // egui lays out in points; the window's pixels can be more of them
        // (a HiDPI/Retina display).
        let pixels_per_point = self.window.pixel_size.width as f32 / self.window.size.width.max(1) as f32;
        // A clone (it's shared inside) - the closure below needs &mut self.
        let ctx = self.debug_ui.ctx.clone();
        ctx.set_pixels_per_point(pixels_per_point);
        let mut scene = self.scene_manager.active_scene.take();
        let output = ctx.run(raw_input, |ctx| {
            if let Some(scene) = scene.as_mut() {
                scene.run_debug_ui(self, ctx);
            }
        });
        if let Some(scene) = scene {
            self.scene_manager.active_scene = Some(scene);
        }
        self.debug_ui.set_frame(output);
    }

    /// Whether the game's debug windows (F1) are drawn this frame.
    fn debug_windows_shown(&self) -> bool {
        self.debug_ui.visible
    }
}

/// Where a physics body is at `now`, from its last reported `pose`: carried
/// on along its velocity and spin for the time since that pose was true -
/// `None` when it's stale (see MAX_LEAD).
/// The physics thread steps at a fixed 120 Hz, out of step with the frames,
/// so a raw pose is anywhere from 0 to ~2 steps old - drawn as is, a fast
/// jet stutters next to anything moved smoothly every frame (a missile, the
/// world around it). Past MAX_LEAD the pose is stale (a hitch, a level
/// that's just restarted) and the body is left where it's drawn.
fn physics_pose_now(pose: &RenderMessage, now: std::time::Instant) -> Option<(nalgebra::Vector3<f32>, nalgebra::UnitQuaternion<f32>)> {
    const MAX_LEAD: f32 = 0.05;
    let rotation = nalgebra::Unit::new_normalize(pose.rotation);
    let lead = now.saturating_duration_since(pose.sampled_at).as_secs_f32();
    if lead > MAX_LEAD {
        return None;
    }
    let spin = nalgebra::UnitQuaternion::from_scaled_axis(pose.angvel * lead);
    Some((pose.translation + pose.linvel * lead, spin * rotation))
}
