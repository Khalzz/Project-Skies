use std::collections::HashMap;
use std::env;
use std::time::{Duration, Instant};


use cgmath::{Matrix4, Quaternion, Rotation3, Vector3, Zero};
use glyphon::{Color, Font, FontSystem, Resolution, SwashCache, TextArea, TextAtlas, TextRenderer};
use rand::Rng;
use sdl2::controller::GameController;
use sdl2::haptic::Haptic;
use sdl2::render::TextureCreator;
use sdl2::video::{DisplayMode, WindowContext};
use sdl2::{GameControllerSubsystem, HapticSubsystem};
use sdl2::{video::Window, Sdl, render::Canvas};
use wgpu::util::DeviceExt;
use wgpu::{BindGroup, BindGroupLayout, BindGroupLayoutDescriptor, Buffer, DepthBiasState, Device, DeviceDescriptor, Features, InstanceDescriptor, Limits, PipelineLayout, Queue, RenderPassDepthStencilAttachment, RenderPipeline, StencilState, Surface, SurfaceConfiguration, TextureUsages};
use crate::game_object::GameObject;
use crate::gameplay::play;
use crate::primitive::rectangle::{self, RectPos};
use crate::rendering::camera::{Camera, CameraRenderizable};
use crate::rendering::depth_renderer::DepthRender;
use crate::rendering::model::{self, DrawModel, Model, Vertex};

use crate::rendering::textures::Texture;
use crate::rendering::vertex::VertexUi;
use crate::resources;
use crate::transform::Transform;
use crate::ui::button::Button;

pub enum GameState {
    Playing,
}



pub struct Text {
    pub text_renderer: TextRenderer,
    pub text_cache: SwashCache,
    pub font_system: FontSystem,
    pub text_atlas: TextAtlas

}

pub struct AppState {
    pub is_running: bool,
    pub state: GameState,
}

pub struct InstanceData {
    pub instance: Instance,
    pub instance_raw: InstanceRaw,
    pub instance_buffer: Buffer,
    pub model: Model
}

pub struct Instance {
    pub position: cgmath::Vector3<f32>,
    pub rotation: cgmath::Quaternion<f32>,
    pub scale: cgmath::Vector3<f32>,
}

impl Instance {
    pub fn to_raw(&self) -> InstanceRaw {
        let translation = cgmath::Matrix4::from_translation(self.position.cast::<f32>().unwrap());
        let rotation = cgmath::Matrix4::from(self.rotation.cast::<f32>().unwrap());
        let scale = cgmath::Matrix4::from_nonuniform_scale(self.scale.x as f32, self.scale.y as f32, self.scale.z as f32);
        let model: Matrix4<f32> = translation * Matrix4::from(rotation) * scale;

        InstanceRaw {
            model: model.into(),
        }
    }
}

// quaternions are not very usable in wgpu so instead of doing math in the shader we are gonna save the raw instance here directly
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct InstanceRaw {
    model: [[f32; 4]; 4],
}

impl InstanceRaw {
    // we create a vertexBuffer
    fn desc() -> wgpu::VertexBufferLayout<'static> {
        use std::mem;
        wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<InstanceRaw>() as wgpu::BufferAddress, // the vertexbuffer is of type instance raw it means that our shader will only change to use the next instance
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                // A mat 4 is basically a vec4 of vec4's so we have to define every vec4 of it
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 5,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: mem::size_of::<[f32; 4]>() as wgpu::BufferAddress,
                    shader_location: 6,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: mem::size_of::<[f32; 8]>() as wgpu::BufferAddress,
                    shader_location: 7,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: mem::size_of::<[f32; 12]>() as wgpu::BufferAddress,
                    shader_location: 8,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        }
    }
}
// Instancing

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

pub struct UiRendering {
    vertex_buffer: Buffer,
    index_buffer: Buffer
}

pub struct App {
    last_frame: Instant,
    pub context: Sdl,
    pub size: Size,
    pub canvas: Canvas<Window>,
    pub current_display: DisplayMode,
    pub texture_creator: TextureCreator<WindowContext>,
    pub surface: Surface,
    pub queue: Queue,
    pub device: Device,
    pub config: SurfaceConfiguration,
    pub render_pipeline: wgpu::RenderPipeline,
    pub ui_pipeline: wgpu::RenderPipeline,
    pub index_buffer: wgpu::Buffer,
    pub camera: CameraRenderizable,
    depth_texture: Texture,
    depth_render: DepthRender,
    pub show_depth_map: bool,
    pub controller_subsystem: GameControllerSubsystem,
    pub haptic_subsystem: HapticSubsystem,
    pub text: Text,
    pub components: Vec<Button>,
    pub dynamic_ui_components: HashMap<String, Vec<Button>>,
    pub mouse_pos: MousePos,
    pub renderizable_instances: HashMap<String, InstanceData>,
    pub throttling: Throttling,
    pub ui_rendering: UiRendering
}

