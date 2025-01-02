use std::time::{Duration, Instant};
use std::collections::HashMap;
use std::env;

use rapier3d::prelude::{CCDSolver, ColliderBuilder, ColliderSet, CollisionPipeline, DefaultBroadPhase, ImpulseJointSet, IntegrationParameters, IslandManager, MultibodyJointSet, NarrowPhase, PhysicsPipeline, QueryPipeline, RigidBodyBuilder, RigidBodySet};
use sdl2::mixer::{self, InitFlag, AUDIO_S16LSB, DEFAULT_CHANNELS};
use wgpu::{BindGroupLayout, BindGroupLayoutDescriptor, Buffer, Device, DeviceDescriptor, Features, InstanceDescriptor, Limits, Queue, RenderPassDepthStencilAttachment, Surface, SurfaceConfiguration, TextureUsages};
use sdl2::{video::DisplayMode, joystick::Joystick, JoystickSubsystem, GameControllerSubsystem, HapticSubsystem, video::Window, Sdl, render::Canvas, controller::GameController};
use glyphon::{Resolution, TextArea};
use nalgebra:: {Unit, Vector3};
use wgpu::util::DeviceExt;
use rapier3d::na::vector;
use ron::from_str;
use tokio::task;

use crate::audio::audio::Audio;
use crate::rendering::instance_management::{InstanceData, InstanceRaw, ModelDataInstance, PhysicsData};
use crate::rendering::physics_rendering::RenderPhysics;
use crate::game_nodes::game_object::{self, GameObject};
use crate::rendering::model::{self, DrawModel, Vertex};
use crate::primitive::manual_vertex::ManualVertex;
use crate::rendering::depth_renderer::DepthRender;
use crate::rendering::camera::CameraRenderizable;
use crate::rendering::textures::Texture;
use crate::rendering::vertex::VertexUi;
use crate::game_nodes::timing::Timing;
use crate::rendering::rendering_utils;
use crate::game_nodes::scene::Scene;
use crate::rendering::light::Light;
use crate::rendering::ui::Ui;

use crate::gameplay::{main_menu, plane_selection, play};
use crate::ui::ui_node::{UiNode, UiNodeContent};
use crate::resources::{self, load_level};
use crate::utils::lerps::lerp_vector3;

pub enum GameState {
    Playing,
    MainMenu,
    SelectingPlane
}

pub struct AppState {
    pub is_running: bool,
    pub state: GameState,
    pub reset: bool
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
    pub context: Sdl,
    pub size: Size,
    pub canvas: Canvas<Window>,
    pub current_display: DisplayMode,
    pub surface: Surface,
    pub queue: Queue,
    pub device: Device,
    pub config: SurfaceConfiguration,
    pub render_pipeline: wgpu::RenderPipeline,
    pub ui: Ui,
    pub camera: CameraRenderizable,
    pub depth_texture: Texture,
    pub depth_render: DepthRender,
    pub show_depth_map: bool,
    pub controller_subsystem: GameControllerSubsystem,
    pub joystick_subsystem: JoystickSubsystem,
    pub _haptic_subsystem: HapticSubsystem,
    // pub renderizable_instances: HashMap<String, HashMap<String, InstanceData>>,
    pub renderizable_instances: HashMap<String, InstanceData>,
    pub throttling: Throttling,
    pub transform_bind_group_layout: BindGroupLayout,
    pub game_models: HashMap<String, ModelDataInstance>,
    pub light: Light,
    pub physics: Physics,
    pub time: Timing,
    pub scene_openned: Option<String>,
    pub audio: Audio
}

pub struct Physics {
    pub physics_pipeline: PhysicsPipeline,
    pub colission_pipeline: CollisionPipeline,
    pub query_pipeline: QueryPipeline,
    pub gravity: Vector3<f32>,
    
    // This values will save the rigidbodies and colliders
    pub rigidbody_set: RigidBodySet, 
    pub collider_set: ColliderSet,

    // for rendering forces
    pub render_physics: RenderPhysics,
}

