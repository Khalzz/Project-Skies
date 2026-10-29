use std::collections::HashMap;

use wgpu::{Device, DeviceDescriptor, InstanceDescriptor, Limits, Queue, Surface, SurfaceConfiguration, TextureUsages};
use glyphon::{Cache, Resolution, Viewport};

use crate::rendering::debug_views::{DebugViews, WireframeRenderer};
use crate::rendering::lines::{LineRenderer, Lines};
use crate::rendering::camera::handler::CameraResources;
use crate::rendering::enviroment::environment::{Environment, PreparedEnvironment};
use crate::rendering::enviroment::light::Light;
use crate::rendering::frame::FramePlan;
use crate::rendering::graphics_backend::GraphicsBackend;
use crate::rendering::models::model::Mesh;
use crate::rendering::models::textures::Texture;
use crate::rendering::render_pipeline::blur_renderer::{offscreen_texture, BlurRender};
use crate::rendering::render_pipeline::depth_renderer::DepthRender;
use crate::rendering::render_sets::RenderSets;
use crate::rendering::shader_library::{Placement, ShaderLibrary, BUILTIN_SCREEN_INPUTS};
use crate::window::window::Window;

pub struct Glyphon {
  pub(crate) cache: Cache,
  pub viewport: Viewport,
}

/**
 * # Renderer
 *
 * Everything that draws: the WGPU device and surface, plus every system that
 * lives as long as the app does and puts something on screen - shaders,
 * camera and light buffers, the environment, debug lines. What a scene
 * draws (its nodes, cameras) lives on the scene itself.
 */
pub struct Renderer {
  // 'static: create_surface_unsafe doesn't actually borrow the window (it
  // copies the raw window/display handles out), so there's no real lifetime
  // to track here.
  pub surface: Surface<'static>,
  pub device: Device,
  pub queue: Queue,
  pub config: SurfaceConfiguration,
  pub depth_texture: Texture,
  pub depth_render: DepthRender,
  pub blur: BlurRender,
  pub glyphon: Glyphon,

  /// The active camera's GPU buffer and bind group - filled from whichever
  /// camera the active scene has selected, every frame.
  pub camera_resources: CameraResources,
  /// The sun's GPU buffer (direction, color, shader time, camera position) -
  /// filled every frame from the active scene's sun `Light` node.
  pub light: Light,
  /// The shaders models are drawn with, by name (see
  /// rendering::shader_library) - and where each material in use is drawn
  /// this frame (worked out at the start of the frame plan).
  pub shaders: ShaderLibrary,
  pub material_placements: HashMap<String, Placement>,
  /// What each 2D material in use (the UI's, the line and text sets') is
  /// drawn with this frame - itself, or its flashing magenta stand-in (see
  /// ShaderLibrary::prepare_overlay).
  pub overlay_materials: HashMap<String, String>,
  /// The fingerprint of what the last material setup read, and the
  /// Fullscreen steps it got ready - see App::place_materials.
  pub(crate) placed: Option<(u64, std::collections::HashSet<String>)>,
  /// 2D nodes' shapes, refilled each frame (see App::prepare_nodes_2d).
  pub shapes_2d: crate::rendering::shapes_2d::Shapes2D,
  /// What a frame draws, in order (see rendering::frame) - the active
  /// scene's, set in its start; back to `default_frame_plan` on a scene
  /// change.
  pub frame_plan: FramePlan,
  pub default_frame_plan: FramePlan,
  /// The line and text sets the frame plan's steps draw - the active
  /// scene's, registered in its start, dropped on a scene change (see
  /// rendering::render_sets).
  pub render_sets: RenderSets,
  /// Lines the game wants drawn this frame, in the world, over everything -
  /// emptied once it's drawn (see rendering::lines).
  pub lines: Lines,
  /// The engine's debug views - see rendering::debug_views.
  pub debug: DebugViews,
  pub(crate) line_renderer: LineRenderer,
  pub(crate) wireframe: WireframeRenderer,
  /// What's drawn behind everything - see `set_environment`.
  environment: Environment,
  /// See `set_ambient_light`.
  ambient_light: [f32; 3],
  /// `environment` built and ready to draw - or, for a moment after it
  /// changed, the one before it - and the `Environment` it was built from.
  pub(crate) prepared_environment: PreparedEnvironment,
  pub(crate) prepared_from: Environment,
  /// The graphics API it's drawing with - see `backend`.
  backend: GraphicsBackend,
  /// The game's own images to draw into - see `add_target`.
  pub(crate) targets: HashMap<String, Texture>,
}

