use std::fs::File;
use std::hash::Hash;
use std::time::{Duration, Instant};
use std::collections::HashMap;
use std::env;

use csv::{ReaderBuilder, StringRecord};
use sdl2::joystick::Joystick;
use sdl2::{joystick, JoystickSubsystem};
use sdl2::{GameControllerSubsystem, HapticSubsystem, video::Window, Sdl, render::Canvas};
use sdl2::controller::GameController;
use sdl2::render::TextureCreator;
use sdl2::video::{DisplayMode, WindowContext};
use tokio::runtime::Runtime;
use tokio::task;
use wgpu::{BindGroupLayout, BindGroupLayoutDescriptor, Buffer, Device, DeviceDescriptor, Features, InstanceDescriptor, Limits, Queue, RenderPassDepthStencilAttachment, Surface, SurfaceConfiguration, TextureUsages};
use wgpu::util::DeviceExt;
use cgmath::{Deg, Euler, Matrix4, Quaternion, Rotation3, Vector3};
use glyphon::{Resolution, TextArea};


use crate::gameplay::play::GameLogic;
use crate::rendering::camera::CameraRenderizable;
use crate::rendering::depth_renderer::DepthRender;
use crate::rendering::instance_management::{Instance, InstanceData, InstanceRaw, LevelDataCsv, ModelDataInstance};
use crate::rendering::model::{self, DrawModel, Model, Vertex};
use crate::rendering::textures::Texture;
use crate::rendering::ui::UI;
use crate::rendering::vertex::VertexUi;

use crate::resources;
use crate::ui::button::Button;
use crate::gameplay::{main_menu, play};
use crate::primitive::rectangle;

pub enum GameState {
    Playing,
    MainMenu
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
    pub game_models: HashMap<String, ModelDataInstance>
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

