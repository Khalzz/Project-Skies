//! The smallest engine project: opens a window with one empty scene.
//!
//! F11 toggles fullscreen, F10 steps through the screen's resolutions.
//!
//! cargo run --example window

use pankarta_software::app::{App, Size};
use pankarta_software::input::input::{self, TypedInput};
use pankarta_software::scene_manager::scene::{FrameContext, Scene, SceneBehaviour};
use pankarta_software::window::window::WindowSettings;
use sdl2::keyboard::Keycode;

/// A scene that only listens for the window keys.
struct EmptyScene {
    /// Which of the screen's resolutions F10 picked last.
    resolution_index: usize,
}

impl SceneBehaviour for EmptyScene {
    fn update(&mut self, _scene: &mut Scene, app: &mut App, _ctx: &mut FrameContext) {
        for typed in input::typed_input() {
            let TypedInput::Key { key, pressed: true, repeat: false, .. } = typed else { continue };

            if key == Keycode::F11 {
                let fullscreen = !app.window_manager.is_fullscreen();
                if let Err(err) = app.set_fullscreen(fullscreen) {
                    eprintln!("Couldn't change fullscreen: {err}");
                }
            } else if key == Keycode::F10 {
                let resolutions = app.window_manager.available_resolutions();
                if resolutions.is_empty() {
                    continue;
                }
                self.resolution_index = (self.resolution_index + 1) % resolutions.len();
                let (width, height) = resolutions[self.resolution_index];
                println!("Resolution: {width}x{height}");
                if let Err(err) = app.set_resolution(width, height) {
                    eprintln!("Couldn't change resolution: {err}");
                }
            }
        }
    }
}

#[tokio::main]
async fn main() {
    /*
        The user should set the window mode NOT IF ITS FULLSCREEN OR NOT
        window can be either fullscreen, windowed or borderless windowed
        find a better way of setting this, remember the window should allow us to:

        1. Set the name of the window
        2. Set the resolution of the window
        3. Check what really the "screen_index" does
        4. Set the window mode to either fullscreen, windowed or borderless windowed

        Also i should be able of ideally doing something like app.window.set_fullscreen(true) or app.window.set_fullscreen(false)
        since the fullscreen change, or all the changes really related to the window itself should be done from the app's "window"
     */

    /*
        I think a good idea of implementation could be that the App::new() directly its able of generating a base window
        the base window by default will have a screen of size 800x600, it will be resizable and windowed with the text "Pankarta Software"
        
        To access to the edition of theese elements i think the best idea its to access them through the next method

        1. Make the app:
            let mut app = App::new().await.unwrap();
        2. Set up the window:
            app.window.set_title("Pankarta Software");
            app.window.set_size(1280, 720);
            app.window.set_window_mode(WindowMode::Windowed);
        3. Run the app:
            app.run();

        This way all the systems made should be loaded by default on the app::new and then edited from the game dev
     */


    let mut app = match App::new(WindowSettings {
        title: "Test Window".to_string(),
        size: Some(Size { width: 800, height: 600 }),
        screen_index: None,
        fullscreen: false,
    }).await {
        Ok(app) => app,
        Err(err) => {
            eprintln!("Couldn't create the app: {err}");
            return;
        }
    };

    app.scene_manager.create_scene("main", |_scene, _app| EmptyScene { resolution_index: 0 });
    app.scene_manager.open_scene("main");

    app.run();
}
