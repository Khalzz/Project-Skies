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
use crate::engine::rendering::camera::CameraHandler;
use crate::engine::rendering::models::textures::Texture;
use crate::engine::game_nodes::timing::Timing;
use crate::engine::rendering::enviroment::light::Light;
use crate::engine::rendering::models::model::{self, Mesh, Model, Vertex};
use crate::engine::rendering::renderer::Renderer;
use crate::engine::scene_manager::scene::{FrameContext, SceneManager};
use crate::engine::splash_screen::SplashScreenConfig;
use crate::engine::input::input;
use crate::engine::rendering::ui::physics_rendering::RenderPhysics;
use crate::engine::rendering::ui::rendering_utils;
use crate::engine::rendering::ui::ui::Ui;
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
    pub camera: CameraHandler,
    // Configured per scene via resources::apply_environment (called from Scene::reset),
    // not loaded automatically - None means the scene just wants clear_color.
    pub skybox: Option<SkyboxRender>,
    pub clear_color: wgpu::Color,
    pub show_depth_map: bool,
    pub joystick_subsystem: JoystickSubsystem,
    pub _haptic_subsystem: HapticSubsystem,
    // pub renderizable_instances: HashMap<String, HashMap<String, InstanceData>>,
    pub renderizable_instances: HashMap<String, InstanceData>,
    pub throttling: Throttling,
    pub game_models: HashMap<String, ModelDataInstance>,
    pub light: Light,
    pub time: Timing,
    pub scene_openned: Option<String>,
    pub audio: Audio,
    pub render_physics: RenderPhysics,
    // Opt-in: None by default, assign before calling run() to show a splash screen.
    pub splash_screen: Option<SplashScreenConfig>,
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
        let ui = Ui::new(&renderer.device, &renderer.queue, &renderer.config, &renderer.glyphon.cache);
        let camera = CameraHandler::new(&renderer.device, &renderer.config);
        let light = Light::new(&renderer.device, &renderer.config, &camera);

        let render_pipeline_layout = renderer.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Render Pipeline Layout"),
            bind_group_layouts: &[
                &Texture::create_bind_group_layout(&renderer.device),
                &camera.bind_group_layout,
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

        let renderizable_instances = HashMap::new();
        let game_models = HashMap::new();

        // No environment loaded yet - each scene declares its own via
        // resources::apply_environment when it resets (see rendering::enviroment::environment).
        let skybox = None;
        let clear_color = environment::DEFAULT_CLEAR_COLOR;

        // physics rendering
        let render_physics = RenderPhysics::new(&renderer.device, &renderer.config, &camera);

        // Physics data
        let time = Timing::new();

        Ok(App {
            window_manager,
            renderer,
            scene_manager: SceneManager::new(),
            render_pipeline,
            ui,
            camera,
            skybox,
            clear_color,
            show_depth_map: false,
            joystick_subsystem,
            renderizable_instances,
            throttling: Throttling { last_ui_update: Instant::now(), ui_update_interval: Duration::from_secs_f32(1.0/120.0) },
            _haptic_subsystem: haptic_subsystem,
            game_models,
            light,
            time,
            scene_openned: None,
            audio: Audio::new(),
            render_physics,
            splash_screen: None,
        })
    }

    pub fn resize(&mut self) {
        let width = self.window_manager.current_display.w as u32;
        let height = self.window_manager.current_display.h as u32;

        self.renderer.resize(width, height);
        self.camera.resize(width, height);
    }

    pub fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
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

    // Rebuilds UI vertex/index/text buffers from the current node tree - only when
    // something actually marked the UI dirty, so idle frames skip it entirely.
    fn prepare_ui_content(&mut self) {
        if !self.ui.has_changed {
            return;
        }

        let mut text_areas: Vec<TextArea> = Vec::new();

        self.ui.ui_rendering.vertices.clear();
        self.ui.ui_rendering.num_vertices = 0;

        self.ui.ui_rendering.indices.clear();
        self.ui.ui_rendering.num_indices = 0;

        self.ui.ui_rendering.image_quads.clear();

        for (_key, ui_node) in &mut self.ui.renderizable_elements {
            // Debug bounds overlay (F2) - has to run before node_content_preparation
            // below, not after: that call's returned TextAreas keep *ui_node mutably
            // borrowed for as long as they're alive (all the way to text_areas being
            // consumed at the end of this function), so nothing else can borrow
            // *ui_node again afterward. Reading the rect from the start of this frame
            // (one frame stale for a node whose layout is still actively changing)
            // instead of after this frame's update is an invisible tradeoff for a
            // debug-only overlay.
            if self.ui.debug_bounds {
                ui_node.debug_bounds_preparation(&self.window_manager.size, &mut self.ui.ui_rendering);
            }

            let (textareas_to_merge, _vertices_to_add, _indices_to_add) = ui_node.node_content_preparation(&self.window_manager.size, &mut self.ui.ui_rendering, &mut self.ui.text.font_system, self.time.delta_time);
            text_areas.extend(textareas_to_merge);
        }

        // Only update buffers if we have data
        if !self.ui.ui_rendering.vertices.is_empty() {
            self.renderer.queue.write_buffer(&self.ui.ui_rendering.vertex_buffer, 0, bytemuck::cast_slice(self.ui.ui_rendering.vertices.as_slice()));
        }
        if !self.ui.ui_rendering.indices.is_empty() {
            self.renderer.queue.write_buffer(&self.ui.ui_rendering.index_buffer, 0, bytemuck::cast_slice(&self.ui.ui_rendering.indices));
        }

        // Only prepare text if we have text areas
        if !text_areas.is_empty() {
            self.ui.text.text_renderer.prepare(&self.renderer.device, &self.renderer.queue, &mut self.ui.text.font_system, &mut self.ui.text.text_atlas, &self.renderer.glyphon.viewport, text_areas, &mut self.ui.text.text_cache).unwrap();
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
                // wants a skybox has to call apply_environment itself in reset/new.
                self.skybox = None;
                self.clear_color = environment::DEFAULT_CLEAR_COLOR;

                // Same for UI - Ui::load_ui/add_to_ui only ever insert, they never
                // clear, so without this whatever the previous scene added would just
                // keep piling up/leaking into scenes that never asked for it. A scene
                // that wants UI has to (re)build it itself in new(), same contract as
                // the skybox above.
                self.ui.renderizable_elements.clear();
                self.ui.has_changed = true;

                // The scene doesn't exist yet at this point - SceneManager::create_scene
                // only registered its constructor. Clone the factory out first (rather
                // than borrowing self.scene_manager.factories directly) so calling it
                // with &mut self below doesn't conflict with that borrow.
                if let Some(factory) = self.scene_manager.factory_for(&active) {
                    // Physics doesn't carry over between scenes either - stop
                    // whatever was running, then start whatever the new scene wants
                    // (if anything). Scenes that don't override Scene::physics get
                    // None here and simply never spin up a thread.
                    if let Some(old_physics) = physics_data_channel.take() {
                        let _ = old_physics.request_data_tx.send(PhysicsCommand::Shutdown);
                    }

                    // This is the only place any scene's real constructor ever runs,
                    // and only for the one actually becoming active.
                    let scene = factory(&mut self);

                    physics_data_channel = scene.fixed_update(&self).map(|(level_path, physics_tick)| {
                        physics_handling(&self.renderer.device, &self.renderer.config, &self.camera, level_path, physics_tick)
                    });

                    self.scene_manager.active_scene = Some(scene);
                } else {
                    eprintln!("No scene registered for state '{}'", active);
                }
                self.scene_manager.reset = false;
            } else {
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
                for (_key, renderizable) in &mut self.renderizable_instances {
                    if let Some(physics_data) = physics_data.get(&_key.to_string()) {
                        renderizable.instance.transform.position = physics_data.translation;
                        renderizable.instance.transform.rotation = nalgebra::Unit::new_normalize(physics_data.rotation);
                    }
                }

                // Tick the active scene - taken out of its slot first so there's no
                // conflicting borrow with the &mut self it needs, put back once done.
                if let Some(mut scene) = self.scene_manager.active_scene.take() {
                    let mut ctx = FrameContext {
                        app_state: &mut app_state,
                        event_pump: &mut event_pump,
                        plane_control_tx: physics_data_channel.as_ref().map(|physics| &physics.plane_control_tx),
                        physics_data: &physics_data,
                        debug_physics: &debug_physics,
                    };
                    scene.update(&mut self, &mut ctx);
                    self.scene_manager.active_scene = Some(scene);
                } else {
                    eprintln!("No active scene to update");
                }

                // Update instance buffers efficiently - group by model type
                let camera_position = self.camera.active().camera.position.coords;
                let mut model_instances: HashMap<String, Vec<InstanceRaw>> = HashMap::new();

                for (_key, renderizable) in &self.renderizable_instances {
                    model_instances
                        .entry(renderizable.model_ref.clone())
                        .or_insert_with(Vec::new)
                        .push(renderizable.instance.transform.to_raw(camera_position));
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
                if let Some(sun) = self.renderizable_instances.get("sun") {
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

                self.camera.update_buffer(&self.renderer.queue);
                self.renderer.queue.write_buffer(&self.renderer.depth_render.near_far_buffer, 0, bytemuck::cast_slice(&[self.renderer.depth_render.near_far_uniform]));
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