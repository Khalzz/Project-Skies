use std::time::{Duration, Instant};
use std::collections::HashMap;
use std::env;

use wgpu::{BindGroupLayout, BindGroupLayoutDescriptor, Device, DeviceDescriptor, Features, InstanceDescriptor, Limits, Queue, Surface, SurfaceConfiguration, TextureUsages};
use sdl2::{joystick::Joystick, JoystickSubsystem, GameControllerSubsystem, HapticSubsystem, controller::GameController};
use glyphon::{Cache, Resolution, TextArea, Viewport};

use crate::engine::audio::audio::Audio;
use crate::engine::physics::physics::{physics_handling, DebugPhysicsMessageType, PhysicsDataTransmission};
use crate::engine::physics::physics_handler::{RenderMessage, PhysicsCommand};
use crate::engine::rendering::enviroment::skybox_renderer::SkyboxRender;
use crate::engine::rendering::enviroment::environment;
use crate::engine::rendering::instance_management::{InstanceData, InstanceRaw, ModelDataInstance};
use crate::engine::rendering::render_pipeline::depth_renderer::DepthRender;
use crate::engine::rendering::camera::CameraRenderizable;
use crate::engine::rendering::models::textures::Texture;
use crate::engine::game_nodes::timing::Timing;
use crate::engine::rendering::enviroment::light::Light;
use crate::engine::rendering::models::model::{self, Mesh, Model, Vertex};
use crate::engine::rendering::renderer::Renderer;
use crate::engine::scene_manager::scene::{Scene, ScenePool, FrameContext, GameState, SceneManager};
use crate::engine::input::input::InputSubsystem;
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
    pub last_controller_update: Instant,
    pub controller_update_interval: Duration,
}

pub struct App {
    pub window_manager: WindowManager,
    pub renderer: Renderer,
    // Placeholder pool/starting scene at construction time - the real configuration
    // is assigned by the caller (see main.rs) before App::run is called.
    pub scene_manager: SceneManager,
    pub render_pipeline: wgpu::RenderPipeline,
    pub ui: Ui,
    pub camera: CameraRenderizable,
    // Configured per scene via resources::apply_environment (called from Scene::reset),
    // not loaded automatically - None means the scene just wants clear_color.
    pub skybox: Option<SkyboxRender>,
    pub clear_color: wgpu::Color,
    pub show_depth_map: bool,
    pub controller_subsystem: GameControllerSubsystem,
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

        let controller_subsystem = window_manager.context.game_controller().unwrap();
        let joystick_subsystem = window_manager.context.joystick().unwrap();
        let haptic_subsystem = window_manager.context.haptic().unwrap();

        // WGPU initialization
        let renderer = Renderer::new(&window_manager).await?;

