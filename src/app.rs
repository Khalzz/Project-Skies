use std::time::{Duration, Instant};
use std::collections::HashMap;
use std::env;

use wgpu::{BindGroupLayout, BindGroupLayoutDescriptor, Device, DeviceDescriptor, Features, InstanceDescriptor, Limits, Queue, Surface, SurfaceConfiguration, TextureUsages};
use sdl2::{JoystickSubsystem, GameControllerSubsystem, HapticSubsystem};
use glyphon::{Cache, Resolution, TextArea, Viewport};

use crate::engine::audio::audio::Audio;
use crate::engine::physics::physics::{physics_handling, DebugPhysicsMessageType, PhysicsDataTransmission};
use crate::engine::physics::physics_handler::{RenderMessage, PhysicsCommand};
use crate::engine::rendering::enviroment::skybox_renderer::SkyboxRender;
use crate::engine::rendering::enviroment::environment;
use crate::engine::rendering::instance_management::{InstanceData, InstanceRaw, ModelDataInstance};
use crate::engine::rendering::render_pipeline::depth_renderer::DepthRender;
use crate::engine::rendering::camera::handler::CameraResources;
use crate::engine::rendering::models::textures::Texture;
use crate::engine::game_nodes::timing::Timing;
use crate::engine::rendering::enviroment::light::Light;
use crate::engine::rendering::models::model::{self, Mesh, Model, Vertex};
use crate::engine::rendering::renderer::Renderer;
use crate::engine::scene_manager::scene::{FrameContext, PendingSceneLoad, Scene, SceneManager};
use crate::engine::splash_screen::SplashScreenConfig;
use crate::engine::input::input;
use crate::engine::rendering::ui::physics_rendering::RenderPhysics;
use crate::engine::rendering::ui::rendering_utils;
use crate::engine::rendering::ui::ui::Ui;
use crate::engine::ui::color::UiColor;
use crate::engine::ui::ui_node::UiNode;
use crate::engine::ui::ui_transform::PositionValue;
use crate::game::scenes::loading::scene::{LoadingScreenScene, FADE_OUT_SECS, PANEL_KEY};
use crate::resources;
use crate::engine::window::window::{WindowManager, WindowSettings};

#[derive(Clone)]
pub struct AppState {
    pub is_running: bool,
}

pub struct Size {
    pub width: u32,
    pub height: u32
}

pub struct Throttling {
    pub last_ui_update: Instant,
    pub ui_update_interval: Duration,
}

pub struct App {
    pub window_manager: WindowManager,
    pub renderer: Renderer,
    // Placeholder pool/starting scene at construction time - the real configuration
    // is assigned by the caller (see main.rs) before App::run is called.
    pub scene_manager: SceneManager,
    pub render_pipeline: wgpu::RenderPipeline,
    pub ui: Ui,
    pub camera_resources: CameraResources,
    // Pre-scene fallback only now - a real scene's own skybox/clear color
    // live on its Scene (see SceneEnvironment), since that's scoped to
    // whichever scene applied it and dropped for free on the next scene
    // switch. These two only matter before the first scene has ever reset
    // (run_splash_screen writes into them directly, since no Scene exists
    // yet at that point) - see render_pass.rs's own fallback read.
    pub skybox: Option<SkyboxRender>,
    pub clear_color: wgpu::Color,
    pub show_depth_map: bool,
    pub joystick_subsystem: JoystickSubsystem,
    pub _haptic_subsystem: HapticSubsystem,
    pub throttling: Throttling,
    pub game_models: HashMap<String, ModelDataInstance>,
    // Loaded (see resources::register_model) but not yet instanced - a model
    // moves out of here into game_models the first time something actually
    // references it (see render_bridge::register_static_model), since a
    // ModelDataInstance's buffer can't exist meaningfully with zero instances
    // (wgpu rejects a zero-size buffer).
    pub loaded_models: HashMap<String, Model>,
    // Named images (see resources::register_texture) - no equivalent split to
    // loaded_models/game_models needed, a texture has no per-instance GPU
    // buffer to size/rebuild the way a model's does.
    pub textures: HashMap<String, Texture>,
    pub light: Light,
    pub time: Timing,
    pub scene_openned: Option<String>,
    pub audio: Audio,
    pub render_physics: RenderPhysics,
    // Opt-in: None by default, assign before calling run() to show a splash screen.
    pub splash_screen: Option<SplashScreenConfig>,
    // Set/cleared by play::ui::open_pause_menu/close_pause_menu - a UI button's
    // on_click only ever gets `&mut App` (see UiNode::on_click), not the
    // physics command channel (that only exists inside GameLogic::update's own
    // FrameContext), so a button can't send PhysicsCommand::TogglePause
    // directly. This flag is the handoff: GameLogic::update polls it every
    // frame and reacts to its rising/falling edge (same idiom as
    // `was_cinematic_active`) to actually pause/resume physics and gate
    // `Plane::update`. Meaningless outside the "playing" scene, same as
    // `show_depth_map`/other single-scene debug flags already living here.
    pub is_paused: bool,
    // (game_ui_was_active, velocity_marker_was_active) - set by
    // play::ui::open_pause_menu, consumed by close_pause_menu. The flight HUD
    // is force-hidden while paused (it'd otherwise still show through/around
    // PauseBackdrop's gradient), but *which* of these were actually visible
    // depends on where in the scene's own timeline pausing happened (e.g.
    // during the mission-intro, before the HUD's own reveal, both are still
    // false) - remembering the real value here is what lets closing the menu
    // restore exactly that instead of just forcing both back on.
    pub paused_hud_visibility: Option<(bool, bool)>,
}

