use std::env;
use std::time::{Duration, Instant};

use cgmath::*;
use rand::Rng;
use sdl2::pixels::Color;
use sdl2::render::TextureCreator;
use sdl2::video::{DisplayMode, WindowContext};
use sdl2::{video::Window, Sdl, render::Canvas};
use wgpu::util::DeviceExt;
use wgpu::{BindGroup, BindGroupLayout, BindGroupLayoutDescriptor, Buffer, DepthBiasState, Device, DeviceDescriptor, Features, InstanceDescriptor, Limits, PipelineLayout, Queue, RenderPassDepthStencilAttachment, RenderPipeline, StencilState, Surface, SurfaceConfiguration, TextureUsages};
use crate::game_object::GameObject;
use crate::gameplay::play;
use crate::input::button_module::Button;
use crate::rendering::camera::{Camera, CameraRenderizable};
use crate::rendering::depth_renderer::DepthRender;
use crate::rendering::model::{self, DrawModel, Model, Vertex};

use crate::rendering::textures::Texture;
use crate::resources;

// instances: these values are just for generating the elements
const NUM_INSTANCES_PER_ROW: u32 = 1;
// instances 

// the bytemuck elements let us have better control on our lists of T elements and without this we can't create our vertex buffer
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ManualVertex {
    pub position: [f32; 3],
    pub tex_coords: [f32; 2],
}

impl ManualVertex {
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<ManualVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute { // position of the vertex
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute { // tex_coords
                    offset: std::mem::size_of::<[f32; 3]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x2,
                }
            ]
        }
    }
}

const DEPTH_VERTICES: &[ManualVertex] = &[
    ManualVertex {
        position: [-1.0, -1.0, 0.0],
        tex_coords: [0.0, 1.0],
    },
    ManualVertex {
        position: [1.0, -1.0, 0.0],
        tex_coords: [1.0, 1.0],
    },
    ManualVertex {
        position: [1.0, 1.0, 0.0],
        tex_coords: [1.0, 0.0],
    },
    ManualVertex {
        position: [-1.0, 1.0, 0.0],
        tex_coords: [0.0, 0.0],
    },
];

const DEPTH_INDICES: &[u16] = &[0, 1, 2, 0, 2, 3];

pub enum GameState {
    Playing,
}

pub enum CameraState {
    Normal,
    Front,
}

pub struct AppState {
    pub is_running: bool,
    pub state: GameState,
}

// Instancing
pub struct Instance {
    pub position: cgmath::Vector3<f32>,
    pub rotation: cgmath::Quaternion<f32>,
    pub scale: cgmath::Vector3<f32>,
}

