use std::f32::consts::PI;
use std::fs::File;
use std::hash::Hash;
use std::time::{Duration, Instant};
use std::collections::HashMap;
use std::env;

use cgmath::Vector3;
use nalgebra::{Matrix3, Unit, UnitQuaternion};
use rapier3d::na::{vector, Vector};
use rapier3d::prelude::{BroadPhase, CCDSolver, ColliderBuilder, ColliderSet, CollisionPipeline, DefaultBroadPhase, ImpulseJointSet, IntegrationParameters, IslandManager, MultibodyJointSet, NarrowPhase, PhysicsPipeline, QueryPipeline, RigidBody, RigidBodyBuilder, RigidBodySet};
use ron::from_str;
use sdl2::joystick::Joystick;
use sdl2::{joystick, JoystickSubsystem};
use sdl2::{GameControllerSubsystem, HapticSubsystem, video::Window, Sdl, render::Canvas};
use sdl2::controller::GameController;
use sdl2::render::TextureCreator;
use sdl2::video::{DisplayMode, WindowContext};
use serde_json::{Map, Value};
use tokio::runtime::Runtime;
use tokio::task;
use wgpu::{BindGroup, BindGroupLayout, BindGroupLayoutDescriptor, Buffer, BufferDescriptor, Device, DeviceDescriptor, Features, InstanceDescriptor, Limits, Queue, RenderPassDepthStencilAttachment, RenderPipeline, Surface, SurfaceConfiguration, TextureUsages, VertexBufferLayout};
use wgpu::util::DeviceExt;
use glyphon::{Resolution, TextArea};


use crate::game_object::{self, GameObject, Scene, Transform};
use crate::primitive::manual_vertex::ManualVertex;
use crate::rendering::camera::CameraRenderizable;
use crate::rendering::depth_renderer::DepthRender;
use crate::rendering::instance_management::{InstanceData, InstanceRaw, LevelData, ModelDataInstance, PhysicsData};
use crate::rendering::model::{self, DrawLight, DrawModel, Vertex};
use crate::rendering::physics_rendering::RenderPhysics;
use crate::rendering::rendering_utils;
use crate::rendering::textures::Texture;
use crate::rendering::ui::UI;
use crate::rendering::vertex::VertexUi;

use crate::resources;
use crate::ui::button::Button;
use crate::gameplay::{main_menu, plane_selection, play};
use crate::primitive::rectangle;

pub enum GameState {
    Playing,
    MainMenu,
    SelectingPlane
}

// Lighting

// this will be a especific point in space that CAN BE COLORED (but we mainly will use white light)
#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct LightUniform {
    position: [f32; 3],
    // Due to uniforms requiring 16 byte (4 float) spacing, we need to use a padding field here
    _padding: u32,
    color: [f32; 3],
    // Due to uniforms requiring 16 byte (4 float) spacing, we need to use a padding field here
    _padding2: u32,
}

struct LightingData {
    pub light_bind_group_layout: BindGroupLayout,
    pub light_bind_group: BindGroup,
    pub light_uniform: LightUniform,
    pub light_buffer: Buffer,
    pub light_render_pipeline: RenderPipeline
}

// Lighting

pub struct AppState {
    pub is_running: bool,
    pub state: GameState,
    pub reset: bool
}

pub struct Size {
    pub width: u32,
    pub height: u32
}
pub struct MousePos {
    pub x: f64,
    pub y: f64,
}

pub struct Throttling {
    pub last_ui_update: Instant,
    pub ui_update_interval: Duration,
    pub last_controller_update: Instant,
    pub controller_update_interval: Duration,
}

