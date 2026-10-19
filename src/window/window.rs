

use std::env;
use sdl2::{Sdl, video::{DisplayMode, FullscreenType, Window, WindowPos}, render::Canvas};

use crate::app::Size;

/*
  let window_settings = {
      title: "Pankarta Software",
      size: { // if None its given the default its setted as the native for the system
          width: 1280,
          height: 720,
      },
      screen_index: 0, // by default its setted to 0 unless you add a specific value
  }
*/
pub struct WindowSettings {
  pub title: String,
  pub size: Option<Size>,
  pub screen_index: Option<i32>,
  pub fullscreen: bool,
}

/*
  # Window manager

  for the creation of the base window inside the engine we need a Window struct
  this will handle the base creation of a window inside the system, and handle all elements
  connected directly to the SDL2 window.
*/
pub struct WindowManager {
    pub canvas: Canvas<Window>,
    pub context: Sdl,
    pub current_display: DisplayMode,
    // Resolved window size in points/logical units: whatever WindowSettings.size
    // specified, or the native display resolution when it was None - always
    // populated either way. This is the coordinate space every Mouse{Motion,
    // Button} event and every UiTransform/hit-test is in - use this (not
    // pixel_size) for anything UI/logic-facing.
    pub size: Size,
    // The window's real backing-pixel size - on a HiDPI/Retina display this is
    // size scaled up by the display's backing scale factor (2x, 3x, ...), same
    // as SDL's own drawable_size(). Only the GPU surface/render targets should
    // be sized off this - it's not the coordinate space anything else (mouse,
    // UI layout) is in.
    pub pixel_size: Size,
}

impl WindowManager {
    pub fn new( window_settings: WindowSettings ) -> WindowManager {
      let context = sdl2::init().expect("SDL2 wasn't initialized");
      let video_susbsystem = context.video().expect("The Video subsystem wasn't initialized");

      let current_display = video_susbsystem.current_display_mode(window_settings.screen_index.unwrap_or(0)).unwrap();

      let width = match &window_settings.size {
          Some(size) => size.width,
          None => current_display.w as u32,
      };
      let height = match &window_settings.size {
          Some(size) => size.height,
          None => current_display.h as u32,
      };

      // Create window in windowed mode first to avoid device loss
      let window: Window = video_susbsystem.window(&window_settings.title, width, height as u32).vulkan().allow_highdpi().resizable().build().expect("The window wasn't created");

      let mut canvas = window.into_canvas().accelerated().build().expect("the canvas wasn't builded");
      canvas.set_blend_mode(sdl2::render::BlendMode::Blend);

      if window_settings.fullscreen {
        canvas.window_mut().set_fullscreen(sdl2::video::FullscreenType::Desktop).expect("Failed to set fullscreen");
      }

      let (width, height) = canvas.window().size();
      let (pixel_width, pixel_height) = canvas.window().drawable_size();

      WindowManager {
        canvas,
        context,
        current_display,
        size: Size { width, height },
        pixel_size: Size { width: pixel_width, height: pixel_height },
      }
    }

    pub fn refresh_size(&mut self) {
        let (width, height) = self.canvas.window().size();
        let (pixel_width, pixel_height) = self.canvas.window().drawable_size();
        self.size = Size { width, height };
        self.pixel_size = Size { width: pixel_width, height: pixel_height };
    }

    /// Whether the window covers its screen (borderless, at the desktop's
    /// resolution).
    pub fn is_fullscreen(&self) -> bool {
        self.canvas.window().fullscreen_state() != FullscreenType::Off
    }

    /// Borderless fullscreen at the desktop's resolution, or back to a window
    /// of the size it had before. Use App::set_fullscreen instead from a game -
    /// it resizes the renderer and UI straight away too.
    pub fn set_fullscreen(&mut self, fullscreen: bool) -> Result<(), String> {
        let mode = if fullscreen { FullscreenType::Desktop } else { FullscreenType::Off };
        self.canvas.window_mut().set_fullscreen(mode)?;
        self.refresh_display();
        self.refresh_size();
        Ok(())
    }

    /// The window's size in windowed mode, centered on its screen. While
    /// fullscreen the window keeps the desktop's resolution, and this is the
    /// size it comes back to. Use App::set_resolution instead from a game.
    pub fn set_resolution(&mut self, width: u32, height: u32) -> Result<(), String> {
        let window = self.canvas.window_mut();
        window.set_size(width, height).map_err(|e| e.to_string())?;
        window.set_position(WindowPos::Centered, WindowPos::Centered);
        self.refresh_size();
        Ok(())
    }

    /// Every resolution the window's current screen supports, biggest first -
    /// for a settings menu's resolution list.
    pub fn available_resolutions(&self) -> Vec<(u32, u32)> {
        let Ok(video) = self.context.video() else { return Vec::new() };
        let Ok(display) = self.canvas.window().display_index() else { return Vec::new() };
        let count = video.num_display_modes(display).unwrap_or(0);
        let mut resolutions: Vec<(u32, u32)> = (0..count)
            .filter_map(|index| video.display_mode(display, index).ok())
            .map(|mode| (mode.w as u32, mode.h as u32))
            .collect();
        resolutions.sort_by(|a, b| b.cmp(a));
        resolutions.dedup();
        resolutions
    }

    /// Re-reads `current_display` from whichever screen the window is on now.
    fn refresh_display(&mut self) {
        let Ok(video) = self.context.video() else { return };
        let Ok(display) = self.canvas.window().display_index() else { return };
        if let Ok(mode) = video.current_display_mode(display) {
            self.current_display = mode;
        }
    }
}