impl App {
    pub async fn new(title: &str, ext_width: Option<u32>, ext_height: Option<u32>) -> App{
        // base sdl2
        let context = sdl2::init().expect("SDL2 wasn't initialized");
        let video_susbsystem = context.video().expect("The Video subsystem wasn't initialized");
        
        let controller_subsystem = context.game_controller().unwrap();
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

        env::set_var("SDL_VIDEO_MINIMIZE_ON_FOCUS_LOSS", "0"); // this is highly needed so the sdl2 can alt tab without generating bugs

        let window: Window = video_susbsystem.window(title, width, height as u32).vulkan().fullscreen().build().expect("The window wasn't created");
        
        // WGPU INSTANCES AND SURFACE
        let instance = wgpu::Instance::new(InstanceDescriptor::default());
        let surface = unsafe { instance.create_surface(&window).unwrap() }; // the surface is where we draw stuff created based on a raw window handle

        // The adapter will let us get information and data from our graphics card (for example the name of it)
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


        let transform = Transform::new(Vector3::zero(), Quaternion::zero(), Vector3::new(1.0, 1.0, 1.0));
        let transform_matrix = transform.to_matrix_bufferable();

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

        // we have to create a bind group for each texture since the fact that the layout and the group are separated is because we can swap the bind group on runtime
        // Textures

        let mut font_system = FontSystem::new();
        let font = include_bytes!("../assets/fonts/Inter-Thin.ttf");
        font_system.db_mut().load_font_data(font.to_vec());

        let text_cache = SwashCache::new();
        let mut text_atlas = TextAtlas::new(&device, &queue, config.format);
        let text_renderer = TextRenderer::new(
            &mut text_atlas,
            &device,
            wgpu::MultisampleState::default(),
            None,
        );

        let text_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/text_shader.wgsl").into()),
        });

        let ui_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("ui render pipeline layout"),
                bind_group_layouts: &[],
                push_constant_ranges: &[],
            });

        let ui_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui render pipeline"),
            layout: Some(&ui_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &text_shader,
                entry_point: "vertex",
                buffers: &[VertexUi::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &text_shader,
                entry_point: "fragment",
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Bgra8UnormSrgb,
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
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            multisample: wgpu::MultisampleState::default(),
            depth_stencil: None,
            multiview: None,
        });

        // Camera
        // we set up the camera
        let camera = CameraRenderizable::new(&device, &config);

        // SHADERING PROCESS 
        // we get access to our shader file
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

        // here we define elements that will be sent to the gpu
        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[model::ModelVertex::desc(), InstanceRaw::desc()], // we set the values of the instance for the render pipeline
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader, 
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
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
                depth_compare: wgpu::CompareFunction::Less, // this sets what pixels to draw in wich order, the less says that pixels will be drawn front to back.
                stencil: StencilState::default(), 
                bias: DepthBiasState::default() 
            }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        });

        let index_buffer = device.create_buffer_init(
            &wgpu::util::BufferInitDescriptor {
                label: Some("Index Buffer"),
                contents: bytemuck::cast_slice(&[0,1,2]),
                usage: wgpu::BufferUsages::INDEX,
            }
        );

        let mut canvas = window.into_canvas().accelerated().build().expect("the canvas wasn't builded");

        canvas.set_blend_mode(sdl2::render::BlendMode::Blend);
        let texture_creator = canvas.texture_creator();

        // instances
        let f14 = resources::load_model_gltf("F14.gltf", &device, &queue, &transform_bind_group_layout).await.unwrap();
        let water = resources::load_model_gltf("water.gltf", &device, &queue, &transform_bind_group_layout).await.unwrap();
        let tower = resources::load_model_gltf("tower.gltf", &device, &queue, &transform_bind_group_layout).await.unwrap();
        let tower2 = resources::load_model_gltf("tower2.gltf", &device, &queue, &transform_bind_group_layout).await.unwrap();
        let crane = resources::load_model_gltf("crane.gltf", &device, &queue, &transform_bind_group_layout).await.unwrap();

        let f14_instance = Instance {
            position: cgmath::Vector3 { x: 0.0, y: 150.0, z: 0.0 },
            rotation: cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_z(), cgmath::Deg(0.0)),
            scale: cgmath::Vector3 { x: 19.0, y: 19.0, z: 19.0 },
        };
        let f14_data = Self::create_instance(f14_instance, &device, f14);

        let water_instance = Instance {
            position: cgmath::Vector3 { x: 0.0, y: 0.0, z: 0.0 },
            rotation: cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_z(), cgmath::Deg(0.0)),
            scale: cgmath::Vector3 { x: 100000.0, y: 0.0, z: 100000.0 },
        };
        let water_data = Self::create_instance(water_instance, &device, water);

        let tower_instance = Instance {
            position: cgmath::Vector3 { x: 0.0, y: 400.0, z: 0.0 },
            rotation: cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_z(), cgmath::Deg(0.0)),
            scale: cgmath::Vector3 { x: 150.0, y: 150.0, z: 150.0 },
        };
        let tower_data = Self::create_instance(tower_instance, &device, tower);

        let tower2_instance = Instance {
            position: cgmath::Vector3 { x: 1300.0, y: 300.0, z: 500.0 },
            rotation: cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_z(), cgmath::Deg(0.0)),
            scale: cgmath::Vector3 { x: 150.0, y: 150.0, z: 150.0 },
        };
        let tower2_data = Self::create_instance(tower2_instance, &device, tower2);

        let crane_instance = Instance {
            position: cgmath::Vector3 { x: 1300.0, y: 500.0, z: 500.0 },
            rotation: cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_z(), cgmath::Deg(0.0)),
            scale: cgmath::Vector3 { x: 100.0, y: 100.0, z: 100.0 },
        };
        let crane_data = Self::create_instance(crane_instance, &device, crane);

        let mut renderizable_instances = HashMap::new();
        renderizable_instances.insert("f14".to_owned(), f14_data);
        renderizable_instances.insert("water".to_owned(), water_data);
        renderizable_instances.insert("tower".to_owned(), tower_data);
        renderizable_instances.insert("tower2".to_owned(), tower2_data);
        renderizable_instances.insert("crane".to_owned(), crane_data);
        // instances

        let depth_render = DepthRender::new(&device, &config);

        let components = vec![];
        
        let mut dynamic_ui_components = HashMap::new();
        dynamic_ui_components.insert("bandits".to_owned(), vec![]);
        
        // Dynamic static is for objects that move in the screen but their main position is based on something that never changes
        dynamic_ui_components.insert("dynamic_static".to_owned(), vec![]); 

        let ui_rendering = UiRendering {
            vertex_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 5000,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            index_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 5000,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };

        App {
            last_frame: Instant::now(),
            current_display,
            context,
            size: Size {width, height},
            canvas,
            texture_creator,
            surface,
            queue,
            device,
            config,
            render_pipeline,
            ui_pipeline,
            index_buffer,
            camera,
            depth_texture,
            depth_render,
            show_depth_map: false,
            controller_subsystem,
            text: Text {
                text_renderer,
                text_cache,
                font_system,
                text_atlas
            },
            components,
            mouse_pos: MousePos { x: 0.0, y: 0.0 },
            renderizable_instances,
            dynamic_ui_components,
            throttling: Throttling { last_ui_update: Instant::now(), ui_update_interval: Duration::from_secs_f32(1.0/60.0), last_controller_update: Instant::now(), controller_update_interval: Duration::from_secs_f32(1.0/400.0) },
            ui_rendering,
            haptic_subsystem
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

        for button in self.components.iter_mut() {
            let button_active = button.is_hovered(&self.mouse_pos);
            let button_vertices = button.rectangle.vertices(button_active, &self.size);

            vertices.extend_from_slice(&button_vertices);
            indices.extend_from_slice(&button.rectangle.indices(num_vertices));

            num_vertices += button_vertices.len() as u16;
            num_indices += rectangle::NUM_INDICES;

            text_areas.push(button.text.text_area(button_active));
        }

        for (key, list) in self.dynamic_ui_components.iter_mut() {
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

        self.queue.write_buffer(&self.ui_rendering.vertex_buffer, 0, bytemuck::cast_slice(vertices.as_slice()));
        self.queue.write_buffer(&self.ui_rendering.index_buffer, 0, bytemuck::cast_slice(&indices));
        self.text.text_renderer.prepare(&self.device, &self.queue, &mut self.text.font_system, &mut self.text.text_atlas, Resolution {width: self.size.width,height: self.size.height},text_areas,&mut self.text.text_cache,).unwrap();
        // UI

        // WGPU
        let output = self.surface.get_current_texture()?;
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default()); // this let us to control how render code interacts with textures

        let mut encoder = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        });

        {
            // we make a render pass, this will have all the methods for drawing in the screen
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor { 
                label: Some("Render Pass"), 
                color_attachments: &[Some(wgpu::RenderPassColorAttachment { // here we will define the base colors of the screen
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
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
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            render_pass.set_pipeline(&self.render_pipeline);

            for (key, renderizable) in &self.renderizable_instances {
                render_pass.set_vertex_buffer(1, renderizable.instance_buffer.slice(..));
                render_pass.draw_model_instanced(&renderizable.model, 0..1 as u32, &self.camera.bind_group);
            }
        }

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
    
            render_pass.set_pipeline(&self.ui_pipeline);
            render_pass.set_vertex_buffer(0, self.ui_rendering.vertex_buffer.slice(..));
            render_pass.set_index_buffer(self.ui_rendering.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            render_pass.draw_indexed(0..num_indices, 0, 0..1);
    
            self.text.text_renderer.render(&self.text.text_atlas, &mut render_pass).unwrap();
        }
        
        if self.show_depth_map {
            self.depth_render.render(&view, &mut encoder);
        }

        // we have the render pass inside the {} so we can do the submit to the queue, we can also drop the render pass if you prefeer
        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();
        self.text.text_atlas.trim();

        Ok(())
    }

    pub fn update(mut self) {
        // SDL2
        let mut app_state = AppState { is_running: true, state: GameState::Playing};
        let mut event_pump = self.context.event_pump().unwrap();

        let mut play = play::GameLogic::new(&mut self, 5.0);
        let mut controller = Self::open_first_available_controller(&self.controller_subsystem);

        while app_state.is_running { 
            let delta_time = self.delta_time().as_secs_f32();
            self.canvas.clear();

            play.update( &mut app_state, &mut event_pump, &mut self, &mut controller);
            
            // let mut start_time = Instant::now(); // benchmarking            
            match self.render() {
                Ok(_) => {},
                Err(wgpu::SurfaceError::Outdated) => { 
                    self.resize()
                }
                Err(e) => eprintln!("Error: {}", e),
            }
            // println!("--- Total: {}", start_time.elapsed().as_micros());

            // start_time = Instant::now(); // benchmarking            
            match app_state.state {
                GameState::Playing => {
                    let plane_rot = self.renderizable_instances.get("f14").unwrap().instance.rotation;
                    self.camera.camera.up = self.renderizable_instances.get("f14").unwrap().instance.rotation * Vector3::unit_y();
                    // play.camera_data.look_at = Some(self.renderizable_instances.get("tower").unwrap().instance.position);
                    play.altitude.altitude = ((self.renderizable_instances.get("f14").unwrap().instance.position.y - self.renderizable_instances.get("water").unwrap().instance.position.y)).round();
                    
                    for (key, renderizable) in &mut self.renderizable_instances {
                        self.queue.write_buffer(&renderizable.instance_buffer, 0, bytemuck::cast_slice(&[renderizable.instance.to_raw()]));

                        if key != "f14" {
                            renderizable.instance.position -= plane_rot * play.velocity * delta_time;
                        }
                    }

                    self.camera.uniform.update_view_proj(&self.camera.camera, &self.camera.projection);
                    self.queue.write_buffer(&self.camera.buffer, 0, bytemuck::cast_slice(&[self.camera.uniform]));
                }
            }
        }
    }

    fn delta_time(&mut self) -> Duration {
        let current_time = Instant::now();
        let delta_time = current_time.duration_since(self.last_frame); // this is our Time.deltatime
        self.last_frame = current_time;
        return delta_time
    }

    // connect the first controller found
    fn open_first_available_controller(controller_subsystem: &GameControllerSubsystem) -> Option<GameController> {
        for id in 0..controller_subsystem.num_joysticks().unwrap() {
            if controller_subsystem.is_game_controller(id) {
                return Some(controller_subsystem.open(id).unwrap());
            }
        }
        None
    }

    fn create_instance(instance: Instance, device: &Device, model: Model) -> InstanceData {
        let instance_raw = instance.to_raw();
        let instance_buffer = device.create_buffer_init(
            &wgpu::util::BufferInitDescriptor {
                label: Some("Instance Buffer"),
                contents: bytemuck::cast_slice(&[instance_raw]),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            }
        );

        InstanceData {
            instance,
            instance_raw,
            instance_buffer,
            model
        }
    }
}