pub struct App {
    last_frame: Instant,
    pub context: Sdl,
    pub size: Size,
    pub canvas: Canvas<Window>,
    pub current_display: DisplayMode,
    pub surface: Surface,
    pub queue: Queue,
    pub device: Device,
    pub config: SurfaceConfiguration,
    pub render_pipeline: wgpu::RenderPipeline,
    pub ui: UI,
    pub camera: CameraRenderizable,
    pub depth_texture: Texture,
    pub depth_render: DepthRender,
    pub show_depth_map: bool,
    pub controller_subsystem: GameControllerSubsystem,
    pub joystick_subsystem: JoystickSubsystem,
    pub _haptic_subsystem: HapticSubsystem,
    pub components: HashMap<String, Button>, // we should transform this to a hashmap to have a better access on what is inside of it
    pub dynamic_ui_components: HashMap<String, Vec<Button>>,
    pub renderizable_instances: HashMap<String, InstanceData>,
    pub mouse_pos: MousePos,
    pub throttling: Throttling,
    pub transform_bind_group_layout: BindGroupLayout,
    pub game_models: HashMap<String, ModelDataInstance>,
    pub lighting_data: LightingData,
    pub physics: Physics
}



pub struct Physics {
    pub physics_pipeline: PhysicsPipeline,
    pub colission_pipeline: CollisionPipeline,
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

        println!("{}", adapter.get_info().name);
        println!("{}", adapter.get_info().backend.to_str());

        let (device, queue) = adapter.request_device(
            &DeviceDescriptor { 
                label: None, 
                features: Features::empty(), 
                limits: Limits::default() }
            , None).await.unwrap();

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
                    // This should match the filterable field of the
                    // corresponding Texture entry above.
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        // Textures
        let ui = UI::new(&device, &queue, &config);

        // Camera
        let camera = CameraRenderizable::new(&device, &config);

        // LIGHTING
        let light_uniform = LightUniform {
            position: [0.0, 5.0, 0.0],
            _padding: 0,
            color: [1.0, 1.0, 1.0],
            _padding2: 0,
        };