impl Instance {
    pub fn to_raw(&self) -> InstanceRaw {
        let translation = cgmath::Matrix4::from_translation(self.position.cast::<f32>().unwrap());
        // Create a rotation matrix from the quaternion
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

pub struct App {
    last_frame: Instant,
    pub context: Sdl,
    pub width: u32,
    pub height: u32,
    pub canvas: Canvas<Window>,
    pub current_display: DisplayMode,
    pub texture_creator: TextureCreator<WindowContext>,
    pub surface: Surface,
    pub queue: Queue,
    pub device: Device,
    pub config: SurfaceConfiguration,
    pub render_pipeline: wgpu::RenderPipeline,
    pub index_buffer: wgpu::Buffer,
    pub diffuse_bind_group: wgpu::BindGroup,
    pub camera: CameraRenderizable,
    pub instances: Vec<Instance>,
    instance_buffer: wgpu::Buffer,
    depth_texture: Texture,
    depth_render: DepthRender,
    pub show_depth_map: bool,
    obj_model: Model,
    pub water: Model,
    pub water_instance_buffer: wgpu::Buffer,
    pub mountain: Model,
    pub mountain_instance_buffer: wgpu::Buffer
}

impl App {
    pub async fn new(title: &str, ext_width: Option<u32>, ext_height: Option<u32>) -> App{
        // base sdl2
        let context = sdl2::init().expect("SDL2 wasn't initialized");
        let video_susbsystem = context.video().expect("The Video subsystem wasn't initialized");

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

        let diffuse_bytes = include_bytes!("../assets/textures/sad_hamster.png"); // search the image
        let diffuse_texture = Texture::from_bytes(diffuse_bytes, &device, &queue, "sad-hamster.png").unwrap();

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
        let diffuse_bind_group = device.create_bind_group(
            &wgpu::BindGroupDescriptor {
                label: Some("diffuse_bind_group"),
                layout: &texture_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&diffuse_texture.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&diffuse_texture.sampler),
                    }
                ],
            }
        );
        // Textures

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
        // this will define a list of instances and setting their position/rotation automatically bassed on the constants especified
        let instances = (0..NUM_INSTANCES_PER_ROW).flat_map(|z| {
            (0..NUM_INSTANCES_PER_ROW).map(move |x| {
                let position = cgmath::Vector3 { x: -600.0, y: 10.0, z: 0.0 };
                let rotation = cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_y(), cgmath::Deg(90.0));
                let scale = cgmath::Vector3 { x: 1.0, y: 1.0, z: 1.0 };

                Instance {
                    position, rotation, scale
                }
            })
        }).collect::<Vec<_>>();

        // now that we have our data we will create our isntances buffer to send at the gpu
        let instance_data = instances.iter().map(Instance::to_raw).collect::<Vec<_>>();
        let instance_buffer = device.create_buffer_init(
            &wgpu::util::BufferInitDescriptor {
                label: Some("Instance Buffer"),
                contents: bytemuck::cast_slice(&instance_data),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST
            }
        );

        // instances
        let obj_model = resources::load_model("F14.obj", &device, &queue, &texture_bind_group_layout).await.unwrap();
        let water = resources::load_model("water.obj", &device, &queue, &texture_bind_group_layout).await.unwrap();
        let mountain = resources::load_model("mountains.obj", &device, &queue, &texture_bind_group_layout).await.unwrap();


        let water_instance = Instance {
            position: cgmath::Vector3 { x: 0.0, y: 0.0, z: 0.0 },
            rotation: cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_z(), cgmath::Deg(0.0)),
            scale: cgmath::Vector3 { x: 10000.0, y: 0.0, z: 10000.0 },
        };

        let water_instance_data = water_instance.to_raw();
        let water_instance_buffer = device.create_buffer_init(
            &wgpu::util::BufferInitDescriptor {
                label: Some("Water Instance Buffer"),
                contents: bytemuck::cast_slice(&[water_instance_data]),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            }
        );

        let mountain_instance = Instance {
            position: cgmath::Vector3 { x: 0.0, y: -30.0, z: 0.0 },
            rotation: cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_z(), cgmath::Deg(0.0)),
            scale: cgmath::Vector3 { x: 1000.0, y: 1000.0, z: 1000.0 },
        };

        let mountain_instance_data = mountain_instance.to_raw();
        let mountain_instance_buffer = device.create_buffer_init(
            &wgpu::util::BufferInitDescriptor {
                label: Some("Water Instance Buffer"),
                contents: bytemuck::cast_slice(&[mountain_instance_data]),
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            }
        );

        let depth_render = DepthRender::new(&device, &config);

        App {
            last_frame: Instant::now(),
            current_display,
            context,
            width,
            height,
            canvas,
            texture_creator,
            surface,
            queue,
            device,
            config,
            render_pipeline,
            index_buffer,
            diffuse_bind_group,
            camera,
            instances,
            instance_buffer,
            depth_texture,
            depth_render,
            show_depth_map: false,
            obj_model,
            water,
            water_instance_buffer,
            mountain,
            mountain_instance_buffer,
        }
    }

    pub fn resize(&mut self) {
        self.config.width = self.current_display.w as u32;
        self.config.height = self.current_display.h as u32;

        self.surface.configure(&self.device, &self.config);
        self.depth_render.resize(&self.device, &self.config);
        self.camera.projection.resize(self.width, self.height);

        self.depth_texture = Texture::create_depth_texture(&self.device, &self.config, "depth_texture");
    }

    pub fn render(&self) -> Result<(), wgpu::SurfaceError> {
        // WGPU
        let output = self.surface.get_current_texture()?;
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default()); // this let us to control how render code interacts with textures
        
        // most graphics frameworks expect commands to be stored in a buffer before sending them to the gpu, the encoder is that buffer
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

            render_pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
            render_pass.set_pipeline(&self.render_pipeline);
            render_pass.set_bind_group(0, &self.diffuse_bind_group, &[]);
            render_pass.draw_model_instanced(&self.obj_model, 0..self.instances.len() as u32, &self.camera.bind_group);

            // make a water object who creates a vertex buffer
            render_pass.set_vertex_buffer(1, self.water_instance_buffer.slice(..));
            render_pass.draw_model_instanced(&self.water, 0..1 as u32, &self.camera.bind_group);

            render_pass.set_vertex_buffer(1, self.mountain_instance_buffer.slice(..));
            render_pass.draw_model_instanced(&self.mountain, 0..1 as u32, &self.camera.bind_group);
        }

        if self.show_depth_map {
            self.depth_render.render(&view, &mut encoder);
        }

        // we have the render pass inside the {} so we can do the submit to the queue, we can also drop the render pass if you prefeer
        self.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }

    pub fn update(mut self) {
        // SDL2
        let mut app_state = AppState { is_running: true, state: GameState::Playing};
        let mut event_pump = self.context.event_pump().unwrap();

        // we define a font for our text
        let ttf_context = sdl2::ttf::init().unwrap(); // we create a "context"
        let use_font = "./assets/fonts/Inter-Thin.ttf";
        let mut _font = ttf_context.load_font(use_font, 20).unwrap();

        // here we define the initial state of our game states
        let mut play = play::GameLogic::new(&mut self, 5.0);

        let example_button = Button::new(GameObject{ active: true, x: 0.0, y: 0.0, width: 100.0, height: 20.0 }, 
        Some("Example".to_owned()), 
        Color::RGB(100, 100, 100), 
        Color::RGB(0, 0, 0), 
        Color::RGB(0, 0, 0), 
        Color::RGB(0, 0, 0), 
        None, 
        crate::input::button_module::TextAlign::Center);

        let camera_state = CameraState::Normal;
        let mut target: Point3<f32>;
        let mut camera_position: Point3<f32>;

        // main game loop
        while app_state.is_running { 
            let delta_time = self.delta_time().as_secs_f32();
            self.canvas.clear();
            example_button.render(&mut self.canvas, &self.texture_creator, &_font);

            play.update(&_font, &mut app_state, &mut event_pump, &mut self);

            match self.render() {
                Ok(_) => {},
                Err(wgpu::SurfaceError::Outdated) => { 
                    self.resize()
                }
                Err(e) => eprintln!("Error: {}", e),
            }

            match app_state.state {
                GameState::Playing => {
                    for instance in &mut self.instances {
                        match camera_state {
                            CameraState::Normal => {
                                camera_position = Point3::new(instance.position.x, instance.position.y, instance.position.z) + (instance.rotation * Vector3::new(0.0, 1.0, -3.0));
                                target = Point3::new(instance.position.x, instance.position.y, instance.position.z) + (instance.rotation * Vector3::new(0.0, 0.0, 100.0));
                
                            },
                            CameraState::Front => {
                                camera_position = Point3::new(instance.position.x, instance.position.y, instance.position.z) + (instance.rotation * Vector3::new(0.0, 1.0, 0.0));
                                target = Point3::new(instance.position.x, instance.position.y, instance.position.z) + (instance.rotation * Vector3::new(0.0, 0.0, 10.0));
                            },
                        }

                        self.camera.camera.position = camera_position;
                        // self.camera.camera.position = Self::lerp_point3(self.camera.camera.position, camera_position, delta_time * 20.0);
                        self.camera.camera.look_at((target.x, target.y, target.z).into());
                        self.camera.camera.up = instance.rotation * Vector3::unit_y();
                    }
                    
                    let instance_data = self.instances.iter().map(Instance::to_raw).collect::<Vec<_>>();

                    self.queue.write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&instance_data));
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

    fn lerp(start: f32, end: f32, t: f32) -> f32 {
        start + (end - start) * t
    }

    fn lerp_point3(start: Point3<f32>, end: Point3<f32>, t: f32) -> Point3<f32> {
        Point3::new(
            start.x + (end.x - start.x) * t,
            start.y + (end.y - start.y) * t,
            start.z + (end.z - start.z) * t
        )
    }
}