        // SHADERING PROCESS 
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/depth.wgsl").into()),
        });

        let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Render Pipeline Layout"),
            bind_group_layouts: &[
                &texture_bind_group_layout,
                &camera.bind_group_layout,
                &transform_bind_group_layout
            ],
            push_constant_ranges: &[],
        });
        
        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[model::ModelVertex::desc(), InstanceRaw::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader, 
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState { 
                topology: wgpu::PrimitiveTopology::TriangleList, 
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState { 
                format: Texture::DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        });

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
            throttling: Throttling { last_ui_update: Instant::now(), ui_update_interval: Duration::from_secs_f32(1.0/60.0), last_controller_update: Instant::now(), controller_update_interval: Duration::from_secs_f32(1.0/400.0) },
            _haptic_subsystem: haptic_subsystem,
            transform_bind_group_layout,
            game_models
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
            
            let _ = &self.renderizable_instances.iter().for_each(|(_key ,renderizable)| {
                match self.game_models.get(&renderizable.model_ref) {
                    Some(model_data) => {
                        render_pass.set_vertex_buffer(1, model_data.instance_buffer.slice(..));
                        render_pass.draw_model_instanced(&model_data.model, 0..model_data.instance_count as u32, &self.camera.bind_group); // usamos la funcion que renderiza los objetos opacos
                    },
                    None => todo!(),
                }
            });
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
                        render_pass.draw_transparent_model_instanced(&model_data.model , 0..model_data.instance_count as u32, &self.camera.bind_group); // usamos la funcion que renderiza los objetos opacos
                    },
                    None => todo!(),
                }
            });
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

        let mut controller = Self::open_first_available_controller(&self.controller_subsystem);
        let _joystick = Self::open_first_avalible_joystick(&self.joystick_subsystem);
        let mut delta_time: f32;

        while app_state.is_running { 
            delta_time = self.delta_time().as_secs_f32();
            self.canvas.clear();
            
            match app_state.state {
                GameState::Playing => {
                    if app_state.reset {
                        self.load_level("./assets/levels/test_chamber/data.csv".to_owned());
                        play = play::GameLogic::new(&mut self);
                        app_state.reset = false;
                    } else {
                        play.plane_systems.altitude = ((self.renderizable_instances.get("player").unwrap().instance.position.y - self.renderizable_instances.get("world").unwrap().instance.position.y)).round();
                        if play.plane_systems.altitude < 0.0 {
                            app_state.reset = true
                        }

                        play.update( &mut app_state, &mut event_pump, &mut self, &mut controller);
                        self.renderizable_instances.get_mut("fellow_aviator").unwrap().transform.position.z += 1000.0 * delta_time;
                        let plane_rot = self.renderizable_instances.get("player").unwrap().instance.rotation;
                        let plane_pos = self.renderizable_instances.get("player").unwrap().transform.position;

                        for (model_key, model) in &self.game_models {
                            let mut offset_index = 0;

                            for (key, renderizable) in &mut self.renderizable_instances {
                                if renderizable.model_ref == *model_key {
                                    let offset = offset_index as u64 * std::mem::size_of::<InstanceRaw>() as u64;

                                    self.queue.write_buffer(
                                        &model.instance_buffer,
                                        offset,
                                        bytemuck::cast_slice(&[renderizable.instance.to_raw()]),
                                    );
                                    offset_index += 1
                                }
                                renderizable.instance.position = renderizable.transform.position - plane_pos;
                            }
                        }

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
        self.renderizable_instances = HashMap::new();

        for (_key, model) in &mut self.game_models {
            model.instance_count = 0;
        }

        let instances_data_to_load = Self::load_instances(level_path);
        match instances_data_to_load {
            Some(instances) => {
                let mut models: Vec<String> = vec![];

                for data in &instances {
                    if !models.contains(&data.model.to_string()) {
                        models.push(data.model.to_string())
                    }
                }

                for model_name in &models {
                    let mut ids: Vec<String> = vec![];
                    let mut model_instances:Vec<Instance> = vec![];

                    for instance in &instances {
                        if &instance.model == model_name {
                            ids.push(instance.id.clone());
                            model_instances.push(instance.instance.clone());
                        }
                    }

                    for (i, instance_data) in model_instances.iter().enumerate() {
                        match self.game_models.get_mut(model_name) {
                            Some(model_data) => {
                                model_data.instance_count += 1;
                                model_data.instance_buffer = Self::create_instance_buffer(&model_instances, &self.device);
                            },
                            None => {
                                let model = task::block_in_place(|| {
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

                        self.renderizable_instances.insert(ids[i].clone(), InstanceData { transform: instance_data.clone(), instance: instance_data.clone(), model_ref: model_name.clone() });
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

    fn create_instance_buffer(instances: &[Instance], device: &Device) -> Buffer {
        let raw_instances: Vec<InstanceRaw> = instances.iter()
        .map(|instance| instance.to_raw())
        .collect();

        device.create_buffer_init(
            &wgpu::util::BufferInitDescriptor {
                label: Some("Instance Buffer"),
                contents: bytemuck::cast_slice(&raw_instances),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            }
        )
    }

    fn load_instances(path: String) -> Option<Vec<LevelDataCsv>> {
        let file = File::open(path);

        match file {
            Ok(data_file) => {
                let mut reader = ReaderBuilder::new().has_headers(true).from_reader(data_file);
                let mut game_objects_to_instance: Vec<LevelDataCsv> = vec![];

                // create a list of all the models to load
                for result in reader.records() {
                    match result {
                        Ok(record) => {

                            let rotation_values = Vector3::new(
                                record[5].parse::<f32>().unwrap_or(0.0), 
                                record[6].parse::<f32>().unwrap_or(0.0), 
                                record[7].parse::<f32>().unwrap_or(0.0)
                            );

                            let euler_radians: Euler<Deg<f32>> = Euler::new(
                                Deg(rotation_values.x),
                                Deg(rotation_values.y),
                                Deg(rotation_values.z),
                            );

                            game_objects_to_instance.push(LevelDataCsv {
                                id: record[0].to_string(),
                                model: record[1].to_string(),
                                instance: Instance {
                                    position: Vector3::new(
                                        record[2].parse::<f32>().unwrap_or(0.0), 
                                        record[3].parse::<f32>().unwrap_or(0.0), 
                                        record[4].parse::<f32>().unwrap_or(0.0)
                                    ), 
                                    rotation: euler_radians.into(), 
                                    scale: Vector3::new(
                                        record[8].parse::<f32>().unwrap_or(0.0), 
                                        record[9].parse::<f32>().unwrap_or(0.0), 
                                        record[10].parse::<f32>().unwrap_or(0.0)
                                )},
                            })
                        },
                        Err(e) => eprintln!("Error reading record: {}", e)
                    }
                }
                return Some(game_objects_to_instance)
            },
            Err(e) => {
                eprintln!("The file was not found: {}", e);
            }
        }
        return None
    }
}