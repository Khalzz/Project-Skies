use wgpu::{Device, DeviceDescriptor, Features, InstanceDescriptor, Limits, Queue, Surface, SurfaceConfiguration, TextureUsages};
use glyphon::{Cache, Resolution, Viewport};

use crate::engine::rendering::models::textures::Texture;
use crate::engine::rendering::render_pipeline::blur_renderer::BlurRender;
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
  pub blur: BlurRender,
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
        // Vsync'd (locked to the display's refresh rate) instead of presenting
        // uncapped - avoids the GPU spinning at max speed while idling on a menu
        // that has nothing worth rendering thousands of FPS for.
        present_mode: wgpu::PresentMode::Fifo,
        // Explicitly Opaque rather than surface_caps.alpha_modes[0] (whatever the
        // driver happens to report first) - this window is a fullscreen game, it
        // should never be alpha-composited against the desktop behind it. Every
        // frame's alpha channel already ends up 1.0 by the time it reaches the
        // swapchain (DEFAULT_CLEAR_COLOR.a is 1.0 and the UI pass's own blend
        // formula preserves full opacity once the destination already has it),
        // but leaving the *declared* mode to chance still lets Windows' compositor
        // treat this surface as alpha-aware for its own purposes - the likely
        // cause of the Alt-Tab/taskbar live thumbnail blinking (DWM's main
        // composited view apparently overrides this for a topmost, screen-
        // covering window, but its separate thumbnail-generation path doesn't get
        // the same override). Opaque is virtually always supported for a desktop
        // swapchain, but fall back to whatever's first if it somehow isn't.
        alpha_mode: if surface_caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
            wgpu::CompositeAlphaMode::Opaque
        } else {
            surface_caps.alpha_modes[0]
        },
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
    let blur = BlurRender::new(&device, &config);

    Ok(Renderer {
      surface,
      device,
      queue,
      config,
      depth_texture,
      depth_render,
      blur,
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
    self.blur.resize(&self.device, &self.config);

    self.glyphon.viewport.update(
      &self.queue,
      Resolution {
          width: self.config.width,
          height: self.config.height,
      },
    );
  }
}
