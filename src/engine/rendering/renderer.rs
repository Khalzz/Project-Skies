use wgpu::{Device, DeviceDescriptor, Features, InstanceDescriptor, Limits, Queue, Surface, SurfaceConfiguration, TextureUsages};
use glyphon::{Cache, Resolution, Viewport};

use crate::engine::rendering::models::textures::Texture;
use crate::engine::rendering::render_pipeline::depth_renderer::DepthRender;
use crate::engine::window::window::WindowManager;

pub struct Glyphon {
  pub(crate) cache: Cache,
  pub viewport: Viewport,
}

/**
 * # Graphics manager
 *
 * Graphics manager its the main iteration of WGPU for initial configuration, rendering and more related to it.
 */

pub struct Renderer {
  // 'static: create_surface_unsafe doesn't actually borrow window_manager (it copies
  // the raw window/display handles out), so there's no real lifetime to track here.
  pub surface: Surface<'static>,
  pub device: Device,
  pub queue: Queue,
  pub config: SurfaceConfiguration,
  pub depth_texture: Texture,
  pub depth_render: DepthRender,
  pub glyphon: Glyphon,
}

impl Renderer {
  pub async fn new(window_manager: &WindowManager) -> Result<Renderer, String> {
    let instance = wgpu::Instance::new(&InstanceDescriptor::default());
    let surface = unsafe {
        match instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::from_window(window_manager.canvas.window()).unwrap()) {
            Ok(s) => s,
            Err(e) => return Err(e.to_string()),
        }
    };

     let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default() // remember that this set every other parameter as their default values
    }).await.unwrap();

    let (device, queue) = adapter.request_device(
        &DeviceDescriptor {
            label: None,
            required_features: Features::empty(),
            required_limits: Limits::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        },
    ).await.unwrap();

    // Surface settings
    let surface_caps = surface.get_capabilities(&adapter);
    let surface_format = surface_caps.formats;

    let config = wgpu::SurfaceConfiguration {
        usage: TextureUsages::RENDER_ATTACHMENT,
        format: surface_format[0],
        width: window_manager.size.width,
        height: window_manager.size.height,
        present_mode: wgpu::PresentMode::AutoNoVsync,
        alpha_mode: surface_caps.alpha_modes[0],
        view_formats: vec![],
        desired_maximum_frame_latency: 1,
    };

    surface.configure(&device, &config);

    // G L Y P H O N
    let cache = Cache::new(&device);
    let mut viewport = Viewport::new(&device, &cache);

    viewport.update(
      &queue,
      Resolution {
          width: config.width,
          height: config.height,
      },
    );
    // G L Y P H O N

    let depth_texture = Texture::create_depth_texture(&device, &config, "depth_texture");
    let depth_render = DepthRender::new(&device, &config);

    Ok(Renderer {
      surface,
      device,
      queue,
      config,
      depth_texture,
      depth_render,
      glyphon: Glyphon {
        cache,
        viewport,
      },
    })
  }

  // Everything here is sized off the surface, so it all gets touched together -
  // called from App::resize with whatever the new window dimensions are.
  pub fn resize(&mut self, width: u32, height: u32) {
    self.config.width = width;
    self.config.height = height;

    self.surface.configure(&self.device, &self.config);
    self.depth_render.resize(&self.device, &self.config);
    self.depth_texture = Texture::create_depth_texture(&self.device, &self.config, "depth_texture");

    self.glyphon.viewport.update(
      &self.queue,
      Resolution {
          width: self.config.width,
          height: self.config.height,
      },
    );
  }
}
