

use std::env;
use sdl2::{Sdl, video::{DisplayMode, Window}, render::Canvas};

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
  pub tittle: String,
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
    // Resolved window size: whatever WindowSettings.size specified, or the native
    // display resolution when it was None - always populated either way.
    pub size: Size,
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
      let window: Window = video_susbsystem.window(&window_settings.tittle, width, height as u32).metal_view().build().expect("The window wasn't created");

      let mut canvas = window.into_canvas().accelerated().build().expect("the canvas wasn't builded");
      canvas.set_blend_mode(sdl2::render::BlendMode::Blend);

      if window_settings.fullscreen {
        canvas.window_mut().set_fullscreen(sdl2::video::FullscreenType::Desktop).expect("Failed to set fullscreen");
      }

      WindowManager {
        canvas,
        context,
        current_display,
        size: Size { width, height },
      }
    }
}