impl App {
    pub async fn new(title: &str, ext_width: Option<u32>, ext_height: Option<u32>) -> Result<App, String> {
        // Window initialization
        
        let window_manager = WindowManager::new(WindowSettings {
            tittle: title.to_string(),
            size: None,
            screen_index: None,
            fullscreen: true,
        });

        env::set_var("SDL_VIDEO_MINIMIZE_ON_FOCUS_LOSS", "0");
        window_manager.context.mouse().set_relative_mouse_mode(true);

        let joystick_subsystem = window_manager.context.joystick().unwrap();
        let haptic_subsystem = window_manager.context.haptic().unwrap();

        // WGPU initialization
        let renderer = Renderer::new(&window_manager).await?;

        // rendering elements
        let ui = Ui::new(&renderer.device, &renderer.queue, &renderer.config, &renderer.glyphon.cache, &renderer.blur.scene_color, &renderer.blur.blurred);
        let camera_resources = CameraResources::new(&renderer.device, &renderer.config);
        let light = Light::new(&renderer.device, &renderer.config, &camera_resources);

        let render_pipeline_layout = renderer.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Render Pipeline Layout"),
            bind_group_layouts: &[
                &Texture::create_bind_group_layout(&renderer.device),
                &camera_resources.bind_group_layout,
                &Mesh::create_bind_group_layout(&renderer.device),
                &light.rendering_data.bind_group_layout
            ],
            push_constant_ranges: &[],
        });
        
        // SHADERING PROCESS 
        let render_pipeline = {
            let shader = wgpu::ShaderModuleDescriptor {
                label: Some("Normal Shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("engine/shaders/depth.wgsl").into()),
            };
            
            rendering_utils::create_render_pipeline(
                &renderer.device,
                &render_pipeline_layout,
                renderer.config.format,
                Some(Texture::DEPTH_FORMAT),
                &[model::ModelVertex::desc(), InstanceRaw::desc()],
                shader,
            )
        };

        let game_models = HashMap::new();

        // No environment loaded yet - each scene declares its own via
        // resources::apply_environment when it resets (see rendering::enviroment::environment).
        let skybox = None;
        let clear_color = environment::DEFAULT_CLEAR_COLOR;

        // physics rendering
        let render_physics = RenderPhysics::new(&renderer.device, &renderer.config, &camera_resources);

        // Physics data
        let time = Timing::new();

        Ok(App {
            window_manager,
            renderer,
            scene_manager: SceneManager::new(),
            render_pipeline,
            ui,
            camera_resources,
            skybox,
            clear_color,
            show_depth_map: false,
            joystick_subsystem,
            throttling: Throttling { last_ui_update: Instant::now(), ui_update_interval: Duration::from_secs_f32(1.0/120.0) },
            _haptic_subsystem: haptic_subsystem,
            game_models,
            loaded_models: HashMap::new(),
            textures: HashMap::new(),
            light,
            time,
            scene_openned: None,
            audio: Audio::new(),
            render_physics,
            splash_screen: None,
            is_paused: false,
            paused_hud_visibility: None,
        })
    }

    pub fn resize(&mut self) {
        self.window_manager.refresh_size();
        // Real backing pixels (see WindowManager::pixel_size) - these three all
        // size actual GPU render targets/aspect ratio, which have to match the
        // surface's real resolution, not the window's points size. UI layout/
        // hit-testing elsewhere stays on window_manager.size (points) - see
        // that field's own doc comment.
        let width = self.window_manager.pixel_size.width;
        let height = self.window_manager.pixel_size.height;

        self.renderer.resize(width, height);
        self.camera_resources.resize(width, height);
        if let Some(cameras) = self.scene_manager.cameras_mut() {
            cameras.resize(width, height);
        }
        self.ui.resize_blur_binding(&self.renderer.device, &self.renderer.queue, &self.renderer.blur.scene_color, &self.renderer.blur.blurred, width, height);
    }

    pub fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        self.sync_no_camera_message();
        self.prepare_ui_content();
        self.fire_ui_click_handlers();

        // WGPU
        let output = self.renderer.surface.get_current_texture()?;
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self.renderer.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });

        self.render_scene_passes(&mut encoder, &view);

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
        // camera-less the whole time - see game::scenes::loading::scene); or no scene
        // has ever been constructed yet at all - run_splash_screen calls
        // render() in its own loop before the first scene reset ever runs,
        // same "nothing to blame a scene for" reasoning as the loading case.
        if self.scene_manager.cameras().is_some_and(|c| c.has_active_camera()) || self.scene_manager.loading.is_some() || self.scene_manager.active_scene.is_none() {
            if self.ui.renderizable_elements.remove(Self::NO_CAMERA_MESSAGE_KEY).is_some() {
                self.ui.has_changed = true;
            }
            return;
        }

        if self.ui.renderizable_elements.contains_key(Self::NO_CAMERA_MESSAGE_KEY) {
            return;
        }

        let screen_width = self.window_manager.size.width as f32;
        let screen_height = self.window_manager.size.height as f32;
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

    // Rebuilds UI vertex/index/text buffers from the current node tree - only when
    // something actually marked the UI dirty, so idle frames skip it entirely.
    fn prepare_ui_content(&mut self) {
        if !self.ui.has_changed {
            return;
        }

        // Two separate batches (see TextRendering::text_renderer_on_top's own doc
        // comment for why one shared Vec/one glyphon TextRenderer can't do this) -
        // main pass's text, and always_on_top nodes' text, prepared+rendered as
        // two independent draw calls so the latter actually lands above the
        // former instead of every node's text piling into one shared final pass.
        let mut text_areas: Vec<TextArea> = Vec::new();
        let mut text_areas_on_top: Vec<TextArea> = Vec::new();

        self.ui.ui_rendering.vertices.clear();
        self.ui.ui_rendering.num_vertices = 0;

        self.ui.ui_rendering.indices.clear();
        self.ui.ui_rendering.num_indices = 0;

        self.ui.ui_rendering.image_quads.clear();

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

        // An active always_on_top node (a modal, currently) should block input to
        // literally everything else while it's up, the same way a real dialog
        // does - otherwise it's only visually on top (see Ui::always_on_top's own
        // doc comment for the rendering half of this), and clicks still fall
        // through to whatever button happens to sit at the same screen position
        // underneath. always_on_top nodes themselves stay hit-testable regardless
        // (true below for that loop) - there's only ever one layer of "on top"
        // today, so nothing can occlude them in turn.
        let blocks_input = on_top_nodes.iter().any(|(_, n)| n.is_active);

        // Points -> real backing pixels (see WindowManager::pixel_size's own doc
        // comment) - node_content_preparation needs this to place glyphon
        // TextAreas correctly, since glyphon's own coordinate space is always
        // real pixels regardless of what space everything else here (UiTransform,
        // mouse position) is in. See Label::text_area's own doc comment.
        let dpi_scale = self.window_manager.pixel_size.width as f32 / self.window_manager.size.width as f32;

        // Debug bounds overlay (F2) - has to run before node_content_preparation
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
                    $ui_node.debug_bounds_preparation(&self.window_manager.size, dpi_scale, &mut self.ui.ui_rendering);
                }
                // None - top-level nodes start unclipped; a scrollable
                // container establishes its own clip for its descendants
                // further down the recursion (see node_content_preparation's
                // Container branch/clip_rect's own doc comment).
                let (textareas_to_merge, _vertices_to_add, _indices_to_add) = $ui_node.node_content_preparation(&self.window_manager.size, dpi_scale, &mut self.ui.ui_rendering, &mut self.ui.text.font_system, self.time.delta_time, $hit_testable, None);
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
        for (_key, ui_node) in &mut on_top_nodes {
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

        // Rebuilt last: glyphon's TextArea values above still borrow from inside
        // renderizable_elements (each label's Buffer), so nothing else can take a
        // &mut of self.ui until text_areas is fully consumed.
        self.ui.build_image_draws(&self.renderer.device);

        self.ui.has_changed = false;
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
        let mut pending: Vec<(String, Box<dyn FnMut(&mut App)>)> = Vec::new();
        for (id, node) in self.ui.renderizable_elements.iter_mut() {
            node.take_click_handlers(id, &mut pending);
        }
        for (path, mut on_click) in pending {
            on_click(self);
            if let Some(node) = Ui::get_ui_node(&mut self.ui.renderizable_elements, &path) {
                node.restore_click_handler(on_click);
            }
        }
    }

    // self.scene_manager is configured by the caller (see main.rs) before run() is
    // called - App only needs to know about the Scene trait, not any concrete scene
    // type. This is safe to do before run() despite the splash screen running first:
    // SceneManager::create_scene only registers a constructor (no side effects, it
    // doesn't run), and open_scene just sets which one is active + requests a reset -
    // the actual construction only happens once the main loop below processes that
    // reset, which is unconditionally after run_splash_screen() regardless of when
    // create_scene/open_scene were called.
    pub fn run(mut self) {
        // SDL2
        let mut app_state = AppState { is_running: true };
        let mut event_pump = self.window_manager.context.event_pump().unwrap();

        // Started/stopped per scene switch below, based on the active scene's
        // Scene::physics() - None until a physics-wanting scene resets.
        let mut physics_data_channel: Option<PhysicsDataTransmission> = None;

        // Device connect/disconnect (hot-plug) is handled inside InputSubsystem itself
        // from here on, not a one-shot "find a controller at startup" step. Obtained
        // fresh here (rather than stored on App) so it can move into the input
        // singleton without partially moving self, which self stays borrowed as a
        // whole for the rest of this function.
        let controller_subsystem = self.window_manager.context.game_controller().unwrap();
        input::init(include_str!("../settings/input.ron"), controller_subsystem);

        self.run_splash_screen(&mut event_pump);

        let mut debug_physics: Vec<DebugPhysicsMessageType> = Vec::new();

        loop {
            // Relevant subsystems update
            self.time.update();
            input::update(&mut event_pump, self.time.delta_time, false);

            if !app_state.is_running {
                // Send shutdown command to physics thread, if one is running
                if let Some(physics) = &physics_data_channel {
                    let _ = physics.request_data_tx.send(PhysicsCommand::Shutdown);
                }
                break
            }

            if self.scene_manager.reset {
                let active = self.scene_manager.active.clone();

                // Environment doesn't carry over between scenes (same as Godot: no
                // WorldEnvironment in the new scene falls back to the default, it
                // doesn't inherit whatever the previous scene had) - a scene that
                // wants a skybox declares it via create_loaded_scene's `environment`
                // argument (or, for a cheap create_scene scene, sets it directly).
                // Like content/cameras, this now happens for free: skybox/clear_color
                // live on Scene (SceneEnvironment) now, so a freshly-constructed one
                // just starts back at Default::default() - no clear call needed here
                // any more (self.skybox/self.clear_color below only matter pre-first-
                // scene, see their own doc comment on App).

                // Same for UI - Ui::load_ui/add_to_ui only ever insert, they never
                // clear, so without this whatever the previous scene added would just
                // keep piling up/leaking into scenes that never asked for it. A scene
                // that wants UI has to (re)build it itself in new(), same contract as
                // the skybox above.
                self.ui.renderizable_elements.clear();
                self.ui.always_on_top.clear();
                self.ui.always_on_bottom.clear();
                self.ui.has_changed = true;

                // The code-first entity system's nodes, the renderable
                // instances they/data.ron put up, AND every camera the scene
                // registered (SceneCameras, see its own doc comment) now all
                // live on the outgoing Scene itself, so they all get dropped
                // for free the moment `active_scene` below is overwritten
                // with a freshly-constructed one - no separate clear calls
                // needed any more for any of them. The fresh Scene's own
                // SceneCameras::new already seeds the "no camera" placeholder
                // and selects it, so there's always a valid active camera
                // even for a scene that never creates its own (see
                // has_active_camera's own doc comment).

                // Physics doesn't carry over between scenes either - stop whatever was
                // running now. The new scene's own physics (if any) starts once it's
                // actually constructed: immediately below for a cheap scene, or once
                // its background load finishes for a heavy one (see the `else` branch
                // below, where PendingSceneLoad gets polled).
                if let Some(old_physics) = physics_data_channel.take() {
                    let _ = old_physics.request_data_tx.send(PhysicsCommand::Shutdown);
                }

                // The scene doesn't exist yet at this point - SceneManager::create_scene/
                // create_loaded_scene only registered its constructor. Clone it out first
                // (rather than borrowing self.scene_manager's maps directly) so calling it
                // with &mut self below doesn't conflict with that borrow.
                if let Some(factory) = self.scene_manager.factory_for(&active) {
                    // This is the only place a cheap scene's real constructor ever runs,
                    // and only for the one actually becoming active.
                    let mut scene = factory(&mut self);
                    scene.run_on_spawn(&mut self);

                    if !scene.cameras.has_active_camera() {
                        eprintln!("scene '{active}' didn't create a camera - showing the fallback \"Add a camera to the scene\" screen");
                    }

                    // Re-baseline the frame clock right after a (synchronous, possibly
                    // multi-second - model/texture loading, all blocking) scene
                    // constructor returns. self.time.update() measures delta_time as
                    // "time since the last update() call" - without this, that call
                    // was the one at the top of THIS frame's loop iteration, before
                    // `factory` ran, so the very first delta_time the new scene's own
                    // update() ever sees would silently include however long its own
                    // loading just took (confirmed via logging: 3+ real seconds for
                    // this project's one test level) - enough to make anything relying
                    // on that first tick's delta_time (e.g. a state machine measuring
                    // elapsed seconds) skip straight past its own timing entirely. This
                    // makes delta_time start counting from "the scene is actually
                    // playable", not "whenever the previous scene's last frame happened
                    // to render" - every other per-frame system gets this for free, not
                    // just the ones that happened to need a manual clamp already.
                    self.time.update();

                    physics_data_channel = scene.run_fixed_update(&self).map(|(physics_bodies, physics_tick)| {
                        physics_handling(&self.renderer.device, &self.renderer.config, &self.camera_resources, physics_bodies, physics_tick)
                    });

                    self.scene_manager.active_scene = Some(scene);
                } else if let Some(spec) = self.scene_manager.loader_for(&active) {
                    // Heavy scene (registered via create_loaded_scene): run its own
                    // `prepare` on a background thread instead of blocking the loop
                    // here, so render() below keeps presenting every frame instead of
                    // the window freezing for however long it takes - the same
                    // std::thread::spawn + std::sync::mpsc pattern the physics thread
                    // already uses (see physics::physics_handling). Device/Queue/
                    // BindGroupLayout are cheap, Arc-backed and Clone in wgpu, so the
                    // spawned thread can build real GPU resources (buffers, textures)
                    // itself - no raw bytes need to cross back.
                    let device = self.renderer.device.clone();
                    let queue = self.renderer.queue.clone();
                    let camera_bind_group_layout = self.camera_resources.bind_group_layout.clone();
                    let config = self.renderer.config.clone();
                    let prepare = spec.prepare.clone();

                    let (tx, rx) = std::sync::mpsc::channel();
                    std::thread::spawn(move || {
                        let prepared = prepare(&device, &queue, &camera_bind_group_layout, &config);
                        let _ = tx.send(prepared);
                    });

                    self.scene_manager.active_scene = Some(Scene::new(&self).set_scene_behaviour(LoadingScreenScene::new(&mut self)));
                    self.scene_manager.loading = Some(PendingSceneLoad { receiver: rx, finish: spec.finish.clone(), ready: None });
                } else {
                    eprintln!("No scene registered for state '{}'", active);
                }
                self.scene_manager.reset = false;
            } else {
                // If a heavy scene's background load (kicked off above on whichever
                // earlier frame set scene_manager.loading) has just finished, don't
                // swap immediately - start fading LoadingScreenScene's own UI back
                // out first (PendingSceneLoad::ready; the actual fade progression and
                // the eventual swap happen further below, *after* this frame's scene
                // tick - see that block's own comment for why the ordering matters).
                // Non-blocking, same try_recv() polling the physics channel below
                // already uses. Until this fires, active_scene stays the
                // LoadingScreenScene the reset branch put there, so this and every
                // frame in between still renders normally.
                if let Some(pending) = self.scene_manager.loading.as_mut() {
                    if pending.ready.is_none() {
                        if let Ok(prepared) = pending.receiver.try_recv() {
                            pending.ready = Some((prepared, 0.0));
                        }
                    }
                }

                // Request physics data from physics thread, only if the active
                // scene actually has one running.
                let physics_data = if let Some(physics) = &physics_data_channel {
                    if let Err(e) = physics.request_data_tx.send(PhysicsCommand::RequestData) {
                        eprintln!("Failed to send physics command: {}", e);
                    }

                    // Toggle debug rendering with F2 (also shows console)
                    if input::is_action_just_pressed("toggle_debug") {
                        self.render_physics.visible = !self.render_physics.visible;
                        if let Err(e) = physics.request_data_tx.send(PhysicsCommand::ToggleDebug) {
                            eprintln!("Failed to send toggle debug command: {}", e);
                        }
                    }

                    // Toggle physics pause with F12
                    if input::is_action_just_pressed("toggle_pause") {
                        if let Err(e) = physics.request_data_tx.send(PhysicsCommand::TogglePause) {
                            eprintln!("Failed to send toggle pause command: {}", e);
                        }
                    }

                    // Recibimos los datos del otro thread
                    let physics_data = match physics.physics_data_rx.try_recv() {
                        Ok(data) => data,
                        Err(_) => HashMap::new(),
                    };

                    // Drain all queued debug physics messages, keep only the latest
                    let mut got_new = false;
                    while let Ok(data) = physics.debug_physics_rx.try_recv() {
                        debug_physics = data;
                        got_new = true;
                    }
                    if !got_new {
                        debug_physics.clear();
                    }

                    physics_data
                } else {
                    debug_physics.clear();
                    HashMap::new()
                };

                // Toggle console independently with F3
                if input::is_action_just_pressed("toggle_console") {
                    crate::engine::tooling::debug_console::toggle_console();
                }

                // Toggle UI bounds overlay with F2 - force a rebuild on the toggle
                // frame so it appears/disappears immediately, and every frame after
                // while it's on, since the overlay has to track wherever nodes
                // actually are right now, not just whenever something else last
                // marked the UI dirty.
                if input::is_action_just_pressed("toggle_ui_debug") {
                    self.ui.debug_bounds = !self.ui.debug_bounds;
                }
                if self.ui.debug_bounds {
                    self.ui.has_changed = true;
                }

                // Clear previous debug lines and add new ones
                self.render_physics.renderizable_lines.clear();

                for message in &debug_physics {
                    match message {
                        DebugPhysicsMessageType::RenderizableLines(lines) => {
                            self.render_physics.renderizable_lines.push(lines.clone());
                        },
                        DebugPhysicsMessageType::RenderizablePoint(point) => {
                        },
                    }
                }

                // Apply physics data to transforms first with smoothing
                // Skipped while paused - PhysicsCommand::TogglePause (sent the
                // instant the pause menu opens, see GameLogic::update) doesn't
                // take effect on the physics thread the same instant it's
                // sent - that thread runs independently and may still be a
                // step or two ahead in flight when this fires. Without this
                // gate, whatever it sends back in that gap keeps landing on
                // the plane's render transform for a few more frames after
                // the camera's already frozen (see camera_control's own early
                // return once app.is_paused), reading as the plane visibly
                // coasting forward a little after everything else has
                // stopped. Skipping this loop entirely just holds the render
                // transform at wherever it already was the instant pausing
                // began, matching the camera exactly.
                if !self.is_paused {
                    if let Some(content) = self.scene_manager.content_mut() {
                        for (_key, renderizable) in &mut content.renderizable_instances {
                            if let Some(physics_data) = physics_data.get(&_key.to_string()) {
                                renderizable.instance.transform.position = physics_data.translation;
                                renderizable.instance.transform.rotation = nalgebra::Unit::new_normalize(physics_data.rotation);
                            }
                        }
                    }
                }

                // Tick the active scene - taken out of its slot first so there's no
                // conflicting borrow with the &mut self it needs, put back once done.
                // The code-first entity system's nodes (see
                // engine::scene_manager::scene_nodes) tick right alongside it, in the
                // same taken-out window, since they now live on this same Scene's own
                // content - fixed_update here is an approximation of a true fixed tick
                // (see Behavior::fixed_update's own doc comment for why), not literally
                // driven by the physics thread's 120Hz accumulator.
                if let Some(mut scene) = self.scene_manager.active_scene.take() {
                    let mut ctx = FrameContext {
                        app_state: &mut app_state,
                        event_pump: &mut event_pump,
                        plane_control_tx: physics_data_channel.as_ref().map(|physics| &physics.plane_control_tx),
                        physics_command_tx: physics_data_channel.as_ref().map(|physics| &physics.request_data_tx),
                        physics_data: &physics_data,
                        debug_physics: &debug_physics,
                    };
                    scene.run_update(&mut self, &mut ctx);

                    let delta_time = self.time.delta_time;
                    scene.content.nodes.update(&mut scene.cameras, &mut self, delta_time);
                    scene.content.nodes.fixed_update(&mut scene.cameras, &mut self, delta_time);

                    // Re-aims every look_at-configured camera at its current
                    // target - last, so it wins over whatever a Behavior/
                    // scene's own update already did to a camera's
                    // position/orientation this frame (see SceneCameras::
                    // update's own doc comment).
                    scene.cameras.update(&scene.content.renderizable_instances);

                    self.scene_manager.active_scene = Some(scene);
                } else {
                    eprintln!("No active scene to update");
                }

                // Advance LoadingScreenScene's fade-out once its background load has
                // actually finished (PendingSceneLoad::ready, set above) - deliberately
                // placed *after* the scene tick above: LoadingScreenScene::update just
                // ran and would have overwritten the loading label's alpha with its own
                // pulse animation, so setting the real fade-out alpha has to happen
                // afterward in the same frame to actually be the value that renders.
                if let Some(mut pending) = self.scene_manager.loading.take() {
                    if let Some((prepared, mut fade_elapsed)) = pending.ready.take() {
                        fade_elapsed += self.time.delta_time;
                        let t = (fade_elapsed / FADE_OUT_SECS).clamp(0.0, 1.0);
                        // Only the "Loading..." text fades - the black backdrop stays
                        // fully opaque right up until the swap below clears it, so the
                        // transition into the real scene's own opaque black intro
                        // backdrop (see play::scene::GameLogic::finish) reads as
                        // seamless instead of the background itself visibly fading.
                        if let Some(node) = Ui::get_ui_node(&mut self.ui.renderizable_elements, &format!("{PANEL_KEY}/label")) {
                            node.set_alpha(1.0 - t);
                        }
                        self.ui.has_changed = true;

                        if t >= 1.0 {
                            // LoadingScreenScene's own UI is now fully transparent -
                            // clear it the same way the reset branch clears the
                            // outgoing scene's UI, so the real scene's own finish()
                            // draws onto a clean slate instead of it ending up stuck
                            // underneath the (now invisible, but still present) loading
                            // card forever.
                            self.ui.renderizable_elements.clear();
                            self.ui.always_on_top.clear();
                            self.ui.always_on_bottom.clear();
                            self.ui.has_changed = true;

                            let mut scene = (pending.finish)(&mut self, prepared);
                            scene.run_on_spawn(&mut self);

                            if !scene.cameras.has_active_camera() {
                                eprintln!("scene didn't create a camera - showing the fallback \"Add a camera to the scene\" screen");
                            }

                            // Same rebaseline + physics-spawn steps the sync path runs
                            // right after its own constructor returns - see that
                            // comment further up.
                            self.time.update();
                            physics_data_channel = scene.run_fixed_update(&self).map(|(physics_bodies, physics_tick)| {
                                physics_handling(&self.renderer.device, &self.renderer.config, &self.camera_resources, physics_bodies, physics_tick)
                            });
                            self.scene_manager.active_scene = Some(scene);
                            // pending (and scene_manager.loading) stays cleared - don't put it back.
                        } else {
                            pending.ready = Some((prepared, fade_elapsed));
                            self.scene_manager.loading = Some(pending);
                        }
                    } else {
                        self.scene_manager.loading = Some(pending);
                    }
                }

                // Update instance buffers efficiently - group by model type
                let camera_position = self.scene_manager.cameras().map(|c| c.active().camera.position().coords).unwrap_or_else(nalgebra::Vector3::zeros);
                let mut model_instances: HashMap<String, Vec<InstanceRaw>> = HashMap::new();

                if let Some(content) = self.scene_manager.content() {
                    for (_key, renderizable) in &content.renderizable_instances {
                        model_instances
                            .entry(renderizable.model_ref.clone())
                            .or_insert_with(Vec::new)
                            .push(renderizable.instance.transform.to_raw(camera_position));
                    }
                }

                // Write all instances for each model type at once
                for (model_ref, instances) in model_instances {
                    if let Some(model) = self.game_models.get(&model_ref) {
                        if !instances.is_empty() {
                            self.renderer.queue.write_buffer(&model.instance_buffer, 0, bytemuck::cast_slice(&instances));
                        }
                    }
                }

                // lighting update
                if let Some(sun) = self.scene_manager.content().and_then(|content| content.renderizable_instances.get("sun")) {
                    // Camera-relative, same as the instance model matrices, since it's
                    // consumed alongside camera-relative world positions in the shaders.
                    let relative_light_position = sun.instance.transform.position - camera_position;
                    self.light.uniform.position = (relative_light_position.x, relative_light_position.y, relative_light_position.z).into();
                    match &sun.instance.metadata.lighting {
                        Some(lighting_data) => {
                            self.light.uniform.color = lighting_data.color.into();
                        },
                        None => {},
                    }
                }

                self.renderer.queue.write_buffer(&self.light.rendering_data.buffer, 0, bytemuck::cast_slice(&[self.light.uniform]));
                // lighting update

                // No-op unless something called SceneCameras::transition_to(...) -
                // advances the blend (if any) before this frame's view_proj upload.
                if let Some(cameras) = self.scene_manager.cameras_mut() {
                    cameras.update_transition(self.time.delta_time);
                    self.camera_resources.update_buffer(&self.renderer.queue, cameras.active());
                }
                self.renderer.queue.write_buffer(&self.renderer.depth_render.near_far_buffer, 0, bytemuck::cast_slice(&[self.renderer.depth_render.near_far_uniform]));

                // TEMP: debug_text!/F3 messages have nowhere on-screen to render yet
                // (drain_messages() was write-only - nothing ever called it) - dump to
                // stdout for now so debug_text! is actually visible somewhere. Remove
                // once there's a real on-screen console, or replace with one.
                let messages = crate::engine::tooling::debug_console::drain_messages();
                if crate::engine::tooling::debug_console::is_console_visible() {
                    for message in messages {
                        println!("{message}");
                    }
                }
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

    

}