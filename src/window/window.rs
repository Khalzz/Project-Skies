use sdl2::rect::Rect;
use sdl2::video::{DisplayMode, FullscreenType, Window as SdlWindow, WindowPos};
use sdl2::VideoSubsystem;

use crate::app::Size;

/// How the window sits on its screen - see `Window::set_window_mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowMode {
    /// A normal window with a title bar, at `Window::size`.
    Windowed,
    /// No border, covering the whole screen at the screen's own resolution.
    /// Switching in and out is instant and safe - the usual choice.
    BorderlessFullscreen,
    /// Takes the screen over and changes its resolution to the window's size
    /// (the closest one the screen supports). Switching is slower and can
    /// make the screen go black for a moment.
    Fullscreen,
}

/// The mouse cursor's shape - see `Window::set_cursor_icon`, and
/// `UiNode::set_cursor` for showing one while the mouse is over a node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CursorIcon {
    /// The plain arrow.
    #[default]
    Default,
    /// A pointing hand - something clickable.
    Pointer,
    /// A text beam - something to type in.
    Text,
    /// Something that can be dragged around.
    Move,
    /// Can't be used right now.
    NotAllowed,
    /// Busy.
    Wait,
    ResizeHorizontal,
    ResizeVertical,
    /// Top-right to bottom-left.
    ResizeNeSw,
    /// Top-left to bottom-right.
    ResizeNwSe,
}

/*
  # Window

  The engine's one window. `App::new` creates it hidden - 800x600, windowed,
  resizable, titled "Pankarta Software" - and `App::run` shows it, so it can
  be set up first without the user seeing it jump around:

      let mut app = App::new().await?;
      app.window.set_title("My Game");
      app.window.set_size(1280, 720);
      app.window.set_window_mode(WindowMode::Windowed);
      app.run();

  Every setter also works while the game runs - the renderer and UI follow
  the window's new size at the start of the next frame (see App::run).
*/
pub struct Window {
    window: SdlWindow,
    video: VideoSubsystem,
    mode: WindowMode,
    /// The size asked for with `set_size` - what the window has in
    /// `Windowed`, the screen resolution in `Fullscreen`. Kept while in
    /// `BorderlessFullscreen`, so going back to a window restores it.
    requested_size: (u32, u32),
    /// Whether the game chose `requested_size` (`set_size`) - if not,
    /// `Fullscreen` uses the screen's own resolution, not the window's
    /// default 800x600.
    size_chosen: bool,
    // Resolved window size in points/logical units. This is the coordinate
    // space every Mouse{Motion, Button} event and every UiTransform/hit-test
    // is in - use this (not pixel_size) for anything UI/logic-facing.
    pub size: Size,
    // The window's real backing-pixel size - on a HiDPI/Retina display this is
    // size scaled up by the display's backing scale factor (2x, 3x, ...), same
    // as SDL's own drawable_size(). Only the GPU surface/render targets should
    // be sized off this - it's not the coordinate space anything else (mouse,
    // UI layout) is in.
    pub pixel_size: Size,
    /// The cursor shown now, and the SDL cursor showing it - kept alive
    /// while it's set. None: SDL's own, the arrow.
    cursor: Option<(CursorIcon, sdl2::mouse::Cursor)>,
}

impl Window {
    pub const DEFAULT_TITLE: &'static str = "Pankarta Software";
    pub const DEFAULT_SIZE: (u32, u32) = (800, 600);

    pub(crate) fn new(video: VideoSubsystem) -> Result<Window, String> {
        let (width, height) = Self::DEFAULT_SIZE;
        let window = video.window(Self::DEFAULT_TITLE, width, height)
            .vulkan()
            .allow_highdpi()
            .resizable()
            .hidden()
            .position_centered()
            .build()
            .map_err(|error| format!("the window couldn't be created: {error}"))?;

        let mut window = Window {
            window,
            video,
            mode: WindowMode::Windowed,
            requested_size: (width, height),
            size_chosen: false,
            size: Size { width, height },
            pixel_size: Size { width, height },
            cursor: None,
        };
        window.refresh_size();
        Ok(window)
    }

    /// The SDL window itself - for the renderer's surface.
    pub(crate) fn sdl_window(&self) -> &SdlWindow {
        &self.window
    }