impl App {
    pub async fn new(title: &str, ext_width: Option<u32>, ext_height: Option<u32>) -> App{
        // base sdl2
        let context = sdl2::init().expect("SDL2 wasn't initialized");
        let video_susbsystem = context.video().expect("The Video subsystem wasn't initialized");
        // let _audio_subsystem = context.audio().expect("The audio subsystem didnt loaded right");

        // so the mouse position gets setted inside the camer (the  mouse can go further than the window size)
        context.mouse().set_relative_mouse_mode(true);

        let controller_subsystem = context.game_controller().unwrap();
        let joystick_subsystem = context.joystick().unwrap();
        let haptic_subsystem = context.haptic().unwrap();

        let current_display = video_susbsystem.current_display_mode(0).unwrap();
        
        let width = match ext_width {
            Some(w) => w,
            None => current_display.w as u32,
        };
        let height =  match ext_height {
            Some(h) => h,
            None => current_display.h as u32,
        };

        env::set_var("SDL_VIDEO_MINIMIZE_ON_FOCUS_LOSS", "0");

        let window: Window = video_susbsystem.window(title, width, height as u32).vulkan().fullscreen().build().expect("The window wasn't created");
        
        let instance = wgpu::Instance::new(InstanceDescriptor::default());
        let surface = unsafe { instance.create_surface(&window).unwrap() };

        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default() // remember that this set every other parameter as their default values
        }).await.unwrap();

        let (device, queue) = adapter.request_device(
            &DeviceDescriptor { 
                label: None, 
                features: Features::empty(), 
                limits: Limits::default() }
            , None).await.unwrap();
        
        let mut canvas = window.into_canvas().accelerated().build().expect("the canvas wasn't builded");

        canvas.set_blend_mode(sdl2::render::BlendMode::Blend);

        // Surface settings
        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps.formats;

        let config = wgpu::SurfaceConfiguration {
            usage: TextureUsages::RENDER_ATTACHMENT,
            format: surface_format[0],
            width,
            height,
            present_mode: wgpu::PresentMode::AutoNoVsync,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
        };

        surface.configure(&device, &config);
        // Surface settings

        // depth
        let depth_texture = Texture::create_depth_texture(&device, &config, "depth_texture");
        // depth

        let transform_bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("transform_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        // The bindgroup describes resources and how the shader will access to them
        let texture_bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("texture_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        // rendering elements
        let ui = Ui::new(&device, &queue, &config);
        let camera = CameraRenderizable::new(&device, &config);
        let light = Light::new(&device, &config, &camera);

        let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Render Pipeline Layout"),
            bind_group_layouts: &[
                &texture_bind_group_layout,
                &camera.bind_group_layout,
                &transform_bind_group_layout,
                &light.rendering_data.bind_group_layout
            ],
            push_constant_ranges: &[],
        });
        
        // SHADERING PROCESS 
        
        let render_pipeline = {
            let shader = wgpu::ShaderModuleDescriptor {
                label: Some("Normal Shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("shaders/depth.wgsl").into()),
            };
            
            rendering_utils::create_render_pipeline(
                &device,
                &render_pipeline_layout,
                config.format,
                Some(Texture::DEPTH_FORMAT),
                &[model::ModelVertex::desc(), InstanceRaw::desc()],
                shader,
            )
        };

        let renderizable_instances = HashMap::new();
        let game_models = HashMap::new();
        let depth_render = DepthRender::new(&device, &config);

        // physics rendering
        let render_physics = RenderPhysics::new(&device, &config, &camera);

        // Physics data
        let physics = Physics {
            physics_pipeline: PhysicsPipeline::new(),
            colission_pipeline: CollisionPipeline::new(),
            query_pipeline: QueryPipeline::new(),
            gravity: Vector3::new(0.0, -9.81, 0.0),
            rigidbody_set: RigidBodySet::new(),
            collider_set: ColliderSet::new(),
            render_physics,
        };

        let time = Timing::new();

        App {
            current_display,
            context,
            size: Size {width, height},
            canvas,
            surface,
            queue,
            device,
            config,
            render_pipeline,
            ui,
            camera,
            depth_texture,
            depth_render,
            show_depth_map: false,
            controller_subsystem,
            joystick_subsystem,
            renderizable_instances,
            throttling: Throttling { last_ui_update: Instant::now(), ui_update_interval: Duration::from_secs_f32(1.0/120.0), last_controller_update: Instant::now(), controller_update_interval: Duration::from_secs_f32(1.0/400.0) },
            _haptic_subsystem: haptic_subsystem,
            transform_bind_group_layout,
            game_models,
            light,
            physics,
            time,
            scene_openned: None,
            audio: Audio::new()
        }
    }

    pub fn resize(&mut self) {
        self.config.width = self.current_display.w as u32;
        self.config.height = self.current_display.h as u32;

        self.surface.configure(&self.device, &self.config);
        self.depth_render.resize(&self.device, &self.config);
        self.camera.projection.resize(self.size.width, self.size.height);

        self.depth_texture = Texture::create_depth_texture(&self.device, &self.config, "depth_texture");
    }

    // Pass to a especific element the values of "render pass" to the self structure, so they are made once and then used here
    // Find a way to make that i can "set when the ui elements change"
    // find a way to optimize the non transparent object rendering
    pub fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        // UI
        if self.ui.has_changed {
            let mut text_areas: Vec<TextArea> = Vec::new();

            self.ui.ui_rendering.vertices.clear();
            self.ui.ui_rendering.num_vertices = 0;
            
            self.ui.ui_rendering.indices.clear();
            self.ui.ui_rendering.num_indices = 0;

            for (_key, list) in &mut self.ui.renderizable_elements {
                match list {
                    crate::rendering::ui::UiContainer::Tagged(hash_map) => {
                        for (_key, ui_node) in hash_map {
                            let (textareas_to_merge, _vertices_to_add, _indices_to_add) = ui_node.node_content_preparation(&self.size, &mut self.ui.ui_rendering, &mut self.ui.text.font_system, self.time.delta_time);
                            text_areas.extend(textareas_to_merge);
                        }
                    },
                    crate::rendering::ui::UiContainer::Untagged(vec) => {
                        for ui_node in vec {
                            let (textareas_to_merge, _vertices_to_add, _indices_to_add) = ui_node.node_content_preparation(&self.size, &mut self.ui.ui_rendering, &mut self.ui.text.font_system, self.time.delta_time);
                            text_areas.extend(textareas_to_merge);
                        }
                    },
                }
            }
            
            self.queue.write_buffer(&self.ui.ui_rendering.vertex_buffer, 0, bytemuck::cast_slice(self.ui.ui_rendering.vertices.as_slice()));
            self.queue.write_buffer(&self.ui.ui_rendering.index_buffer, 0, bytemuck::cast_slice(&self.ui.ui_rendering.indices));
            
            self.ui.text.text_renderer.prepare(&self.device, &self.queue, &mut self.ui.text.font_system, &mut self.ui.text.text_atlas, Resolution {width: self.size.width, height: self.size.height}, text_areas, &mut self.ui.text.text_cache).unwrap();
            // self.ui.has_changed = false
        }
        
        // UI

        // WGPU
        let output = self.surface.get_current_texture()?;
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default()); // this let us to control how render code interacts with textures
        
        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });
        
        // Opaque
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor { 
                label: Some("Render Pass"), 
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { // nuestro render pass limpia la pantalla y setea un color de fondo
                            r: 0.1,
                            g: 0.2,
                            b: 0.3,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &self.depth_render.texture.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0), // limpiamos nuestro "depth stencil para este estado"
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
            });


            render_pass.set_pipeline(&self.render_pipeline);

            for (key, renderizable) in &self.renderizable_instances {
                if let Some(model_data) = self.game_models.get(&renderizable.model_ref)  {
                    if key != "sun" {
                        render_pass.set_vertex_buffer(1, model_data.instance_buffer.slice(..));
                        render_pass.draw_model_instanced_from_list(&model_data.model, 0..model_data.instance_count as u32, &self.camera.bind_group, &self.light.rendering_data.bind_group, &"opaque".to_string()); // usamos la funcion que renderiza los objetos opacos
                    }
                }
            }
        }

        // transparency render pass

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor { 
                label: Some("Render Pass"), 
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load, // se carga el color anteriormente seteado
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &self.depth_render.texture.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load, // cargamos la informacion de profundidad anteriormente definida
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            
            render_pass.set_pipeline(&self.render_pipeline);
            
            for (_key, renderizable) in &self.renderizable_instances {
                if let Some(model_data) = self.game_models.get(&renderizable.model_ref)  {
                    render_pass.set_vertex_buffer(1, model_data.instance_buffer.slice(..));
                    render_pass.draw_model_instanced_from_list(&model_data.model, 0..model_data.instance_count as u32, &self.camera.bind_group, &self.light.rendering_data.bind_group, &"transparent".to_string()); // usamos la funcion que renderiza los objetos opacos
                }
            }
        }
        
        // Ui Pass
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("UI Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load, // Load to preserve 3D rendering results
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None, // No depth-stencil for UI
                occlusion_query_set: None,
                timestamp_writes: None,
            });

            render_pass.set_pipeline(&self.physics.render_physics.render_pipeline);
            render_pass.set_bind_group(0, &self.physics.render_physics.bind_group, &[]);
            render_pass.set_bind_group(1, &self.camera.bind_group, &[]);
    
            if !self.show_depth_map {
                // Prepare vertex and index buffers specifically for physics rendering
                let vertices: Vec<ManualVertex> = self.physics.render_physics.renderizable_lines.iter()
                .flat_map(|line| line.to_vec())
                .collect();

                if !vertices.is_empty() {
                    self.physics.render_physics.vertex_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Updated ManualVertex Buffer"),
                        contents: bytemuck::cast_slice(&vertices),
                        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    });

                    // Update index buffer for all lines
                    let mut indices = Vec::new();
                    for i in 0..self.physics.render_physics.renderizable_lines.len() {
                        let base_index = (i * 2) as u16; // Each line has two vertices
                        indices.push(base_index);
                        indices.push(base_index + 1);
                    }

                    if !indices.is_empty() {
                        self.physics.render_physics.index_buffer = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("Index Buffer"),
                            contents: bytemuck::cast_slice(&indices),
                            usage: wgpu::BufferUsages::INDEX,
                        });

                        // Set vertex and index buffers once before drawing
                        render_pass.set_vertex_buffer(0, self.physics.render_physics.vertex_buffer.slice(..));
                        render_pass.set_index_buffer(self.physics.render_physics.index_buffer.slice(..), wgpu::IndexFormat::Uint16);

                        // Draw all lines
                        render_pass.draw_indexed(0..(indices.len() as u32), 0, 0..1);
                    }
                }
                
            }
            // Clear the line vertices after drawing
            self.physics.render_physics.renderizable_lines.clear();

            render_pass.set_pipeline(&self.ui.ui_pipeline);
            render_pass.set_vertex_buffer(0, self.ui.ui_rendering.vertex_buffer.slice(..));
            render_pass.set_index_buffer(self.ui.ui_rendering.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            render_pass.draw_indexed(0..self.ui.ui_rendering.num_indices, 0, 0..1);

            self.ui.text.text_renderer.render(&self.ui.text.text_atlas, &mut render_pass).unwrap();
        }

        /* 
        if self.show_depth_map {
            self.depth_render.render(&view, &mut encoder);
        }
        */

        // we have the render pass inside the {} so we can do the submit to the queue, we can also drop the render pass if you prefeer
        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();
        self.ui.text.text_atlas.trim();

        Ok(())
    }

    pub fn update(mut self) {
        // SDL2
        let mut app_state = AppState { is_running: true, state: GameState::Playing, reset: true};
        let mut event_pump = self.context.event_pump().unwrap();

        let mut play = play::GameLogic::new(&mut self);

        let _main_menu = main_menu::GameLogic::new(&mut self);
        let _selecting_plane = plane_selection::GameLogic::new(&mut self);

        let mut controller = Self::open_first_available_controller(&self.controller_subsystem);
        let _joystick = Self::open_first_avalible_joystick(&self.joystick_subsystem);
        
        // Phyisics timealization
        const FIXED_TIMESTEP: f32 = 1.0 / 60.0; // Fixed timestep for 60 FPS
        let mut accumulator = 0.0;

        let integration_parameters = IntegrationParameters { dt: FIXED_TIMESTEP, ..Default::default() };
        let mut island_manager = IslandManager::new();
        let mut broad_phase = DefaultBroadPhase::new();
        let mut narrow_phase = NarrowPhase::new();
        let mut impulse_joint_set = ImpulseJointSet::new();
        let mut multibody_joint_set = MultibodyJointSet::new();
        let mut ccd_solver = CCDSolver::new();
        let physics_hooks = ();
        let event_handler = ();

        loop {
            self.time.update();
            
            if !app_state.is_running {
                break
            }

            match app_state.state {
                GameState::Playing => {
                    if app_state.reset {
                        load_level(&mut self, "./assets/scenes/test_chamber".to_owned());
                        play = play::GameLogic::new(&mut self);
                        app_state.reset = false;
                    } else {
                        accumulator += self.time.delta_time;

                        while accumulator >= FIXED_TIMESTEP {
                            self.physics.physics_pipeline.step(
                                &play.gravity,
                                &integration_parameters,
                                &mut island_manager,
                                &mut broad_phase,
                                &mut narrow_phase,
                                &mut self.physics.rigidbody_set,
                                &mut self.physics.collider_set,
                                &mut impulse_joint_set,
                                &mut multibody_joint_set,
                                &mut ccd_solver,
                                Some(&mut self.physics.query_pipeline),
                                &physics_hooks,
                                &event_handler,
                            );  

                            if let Some(instance) = self.renderizable_instances.get_mut("player") {
                                match &instance.physics_data {
                                    Some(physics_data) => {
                                        if let Some(rigidbody) = self.physics.rigidbody_set.get_mut(physics_data.rigidbody_handle) {
                                            play.plane_systems.flight_data.g_meter = play.calculate_g_forces(rigidbody, self.time.delta_time);
                                        }
                                    },
                                    None => {}
                                }
                            }

                            accumulator -= FIXED_TIMESTEP
                        }

                        play.update( &mut app_state, &mut event_pump, &mut self, &mut controller);
                        
                        for (model_key, model) in &self.game_models {
                            let mut offset_index = 0;
                        
                            for (_key, renderizable) in &mut self.renderizable_instances {
                                if renderizable.model_ref == *model_key {
                                    match &renderizable.physics_data {
                                        Some(physics_info) => {
                                            if let Some(rigid_body) = self.physics.rigidbody_set.get(physics_info.rigidbody_handle) {
                                                // Update position and rotation
                                                // println!("key: {}, position: {}",_key,  renderizable.instance.transform.position);
                                                renderizable.instance.transform.position = *rigid_body.translation();
                                                renderizable.instance.transform.rotation = *rigid_body.rotation();
                        
                                                // Calculate unique offset based on instance count
                                                let offset = (offset_index * std::mem::size_of::<InstanceRaw>()) as u64;
                                                self.queue.write_buffer(&model.instance_buffer, offset, bytemuck::cast_slice(&[renderizable.instance.transform.to_raw()]));
                        
                                                offset_index += 1; // Increment for next unique instance
                                            }
                                        },
                                        None => {},
                                    }
                                }
                            }
                        }

                        // lighting update
                        if let Some(sun) = self.renderizable_instances.get("sun") {
                            self.light.uniform.position = (sun.instance.transform.position.x, sun.instance.transform.position.y, sun.instance.transform.position.z).into();
                            match &sun.instance.metadata.lighting {
                                Some(lighting_data) => {
                                    self.light.uniform.color = lighting_data.color.into();
                                },
                                None => {},
                            }
                        }
                        self.queue.write_buffer(&self.light.rendering_data.buffer, 0, bytemuck::cast_slice(&[self.light.uniform]));
                        // lighting update

                        self.camera.uniform.update_view_proj(&self.camera.camera, &self.camera.projection);
                        self.queue.write_buffer(&self.camera.buffer, 0, bytemuck::cast_slice(&[self.camera.uniform]));
                        self.queue.write_buffer(&self.depth_render.near_far_buffer, 0, bytemuck::cast_slice(&[self.depth_render.near_far_uniform]));
                    }
                },
                GameState::MainMenu => {},
                GameState::SelectingPlane => {}
            }

            match self.render() {
                Ok(_) => {},
                Err(wgpu::SurfaceError::Outdated) => { 
                    self.resize()
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
            print!("{}: {}", index, joy.name());
            return Some(joy)
        }
        None
    }

    
}