        // rendering elements
        let ui = Ui::new(&renderer.device, &renderer.queue, &renderer.config, &renderer.glyphon.cache);
        let camera = CameraRenderizable::new(&renderer.device, &renderer.config);
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
            scene_manager: SceneManager::new(HashMap::new(), GameState::Playing),
            render_pipeline,
            ui,
            camera,
            skybox,
            clear_color,
            show_depth_map: false,
            controller_subsystem,
            joystick_subsystem,
            renderizable_instances,
            throttling: Throttling { last_ui_update: Instant::now(), ui_update_interval: Duration::from_secs_f32(1.0/120.0), last_controller_update: Instant::now(), controller_update_interval: Duration::from_secs_f32(1.0/400.0) },
            _haptic_subsystem: haptic_subsystem,
            game_models,
            light,
            time,
            scene_openned: None,
            audio: Audio::new(),
            render_physics,
        })
    }

    pub fn resize(&mut self) {
        let width = self.window_manager.current_display.w as u32;
        let height = self.window_manager.current_display.h as u32;

        self.renderer.resize(width, height);
        self.camera.projection.resize(width, height);
    }

    pub fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        self.prepare_ui_content();

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

        for (_key, ui_node) in &mut self.ui.renderizable_elements {
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
        self.ui.has_changed = false;
    }

    // self.scene_manager is configured by the caller (see main.rs) before run() is
    // called - App only needs to know about the Scene trait / ScenePool, not any
    // concrete scene type.
    pub fn run(mut self) {
        // SDL2
        let mut app_state = AppState { is_running: true };
        let mut event_pump = self.window_manager.context.event_pump().unwrap();

        let mut controller = Self::open_first_available_controller(&self.controller_subsystem);
        let _joystick = Self::open_first_avalible_joystick(&self.joystick_subsystem);

        // Started/stopped per scene switch below, based on the active scene's
        // Scene::physics() - None until a physics-wanting scene resets.
        let mut physics_data_channel: Option<PhysicsDataTransmission> = None;

        let mut input_subsystem = InputSubsystem::new(include_str!("../settings/input.ron"));

        let mut debug_physics: Vec<DebugPhysicsMessageType> = Vec::new();

        loop {
            // Relevant subsystems update
            self.time.update();
            input_subsystem.update(&mut event_pump, self.time.delta_time, false);

            if !app_state.is_running {
                // Send shutdown command to physics thread, if one is running
                if let Some(physics) = &physics_data_channel {
                    let _ = physics.request_data_tx.send(PhysicsCommand::Shutdown);
                }
                break
            }

            if self.scene_manager.reset {
                let active = self.scene_manager.active;

                // Environment doesn't carry over between scenes (same as Godot: no
                // WorldEnvironment in the new scene falls back to the default, it
                // doesn't inherit whatever the previous scene had) - a scene that
                // wants a skybox has to call apply_environment itself in reset/new.
                self.skybox = None;
                self.clear_color = environment::DEFAULT_CLEAR_COLOR;

                // Scenes need &mut App to reset themselves, but the pool they're
                // stored in lives on App too - take the scene out first so there's
                // no conflicting borrow, then put it back once it's done with self.
                if let Some(mut scene) = self.scene_manager.scenes.remove(&active) {
                    // Physics doesn't carry over between scenes either - stop
                    // whatever was running, then start whatever the new scene wants
                    // (if anything). Scenes that don't override Scene::physics get
                    // None here and simply never spin up a thread.
                    if let Some(old_physics) = physics_data_channel.take() {
                        let _ = old_physics.request_data_tx.send(PhysicsCommand::Shutdown);
                    }

                    scene.reset(&mut self);

                    physics_data_channel = scene.physics(&self).map(|(level_path, physics_tick)| {
                        physics_handling(&self.renderer.device, &self.renderer.config, &self.camera, level_path, physics_tick)
                    });

                    self.scene_manager.scenes.insert(active, scene);
                } else {
                    eprintln!("No scene registered for state '{:?}'", active);
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
                    if input_subsystem.is_just_pressed("toggle_debug") {
                        self.render_physics.visible = !self.render_physics.visible;
                        if let Err(e) = physics.request_data_tx.send(PhysicsCommand::ToggleDebug) {
                            eprintln!("Failed to send toggle debug command: {}", e);
                        }
                    }

                    // Toggle physics pause with F12
                    if input_subsystem.is_just_pressed("toggle_pause") {
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
                if input_subsystem.is_just_pressed("toggle_console") {
                    crate::engine::tooling::debug_console::toggle_console();
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

                // Tick whichever scene is currently active - same remove/reinsert
                // dance as the reset branch, since tick also needs &mut App.
                let active_scene_key = self.scene_manager.active;
                if let Some(mut scene) = self.scene_manager.scenes.remove(&active_scene_key) {
                    let mut ctx = FrameContext {
                        app_state: &mut app_state,
                        event_pump: &mut event_pump,
                        controller: &mut controller,
                        input_subsystem: &input_subsystem,
                        plane_control_tx: physics_data_channel.as_ref().map(|physics| &physics.plane_control_tx),
                        physics_data: &physics_data,
                        debug_physics: &debug_physics,
                    };
                    scene.tick(&mut self, &mut ctx);
                    self.scene_manager.scenes.insert(active_scene_key, scene);
                } else {
                    eprintln!("No scene registered for state '{:?}'", active_scene_key);
                }

                // Update instance buffers efficiently - group by model type
                let camera_position = self.camera.camera.position.coords;
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

                self.camera.uniform.update_view_proj(&self.camera.camera, &self.camera.projection);
                self.renderer.queue.write_buffer(&self.camera.buffer, 0, bytemuck::cast_slice(&[self.camera.uniform]));
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

    

    fn open_first_available_controller(controller_subsystem: &GameControllerSubsystem) -> Option<GameController> {
        for id in 0..controller_subsystem.num_joysticks().unwrap() {
            if controller_subsystem.is_game_controller(id) {
                // println!("{}", controller_subsystem.name_for_index(id).unwrap());
                return Some(controller_subsystem.open(id).unwrap());
            }
        }
        None
    }

    fn open_first_avalible_joystick(joystick_subsystem: &JoystickSubsystem) -> Option<Joystick> {
        for index in 0..joystick_subsystem.num_joysticks().unwrap() {
            let joy = joystick_subsystem.open(index).unwrap();
            println!("{}: {}", index, joy.name());
            return Some(joy)
        }
        None
    }

    
}