impl Renderer {
  /// The graphics API it's drawing with - the one asked for, or what `Auto`
  /// picked (never `Auto` itself).
  pub fn backend(&self) -> GraphicsBackend {
    self.backend
  }

  /// The renderer on `backend` (see GraphicsBackend) - an error, not a
  /// crash, when this machine can't draw with it.
  pub async fn new(window: &Window, backend: GraphicsBackend) -> Result<Renderer, String> {
    // wgpu validates every call in debug builds (release builds never do);
    // the WGPU_VALIDATION / WGPU_DEBUG environment variables ("0" or "1")
    // override that either way. Measure speed in release: debug builds of
    // wgpu itself are slower, DX12's much more than Vulkan's.
    let flags = wgpu::InstanceFlags::default().with_env();
    let instance = wgpu::Instance::new(&InstanceDescriptor { backends: backend.wgpu_backends(), flags, ..Default::default() });
    let surface = unsafe {
        let target = wgpu::SurfaceTargetUnsafe::from_window(window.sdl_window()).map_err(|error| error.to_string())?;
        instance.create_surface_unsafe(target).map_err(|error| format!("couldn't draw to the window with {backend}: {error}"))?
    };

    let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: Some(&surface),
        ..Default::default()
    }).await.map_err(|error| format!("no GPU can draw with {backend} here: {error}"))?;
    let backend = GraphicsBackend::from_wgpu(adapter.get_info().backend);

    let (device, queue) = adapter.request_device(
        &DeviceDescriptor {
            label: None,
            required_features: wgpu::Features::empty(),
            required_limits: Limits::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        },
    ).await.map_err(|error| format!("the GPU couldn't start on {backend}: {error}"))?;

    // Surface settings
    let surface_caps = surface.get_capabilities(&adapter);
    let surface_format = surface_caps.formats;

    let config = wgpu::SurfaceConfiguration {
        // COPY_SRC where the screen allows it: the always-on-top UI (a modal)
        // blurs a copy of the frame drawn so far, HUD included - see
        // BlurRender::blur_backdrop. Without it, it blurs only the world.
        usage: TextureUsages::RENDER_ATTACHMENT | (surface_caps.usages & TextureUsages::COPY_SRC),
        format: surface_format[0],
        // Real backing pixels, not the window's points - on a HiDPI/Retina
        // display those differ (see Window::pixel_size's own doc comment), and
        // the GPU surface needs the real pixel count to actually render at
        // native display resolution instead of being upscaled.
        width: window.pixel_size.width,
        height: window.pixel_size.height,
        // Vsync on by default - see set_vsync.
        present_mode: Self::present_mode(true),
        // Explicitly Opaque rather than surface_caps.alpha_modes[0] (whatever the
        // driver happens to report first) - the window should never be
        // alpha-composited against the desktop behind it. Every frame's alpha
        // channel already ends up 1.0 by the time it reaches the swapchain, but
        // leaving the *declared* mode to chance still lets Windows' compositor
        // treat this surface as alpha-aware for its own purposes - the likely
        // cause of the Alt-Tab/taskbar live thumbnail blinking. Opaque is
        // virtually always supported for a desktop swapchain, but fall back to
        // whatever's first if it somehow isn't.
        alpha_mode: if surface_caps.alpha_modes.contains(&wgpu::CompositeAlphaMode::Opaque) {
            wgpu::CompositeAlphaMode::Opaque
        } else {
            surface_caps.alpha_modes[0]
        },
        view_formats: vec![],
        // 1 meant the CPU couldn't start a new frame until the GPU had
        // actually finished presenting the last one - get_current_texture()
        // was blocking on that every frame (~2.5ms/frame). 3 lets the CPU
        // queue further ahead instead of stalling there, for a few extra ms of
        // input latency - a standard tradeoff.
        desired_maximum_frame_latency: 3,
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

    let camera_resources = CameraResources::new(&device, &config);
    let light = Light::new(&device, &config, &camera_resources);

    // Every model is drawn with a shader from here, by its material's name
    // (see rendering::shader_library) - built when first used, with these
    // bind groups: the mesh's texture, the camera, the mesh's transform, the
    // light.
    let mut shaders = ShaderLibrary::new(
        device.clone(),
        queue.clone(),
        config.format,
        [
            Texture::create_bind_group_layout(&device),
            camera_resources.bind_group_layout.clone(),
            Mesh::create_bind_group_layout(&device),
            light.rendering_data.bind_group_layout.clone(),
        ],
    );
    // What shaders that `reads: [Depth]` get - see ShaderLibrary::set_depth_copy.
    shaders.set_depth_copy(depth_render.foam_depth_copy.view.clone(), depth_render.foam_depth_copy.sampler.clone());
    shaders.set_screen_inputs(Self::screen_inputs(&blur, &HashMap::new()));
    // The toasts' icons (see widgets::toast).
    shaders.add(crate::rendering::shader_library::TILED, include_str!("../shaders/materials/tiled.wgsl"), None);
    crate::ui::widgets::toast::add_icon_shaders(&mut shaders);
    // The dropdowns' chevron (see widgets::dropdown).
    crate::ui::widgets::dropdown::add_icon_shaders(&mut shaders);

    let line_renderer = LineRenderer::new(&device, config.format, &light.rendering_data.bind_group_layout, &camera_resources.bind_group_layout);
    let wireframe = WireframeRenderer::new(&device, config.format, &camera_resources.bind_group_layout);
    let shapes_2d = crate::rendering::shapes_2d::Shapes2D::new(&device, config.format);


    let environment = Environment::default();
    let prepared_environment = crate::resources::prepare_environment(&device, &queue, &camera_resources.bind_group_layout, &config, environment.clone());

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
      camera_resources,
      light,
      shaders,
      material_placements: HashMap::new(),
      overlay_materials: HashMap::new(),
      placed: None,
      shapes_2d,
      frame_plan: FramePlan::engine_default(),
      default_frame_plan: FramePlan::engine_default(),
      render_sets: Default::default(),
      lines: Lines::default(),
      debug: DebugViews::default(),
      line_renderer,
      wireframe,
      ambient_light: [0.25, 0.25, 0.3],
      prepared_from: environment.clone(),
      environment,
      prepared_environment,
      backend,
      targets: HashMap::new(),
    })
  }

  /// Vsync on: frames wait for the screen's refresh (no tearing, no GPU
  /// running flat out on a menu). Off: as many frames as the GPU can make -
  /// for measuring real throughput, or a player who wants it. On by default;
  /// can change while the game runs.
  pub fn set_vsync(&mut self, vsync: bool) {
    let present_mode = Self::present_mode(vsync);
    if self.config.present_mode != present_mode {
      self.config.present_mode = present_mode;
      self.surface.configure(&self.device, &self.config);
    }
  }

  pub fn vsync(&self) -> bool {
    self.config.present_mode == Self::present_mode(true)
  }

  /// The surface's present mode for vsync on or off - wgpu's Auto modes
  /// pick the best one this GPU supports (Fifo/FifoRelaxed on; Mailbox, which
  /// doesn't tear, or Immediate off), so either works on every GPU.
  fn present_mode(vsync: bool) -> wgpu::PresentMode {
    if vsync { wgpu::PresentMode::AutoVsync } else { wgpu::PresentMode::AutoNoVsync }
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
    for (name, target) in self.targets.iter_mut() {
      *target = offscreen_texture(&self.device, width.max(1), height.max(1), self.config.format, name);
    }

    self.glyphon.viewport.update(
      &self.queue,
      Resolution {
          width: self.config.width,
          height: self.config.height,
      },
    );

    self.camera_resources.resize(width, height);
    // The depth copy and blur targets were just rebuilt at the new size - the
    // shaders that read them need the new ones (a bind group is tied to the
    // specific texture view it was created against). Particles read the
    // depth copy too - App::resize hands it to them.
    self.shaders.set_depth_copy(self.depth_render.foam_depth_copy.view.clone(), self.depth_render.foam_depth_copy.sampler.clone());
    self.shaders.set_screen_inputs(Self::screen_inputs(&self.blur, &self.targets));
  }

  /// The images Fullscreen shaders read, by name - the world drawn this
  /// frame ("scene"), the world blurred ("blurred"), and every one of the
  /// game's own targets (see `add_target`).
  fn screen_inputs<'a>(blur: &BlurRender, targets: &'a HashMap<String, Texture>) -> Vec<(&'a str, wgpu::TextureView, wgpu::Sampler)> {
    let mut inputs = vec![
        ("scene", blur.scene_color.view.clone(), blur.scene_color.sampler.clone()),
        ("blurred", blur.blurred_view.clone(), blur.sampler.clone()),
    ];
    inputs.extend(targets.iter().map(|(name, target)| (name.as_str(), target.view.clone(), target.sampler.clone())));
    inputs
  }

  /// An image of the game's own, `name`d, the size of the screen - drawn
  /// into by the frame plan's steps between `.target(name, clear)` and
  /// `.end_target()`, and read by a Fullscreen shader that has `name` in its
  /// `inputs` (a HUD drawn apart, then blended over the world). Kept for the
  /// app's life, through every scene. "scene" and "blurred" are the
  /// engine's own images, and can't be taken.
  pub fn add_target(&mut self, name: &str) {
    if BUILTIN_SCREEN_INPUTS.contains(&name) {
      eprintln!("add_target: \"{name}\" is one of the engine's own images ({}) - pick another name", BUILTIN_SCREEN_INPUTS.join(", "));
      return;
    }
    if self.targets.contains_key(name) {
      return;
    }
    let target = offscreen_texture(&self.device, self.config.width.max(1), self.config.height.max(1), self.config.format, name);
    self.targets.insert(name.to_owned(), target);
    self.shaders.set_screen_inputs(Self::screen_inputs(&self.blur, &self.targets));
  }

  /// Whether the game added a target `name` (see `add_target`).
  pub fn has_target(&self, name: &str) -> bool {
    self.targets.contains_key(name)
  }

  /// What's drawn behind everything.
  pub fn environment(&self) -> &Environment {
    &self.environment
  }

  /// Sets what's drawn behind everything - a color, a skybox, the
  /// procedural sky - for every scene, until it's set again. Set from a
  /// scene's constructor, a skybox's textures load with the scene's assets,
  /// under the loading screen; set while a scene runs, they're read on the
  /// spot.
  pub fn set_environment(&mut self, environment: Environment) {
    self.environment = environment;
  }

  /// The light from the whole sky - see `set_ambient_light`.
  pub fn ambient_light(&self) -> [f32; 3] {
    self.ambient_light
  }

  /// Sets the light from the whole sky (linear RGB) - falling on every
  /// surface whichever way it faces, sun or no sun: what keeps a surface
  /// turned away from the sun, or a scene without one, from going black.
  /// Black for a truly dark scene. For every scene, until it's set again;
  /// a soft blue-grey (0.25, 0.25, 0.3) by default.
  pub fn set_ambient_light(&mut self, color: [f32; 3]) {
    self.ambient_light = color;
  }

  /// Builds `environment` (a skybox's textures, the procedural sky...) - on
  /// the main thread, for a change made while a scene runs.
  pub(crate) fn prepare_environment(&self, environment: Environment) -> PreparedEnvironment {
    crate::resources::prepare_environment(&self.device, &self.queue, &self.camera_resources.bind_group_layout, &self.config, environment)
  }
}