    /// Shows the window - `App::run` does this once everything's set up.
    pub(crate) fn show(&mut self) {
        self.window.show();
    }

    /// Re-reads `size` and `pixel_size` from the window - see App::resize.
    pub(crate) fn refresh_size(&mut self) {
        let (width, height) = self.window.size();
        let (pixel_width, pixel_height) = self.window.drawable_size();
        self.size = Size { width, height };
        self.pixel_size = Size { width: pixel_width, height: pixel_height };
    }

    pub fn title(&self) -> &str {
        self.window.title()
    }

    pub fn set_title(&mut self, title: &str) {
        if let Err(error) = self.window.set_title(title) {
            eprintln!("Couldn't set the window title: {error}");
        }
    }

    /// The window's size (its current one is the `size` field):
    /// - `Windowed`: resized now, and centered on its screen.
    /// - `Fullscreen`: the screen changes to the closest resolution it supports
    ///   (without a `set_size`, it stays at its own).
    /// - `BorderlessFullscreen`: remembered - the window keeps the screen's
    ///   size, and gets this one when it goes back to `Windowed`.
    pub fn set_size(&mut self, width: u32, height: u32) -> Result<(), String> {
        self.requested_size = (width, height);
        self.size_chosen = true;
        match self.mode {
            WindowMode::Windowed => self.apply_windowed_size()?,
            WindowMode::Fullscreen => {
                // Changing the mode of a window that's already fullscreen
                // changes the screen's resolution but leaves the window at its
                // old size - the game then draws into the old size while laying
                // its UI out for the new one. Out of fullscreen and back in at
                // the new mode, the same way set_window_mode goes in.
                self.window.set_fullscreen(FullscreenType::Off)?;
                self.apply_display_mode()?;
                self.window.set_fullscreen(FullscreenType::True)?;
            }
            WindowMode::BorderlessFullscreen => {}
        }
        self.refresh_size();
        Ok(())
    }

    pub fn window_mode(&self) -> WindowMode {
        self.mode
    }

    pub fn set_window_mode(&mut self, mode: WindowMode) -> Result<(), String> {
        match mode {
            WindowMode::Windowed => {
                self.window.set_fullscreen(FullscreenType::Off)?;
                self.apply_windowed_size()?;
            }
            WindowMode::BorderlessFullscreen => {
                self.window.set_fullscreen(FullscreenType::Desktop)?;
            }
            WindowMode::Fullscreen => {
                // The resolution has to be chosen before going fullscreen.
                self.apply_display_mode()?;
                self.window.set_fullscreen(FullscreenType::True)?;
            }
        }
        self.mode = mode;
        self.refresh_size();
        Ok(())
    }

    /// Whether the user can resize the window by dragging its edges (in
    /// `Windowed`). On by default.
    pub fn set_resizable(&mut self, resizable: bool) {
        self.window.set_resizable(resizable);
    }

    /// How many screens are connected.
    pub fn display_count(&self) -> i32 {
        self.video.num_video_displays().unwrap_or(1)
    }

    /// Which screen the window is on (0 is the main one).
    pub fn display(&self) -> i32 {
        self.window.display_index().unwrap_or(0)
    }

    /// Moves the window to screen `index` (0 is the main one), centered on it
    /// - in a fullscreen mode it covers that screen instead.
    pub fn set_display(&mut self, index: i32) -> Result<(), String> {
        let count = self.display_count();
        if index < 0 || index >= count {
            return Err(format!("there's no screen {index} - {count} connected"));
        }
        // A fullscreen window can't be moved - out of it, over, and back.
        let mode = self.mode;
        if mode != WindowMode::Windowed {
            self.window.set_fullscreen(FullscreenType::Off)?;
        }
        let bounds = self.video.display_bounds(index)?;
        self.center_on(bounds);
        if mode != WindowMode::Windowed {
            self.set_window_mode(mode)?;
        }
        self.refresh_size();
        Ok(())
    }

    /// Every resolution the window's current screen supports, biggest first -
    /// for a settings menu's resolution list.
    pub fn available_resolutions(&self) -> Vec<(u32, u32)> {
        let display = self.display();
        let count = self.video.num_display_modes(display).unwrap_or(0);
        let mut resolutions: Vec<(u32, u32)> = (0..count)
            .filter_map(|index| self.video.display_mode(display, index).ok())
            .map(|mode| (mode.w as u32, mode.h as u32))
            .collect();
        resolutions.sort_by(|a, b| b.cmp(a));
        resolutions.dedup();
        resolutions
    }