         // We'll want to update our lights position, so we use COPY_DST
        let light_buffer = device.create_buffer_init(
            &wgpu::util::BufferInitDescriptor {
                label: Some("Light VB"),
                contents: bytemuck::cast_slice(&[light_uniform]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, // remember that copydst let us change later elements like position of objects
            }
        );

        let light_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
            label: None,
        });

        let light_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &light_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: light_buffer.as_entire_binding(),
            }],
            label: None,
        });
        // LIGHTING

        let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Render Pipeline Layout"),
            bind_group_layouts: &[
                &texture_bind_group_layout,
                &camera.bind_group_layout,
                &transform_bind_group_layout,
                &light_bind_group_layout
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

        let light_render_pipeline = {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Light Pipeline Layout"),
                bind_group_layouts: &[&camera.bind_group_layout, &light_bind_group_layout],
                push_constant_ranges: &[],
            });

            let shader = wgpu::ShaderModuleDescriptor {
                label: Some("Light Shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("shaders/light.wgsl").into()),
            };
            
            rendering_utils::create_render_pipeline(
                &device,
                &layout,
                config.format,
                Some(Texture::DEPTH_FORMAT),
                &[model::ModelVertex::desc()],
                shader,
            )
        };

        let mut canvas = window.into_canvas().accelerated().build().expect("the canvas wasn't builded");

        canvas.set_blend_mode(sdl2::render::BlendMode::Blend);

        let renderizable_instances = HashMap::new();
        let game_models = HashMap::new();
        let depth_render = DepthRender::new(&device, &config);
        let components = HashMap::new();
        let mut dynamic_ui_components = HashMap::new();

        dynamic_ui_components.insert("bandits".to_owned(), vec![]);
        
        // Dynamic static is for objects that move in the screen but their main position is based on something that never changes
        dynamic_ui_components.insert("dynamic_static".to_owned(), vec![]); 

        // physics rendering
        let render_physics = RenderPhysics::new(&device, &config, &camera);

        // Physics data
        let physics = Physics {
            physics_pipeline: PhysicsPipeline::new(),
            colission_pipeline: CollisionPipeline::new(),
            gravity: Vector3::new(0.0, -9.81, 0.0),
            rigidbody_set: RigidBodySet::new(),
            collider_set: ColliderSet::new(),
            render_physics,
        };

        

        App {
            last_frame: Instant::now(),
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
            components,
            mouse_pos: MousePos { x: 0.0, y: 0.0 },
            renderizable_instances,
            dynamic_ui_components,
            throttling: Throttling { last_ui_update: Instant::now(), ui_update_interval: Duration::from_secs_f32(1.0/400.0), last_controller_update: Instant::now(), controller_update_interval: Duration::from_secs_f32(1.0/400.0) },
            _haptic_subsystem: haptic_subsystem,
            transform_bind_group_layout,
            game_models,
            lighting_data: LightingData { light_bind_group_layout, light_bind_group, light_uniform, light_buffer, light_render_pipeline },
            physics
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

    pub fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        // UI
        let mut text_areas: Vec<TextArea> = Vec::new();
        let mut vertices: Vec<VertexUi> = Vec::new();
        let mut indices: Vec<u16> = Vec::new();
        let mut num_vertices = 0;
        let mut num_indices = 0;

        for (_key, button) in self.components.iter_mut() {
            let button_active = button.is_hovered(&self.mouse_pos);
            let button_vertices = button.rectangle.vertices(button_active, &self.size);
            vertices.extend_from_slice(&button_vertices);
            indices.extend_from_slice(&button.rectangle.indices(num_vertices));
            num_vertices += button_vertices.len() as u16;
            num_indices += rectangle::NUM_INDICES;
            text_areas.push(button.text.text_area(button_active));
        }

        for (_key, list) in self.dynamic_ui_components.iter_mut() {
            for button in list {
                let button_active = button.is_hovered(&self.mouse_pos);
                let button_vertices = button.rectangle.vertices(button_active, &self.size);

                vertices.extend_from_slice(&button_vertices);
                indices.extend_from_slice(&button.rectangle.indices(num_vertices));

                num_vertices += button_vertices.len() as u16;
                num_indices += rectangle::NUM_INDICES;

                text_areas.push(button.text.text_area(button_active));
            }
        }

        self.queue.write_buffer(&self.ui.ui_rendering.vertex_buffer, 0, bytemuck::cast_slice(vertices.as_slice()));
        self.queue.write_buffer(&self.ui.ui_rendering.index_buffer, 0, bytemuck::cast_slice(&indices));
        self.ui.text.text_renderer.prepare(&self.device, &self.queue, &mut self.ui.text.font_system, &mut self.ui.text.text_atlas, Resolution {width: self.size.width,height: self.size.height},text_areas,&mut self.ui.text.text_cache,).unwrap();
        // UI

        // WGPU
        let output = self.surface.get_current_texture()?;
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default()); // this let us to control how render code interacts with textures

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });

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

            let _ = &self.renderizable_instances.iter().for_each(|(key ,renderizable)| {
                match self.game_models.get(&renderizable.model_ref) {
                    Some(model_data) => {
                        if key != "sun" {
                            render_pass.set_vertex_buffer(1, model_data.instance_buffer.slice(..));
                            // render_pass.draw_light_model(&model_data.model, &self.camera.bind_group, &self.lighting_data.light_bind_group,);
                            render_pass.draw_model_instanced(&model_data.model, 0..model_data.instance_count as u32, &self.camera.bind_group, &self.lighting_data.light_bind_group); // usamos la funcion que renderiza los objetos opacos
                        }
                    },
                    None => {},
                }
            });

            render_pass.set_pipeline(&self.lighting_data.light_render_pipeline);

            if let Some(sun_model) = self.renderizable_instances.get("sun") {
                match self.game_models.get(&sun_model.model_ref) {
                    Some(model_data) => {
                        render_pass.set_vertex_buffer(1, model_data.instance_buffer.slice(..));
                        render_pass.draw_light_model(&model_data.model, &self.camera.bind_group, &self.lighting_data.light_bind_group,);
                    },
                    None => {},
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

            let _ = &self.renderizable_instances.iter().for_each(|(_key ,renderizable)| {
                match self.game_models.get(&renderizable.model_ref) {
                    Some(model_data) => {
                        render_pass.set_vertex_buffer(1, model_data.instance_buffer.slice(..));
                        render_pass.draw_transparent_model_instanced(&model_data.model , 0..model_data.instance_count as u32, &self.camera.bind_group, &self.lighting_data.light_bind_group); // usamos la funcion que renderiza los objetos opacos
                    },
                    None => {},
                }
            });
        }

        // physics render
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Physics Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
                    view: &self.depth_render.texture.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
            });
    
            // Set up the pipeline for physics rendering
            render_pass.set_pipeline(&self.physics.render_physics.render_pipeline);
            render_pass.set_bind_group(0, &self.physics.render_physics.bind_group, &[]);
            render_pass.set_bind_group(1, &self.camera.bind_group, &[]);
    
            // Prepare vertex and index buffers specifically for physics rendering
            let vertices: Vec<ManualVertex> = self.physics.render_physics.renderizable_lines.iter()
                .flat_map(|line| line.to_vec()) // Flatten pairs of vertices into a single vector
                .collect();
    
            if !vertices.is_empty() {
                // Update vertex buffer only if there are vertices
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
    
            // Clear the line vertices after drawing
            self.physics.render_physics.renderizable_lines.clear();
        }

        if self.show_depth_map {
            self.depth_render.render(&view, &mut encoder);
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
    
            render_pass.set_pipeline(&self.ui.ui_pipeline);
            render_pass.set_vertex_buffer(0, self.ui.ui_rendering.vertex_buffer.slice(..));
            render_pass.set_index_buffer(self.ui.ui_rendering.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            render_pass.draw_indexed(0..num_indices, 0, 0..1);
    
            self.ui.text.text_renderer.render(&self.ui.text.text_atlas, &mut render_pass).unwrap();
        }
        
        if self.show_depth_map {
            self.depth_render.render(&view, &mut encoder);
        }

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

        let mut main_menu = main_menu::GameLogic::new(&mut self);
        let mut selecting_plane = plane_selection::GameLogic::new(&mut self);

        let mut controller = Self::open_first_available_controller(&self.controller_subsystem);
        let _joystick = Self::open_first_avalible_joystick(&self.joystick_subsystem);
        let mut delta_time: f32;

        let integration_parameters = IntegrationParameters::default();
        let mut island_manager = IslandManager::new();
        let mut broad_phase = DefaultBroadPhase::new();
        let mut narrow_phase = NarrowPhase::new();
        let mut impulse_joint_set = ImpulseJointSet::new();
        let mut multibody_joint_set = MultibodyJointSet::new();
        let mut ccd_solver = CCDSolver::new();
        let mut query_pipeline = QueryPipeline::new();
        let physics_hooks = ();
        let event_handler = ();

        let gravity = vector![0.0, -9.81, 0.0];

        while app_state.is_running {
            delta_time = self.delta_time().as_secs_f32();
            self.canvas.clear();
            
            match app_state.state {
                GameState::Playing => {
                    if app_state.reset {
                        self.load_level("./assets/scenes/test_chamber/data.ron".to_owned());
                        play = play::GameLogic::new(&mut self);
                        app_state.reset = false;
                    } else {
                        play.plane_systems.altitude = ((self.renderizable_instances.get("player").unwrap().instance.transform.position.y - self.renderizable_instances.get("world").unwrap().instance.transform.position.y)).round();

                        play.update( &mut app_state, &mut event_pump, &mut self, &mut controller);
                        self.renderizable_instances.get_mut("fellow_aviator").unwrap().renderizable_transform.position.z += 1000.0 * delta_time;
                        let plane_rot = self.renderizable_instances.get("player").unwrap().instance.transform.rotation;
                        let plane_pos = self.renderizable_instances.get("player").unwrap().renderizable_transform.position ;

                        for (model_key, model) in &self.game_models {
                            let mut offset_index = 0;
                            
                            for (key, renderizable) in &mut self.renderizable_instances {
                                if renderizable.model_ref == *model_key {

                                    // instead of doing it from the plane position, do it from the "rigid_body" one
                                    // renderizable.instance.transform.position = renderizable.renderizable_transform.position - plane_pos;
                                    
                                    let offset = offset_index as u64 * std::mem::size_of::<InstanceRaw>() as u64;
                                    self.queue.write_buffer(
                                        &model.instance_buffer,
                                        offset,
                                        bytemuck::cast_slice(&[renderizable.instance.transform.to_raw()]),
                                    );
                                    offset_index += 1
                                }

                                match &renderizable.physics_data {
                                    Some(physics_info) => {
                                        if let Some(rigid_body) = self.physics.rigidbody_set.get(physics_info.rigidbody_handle) {
                                            renderizable.instance.transform.position = cgmath::Vector3::new(rigid_body.translation().x, rigid_body.translation().y, rigid_body.translation().z);
                                            renderizable.instance.transform.rotation = cgmath::Quaternion::new(rigid_body.rotation().w, rigid_body.rotation().coords.x, rigid_body.rotation().coords.y, rigid_body.rotation().coords.z);
                                        }
                                    },
                                    None => {},
                                }
                            }
                        }

                        self.physics.physics_pipeline.step(
                            &gravity,
                            &integration_parameters,
                            &mut island_manager,
                            &mut broad_phase,
                            &mut narrow_phase,
                            &mut self.physics.rigidbody_set,
                            &mut self.physics.collider_set,
                            &mut impulse_joint_set,
                            &mut multibody_joint_set,
                            &mut ccd_solver,
                            Some(&mut query_pipeline),
                            &physics_hooks,
                            &event_handler,
                        );
                        
                        // lighting update
                        if let Some(sun) = self.renderizable_instances.get("sun") {
                            self.lighting_data.light_uniform.position = (sun.instance.transform.position.x, sun.instance.transform.position.y, sun.instance.transform.position.z).into();
                            match &sun.instance.metadata.lighting {
                                Some(lighting_data) => {
                                    self.lighting_data.light_uniform.color = lighting_data.color.into();
                                },
                                None => {},
                            }
                        }
                        self.queue.write_buffer(&self.lighting_data.light_buffer, 0, bytemuck::cast_slice(&[self.lighting_data.light_uniform]));
                        // lighting update

                        self.camera.uniform.update_view_proj(&self.camera.camera, &self.camera.projection);
                        self.queue.write_buffer(&self.camera.buffer, 0, bytemuck::cast_slice(&[self.camera.uniform]));
                        self.queue.write_buffer(&self.depth_render.near_far_buffer, 0, bytemuck::cast_slice(&[self.depth_render.near_far_uniform]));


                    }
                },
                GameState::MainMenu => {
                    if app_state.reset {
                        main_menu = main_menu::GameLogic::new(&mut self);
                        app_state.reset = false;
                    }
                    main_menu.update(&mut app_state, &mut event_pump, &mut self, &mut controller)
                },
                GameState::SelectingPlane => {
                    if app_state.reset {
                        self.load_level("./assets/scenes/selecting_plane/data.ron".to_owned());
                        selecting_plane = plane_selection::GameLogic::new(&mut self);
                        app_state.reset = false;
                    }
                    selecting_plane.update(&mut app_state, &mut event_pump, &mut self, &mut controller);
                    
                    for (model_key, model) in &self.game_models {
                        let mut offset_index = 0;

                        for (_key, renderizable) in &mut self.renderizable_instances {
                            if renderizable.model_ref == *model_key {
                                let offset = offset_index as u64 * std::mem::size_of::<InstanceRaw>() as u64;

                                self.queue.write_buffer(
                                    &model.instance_buffer,
                                    offset,
                                    bytemuck::cast_slice(&[renderizable.instance.transform.to_raw()]),
                                );
                                offset_index += 1
                            }

                            match &renderizable.physics_data {
                                Some(physics_info) => {
                                    if let Some(rigid_body) = self.physics.rigidbody_set.get(physics_info.rigidbody_handle) {
                                        renderizable.instance.transform.position = cgmath::Vector3::new(rigid_body.translation().x, rigid_body.translation().y, rigid_body.translation().z);
                                        renderizable.instance.transform.rotation = cgmath::Quaternion::new(rigid_body.rotation().w, rigid_body.rotation().coords.x, rigid_body.rotation().coords.y, rigid_body.rotation().coords.z);
                                    }
                                },
                                None => {
                                },
                            }
                            // renderizable.renderizable_transform.position = renderizable.instance.transform.position - plane_pos;
                        }
                    }

                    self.physics.physics_pipeline.step(
                        &gravity,
                        &integration_parameters,
                        &mut island_manager,
                        &mut broad_phase,
                        &mut narrow_phase,
                        &mut self.physics.rigidbody_set,
                        &mut self.physics.collider_set,
                        &mut impulse_joint_set,
                        &mut multibody_joint_set,
                        &mut ccd_solver,
                        Some(&mut query_pipeline),
                        &physics_hooks,
                        &event_handler,
                    );

                    self.camera.uniform.update_view_proj(&self.camera.camera, &self.camera.projection);
                    self.queue.write_buffer(&self.camera.buffer, 0, bytemuck::cast_slice(&[self.camera.uniform]));
                    self.queue.write_buffer(&self.depth_render.near_far_buffer, 0, bytemuck::cast_slice(&[self.depth_render.near_far_uniform]));
                }
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

    fn load_level(&mut self, level_path: String) {
        // i get the json data
        self.renderizable_instances = HashMap::new();

        for (_key, model) in &mut self.game_models {
            model.instance_count = 0;
        }

        let instances_data_to_load = Self::load_instances(level_path);
        match instances_data_to_load {
            Some(instances) => {
                // models to load
                let mut models: Vec<String> = vec![];

                for data in &instances {
                    dbg!("{}", data);
                    if !models.contains(&data.model.to_string()) {
                        models.push(data.model.to_string())
                    }
                }

                // we get all data id and game_objects
                for model_name in &models {
                    let mut ids: Vec<String> = vec![];
                    let mut model_instances:Vec<&GameObject> = vec![];

                    for game_object in &instances {
                        if &game_object.model == model_name {
                            ids.push(game_object.id.clone());
                            model_instances.push(game_object);
                        }
                    }

                    for (i, instance_data) in model_instances.iter().enumerate() {
                        match self.game_models.get_mut(model_name) {
                            Some(model_data) => {
                                model_data.instance_count += 1;
                                model_data.instance_buffer = Self::create_instance_buffer(&model_instances, &self.device);
                            },
                            None => {
                                let model = task::block_in_place( || {
                                    tokio::runtime::Runtime::new()
                                        .unwrap()
                                        .block_on(resources::load_model_gltf(&model_name, &self.device, &self.queue, &self.transform_bind_group_layout))
                                });

                                match model {
                                    Ok(correct_model) => {
                                        self.game_models.insert(
                                            model_name.to_string(), 
                                            ModelDataInstance {
                                                model: correct_model,
                                                instance_count: 1,
                                                instance_buffer: Self::create_instance_buffer(&model_instances, &self.device)
                                            }
                                        );
                                    },
                                    Err(e) => eprintln!("The element was not loaded as an instance: {}", e),
                                }
                            },
                        }
                        
                        // Physics
                        let mut physics_data: Option<PhysicsData> = None;

                        if let Some(physics_obj_data) = &instance_data.metadata.physics {
                            println!("is a physics object");
                            let mut rigid_body = if physics_obj_data.rigidbody.is_static {
                                RigidBodyBuilder::fixed().additional_mass(physics_obj_data.rigidbody.mass).translation(vector![instance_data.transform.position.x, instance_data.transform.position.y, instance_data.transform.position.z]).build()
                            } else {
                                
                                // this was what you had on your cpp code
                                let inertia_tensor = Matrix3::new(
                                    48531.0 as f32, -1320.0, 0.0,
                                    -1320.0, 256608.0, 0.0,
                                    0.0, 0.0, 211333.0,
                                );
                                

                                // i had to do this
                                let principal_inertia = nalgebra::Vector3::new(48531.0, 256608.0, 211333.0);

                                RigidBodyBuilder::dynamic()
                                // .additional_mass(physics_obj_data.rigidbody.mass)
                                .additional_mass_properties(rapier3d::prelude::MassProperties::new(vector![0.0, 0.0, 0.0].into(), physics_obj_data.rigidbody.mass, principal_inertia))
                                .translation(vector![instance_data.transform.position.x, instance_data.transform.position.y, instance_data.transform.position.z])
                                // .rotation(instance_data.transform.rotation)
                                .build()
                                
                                
                            };

                            rigid_body.set_linvel(physics_obj_data.rigidbody.initial_velocity, true);

                            let rigidbody_handle = self.physics.rigidbody_set.insert(rigid_body);

                            // collisions
                            match &physics_obj_data.collider {
                                Some(collider_data) => {
                                    let collider = match collider_data {
                                        game_object::ColliderType::Cuboid { half_extents } => {
                                            ColliderBuilder::cuboid(half_extents.0, half_extents.1, half_extents.2).build()
                                        },
                                        game_object::ColliderType::HalfSpace { normal } => {
                                            ColliderBuilder::halfspace(Unit::new_normalize(*normal)).build()
                                        },
                                        game_object::ColliderType::Ball { radius } => todo!(),
                                        game_object::ColliderType::Cylinder { half_height, radius } => todo!(),
                                        game_object::ColliderType::HeightField { heights, scale_x, scale_y } => todo!(),
                                    };

                                    self.physics.collider_set.insert_with_parent(collider, rigidbody_handle, &mut self.physics.rigidbody_set);
                                },
                                None => todo!(),
                            }

                            physics_data = Some(PhysicsData { rigidbody_handle })
                        };

                        println!("loaded data: {}", ids[i]);
                        self.renderizable_instances.insert(ids[i].clone(), InstanceData { physics_data: physics_data, renderizable_transform: instance_data.transform.clone(), instance: (**instance_data).clone(), model_ref: model_name.clone() });
                    }
                }
            },
            None => eprintln!("The instance data was not correctly loaded"),
        }
    }

    fn delta_time(&mut self) -> Duration {
        let current_time = Instant::now();
        let delta_time = current_time.duration_since(self.last_frame); // this is our Time.deltatime
        self.last_frame = current_time;
        return delta_time
    }

    fn open_first_available_controller(controller_subsystem: &GameControllerSubsystem) -> Option<GameController> {
        for id in 0..controller_subsystem.num_joysticks().unwrap() {
            if controller_subsystem.is_game_controller(id) {
                println!("{}", controller_subsystem.name_for_index(id).unwrap());
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

    pub fn create_instance_buffer(instances: &Vec<&GameObject>, device: &Device) -> Buffer {
        let raw_instances: Vec<InstanceRaw> = instances.iter()
        .map(|instance| instance.transform.to_raw())
        .collect();

        device.create_buffer_init(
            &wgpu::util::BufferInitDescriptor {
                label: Some("Instance Buffer"),
                contents: bytemuck::cast_slice(&raw_instances),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            }
        )
    }

    fn load_instances(path: String) -> Option<Vec<GameObject>> {
        match std::fs::read_to_string(path) {
            Ok(file_contents) => {
                match from_str::<Scene>(&file_contents) {
                    Ok(level) => {
                        println!("--------------------------------------- Level open ---------------------------------------");
                        println!("{}", level.id);
                        println!("{}", level.description);
                        return Some(level.children);
                    },
                    Err(e) => {
                        // Handle the error if deserialization fails
                        eprintln!("Error deserializing RON: {}", e);
                    }
                }
            },
            _ => {}
        }
        return None
    }
}