    /// Locks the mouse to the window and hides it - it then reports only
    /// movement (input::mouse_rel_x/y), for a camera looked around with the
    /// mouse. Off by default.
    pub fn set_cursor_locked(&mut self, locked: bool) {
        self.video.sdl().mouse().set_relative_mouse_mode(locked);
    }

    pub fn is_cursor_locked(&self) -> bool {
        self.video.sdl().mouse().relative_mouse_mode()
    }

    /// Shows the mouse cursor as `icon` - only re-created when it changes.
    /// The engine sets it every frame from the UI under the mouse (see
    /// `UiNode::set_cursor`), so a game sets it on its nodes instead.
    pub fn set_cursor_icon(&mut self, icon: CursorIcon) {
        use sdl2::mouse::SystemCursor;
        if self.cursor.as_ref().is_some_and(|(current, _)| *current == icon) {
            return;
        }
        let system = match icon {
            CursorIcon::Default => SystemCursor::Arrow,
            CursorIcon::Pointer => SystemCursor::Hand,
            CursorIcon::Text => SystemCursor::IBeam,
            CursorIcon::Move => SystemCursor::SizeAll,
            CursorIcon::NotAllowed => SystemCursor::No,
            CursorIcon::Wait => SystemCursor::Wait,
            CursorIcon::ResizeHorizontal => SystemCursor::SizeWE,
            CursorIcon::ResizeVertical => SystemCursor::SizeNS,
            CursorIcon::ResizeNeSw => SystemCursor::SizeNESW,
            CursorIcon::ResizeNwSe => SystemCursor::SizeNWSE,
        };
        if let Ok(cursor) = sdl2::mouse::Cursor::from_system(system) {
            cursor.set();
            self.cursor = Some((icon, cursor));
        }
    }

    pub fn cursor_icon(&self) -> CursorIcon {
        self.cursor.as_ref().map_or(CursorIcon::Default, |(icon, _)| *icon)
    }

    /// The window at `requested_size`, centered on the screen it's on.
    fn apply_windowed_size(&mut self) -> Result<(), String> {
        let (width, height) = self.requested_size;
        self.window.set_size(width, height).map_err(|error| error.to_string())?;
        let bounds = self.video.display_bounds(self.display())?;
        self.center_on(bounds);
        Ok(())
    }

    fn center_on(&mut self, bounds: Rect) {
        let (width, height) = self.window.size();
        let x = bounds.x() + (bounds.width() as i32 - width as i32) / 2;
        let y = bounds.y() + (bounds.height() as i32 - height as i32) / 2;
        self.window.set_position(WindowPos::Positioned(x), WindowPos::Positioned(y));
    }

    /// The screen mode `Fullscreen` uses - the one closest to
    /// `requested_size`, or the screen's own (its desktop resolution) when
    /// the game never chose a size.
    fn apply_display_mode(&mut self) -> Result<(), String> {
        let display = self.display();
        let desktop = self.video.desktop_display_mode(display)?;
        if !self.size_chosen {
            return self.set_fullscreen_mode(desktop);
        }
        let (width, height) = self.requested_size;
        let wanted = DisplayMode::new(desktop.format, width as i32, height as i32, desktop.refresh_rate);
        let mode = self.video.closest_display_mode(display, &wanted)?;
        self.set_fullscreen_mode(mode)
    }

    /// SDL_SetWindowDisplayMode, called directly: sdl2's own
    /// `Window::set_display_mode` (0.37) hands SDL a pointer to a copy of the
    /// mode that's already gone by the time SDL reads it - SDL gets garbage,
    /// finds no screen mode like it, and `Fullscreen` silently stays a window.
    fn set_fullscreen_mode(&mut self, mode: DisplayMode) -> Result<(), String> {
        let mode = mode.to_ll();
        // SAFETY: the window is alive (we own it), and `mode` outlives the
        // call - SDL copies it.
        let result = unsafe { sdl2::sys::SDL_SetWindowDisplayMode(self.window.raw(), &mode) };
        if result < 0 { Err(sdl2::get_error()) } else { Ok(()) }
    }
}
