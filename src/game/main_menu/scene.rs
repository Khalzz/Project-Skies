use glyphon::Color;
use nalgebra::{Point3, Vector3};

use crate::app::App;
use crate::engine::scene_manager::scene::{FrameContext, Scene};
use crate::engine::rendering::ui::ui::Ui;
use crate::engine::ui::ui_node::{Style, UiNode};
use crate::engine::ui::ui_transform::{Orientation, PositionValue, SizeValue};
use crate::engine::ui::layer::Layer;
use crate::game::play::event_handling::EventSystem;
use crate::game::ui::card;
use crate::resources::{apply_environment, load_level};
use crate::engine::rendering::enviroment::environment::{Environment, SkyboxFaces};
use crate::game::tooling::free_camera;

const MENU_CAMERA_NAME: &str = "main_menu";

pub struct GameLogic {
    base_position: Point3<f32>,
    base_yaw: f32,
    base_pitch: f32,
    elapsed: f32,
}

fn button (app: &mut App, label: &str, on_click: impl Fn(&mut App) + 'static) -> UiNode {
    UiNode::label(&mut app.ui.text.font_system, label, Some(150.0), None)
        .set_text_color(Color::rgba(200, 200, 200, 255))
        .set_corner_radius(10.0)
        .set_padding((10.0, 2.0))
        .set_font_size(20.0)
        .on_hover(Style { background_color: Some([0.0, 0.0, 0.0, 0.5]), text_color: Some(Color::rgba(255, 255, 255, 255)), ..Default::default() })
        .set_transition(50.0)
        .on_click(on_click)
        .set_border_width(1.0)
}

fn show_panel(app: &mut App, id: &str) {
    for panel_id in ["Main Menu", "Settings"] {
        if let Some(panel) = Ui::get_ui_node(&mut app.ui.renderizable_elements, panel_id) {
            panel.set_active(panel_id == id);
        }
    }
}

const SETTINGS_TABS: [&str; 2] = ["Video", "Controller"];

fn show_settings_tab(app: &mut App, id: &str) {
    for tab_id in SETTINGS_TABS {
        if let Some(tab) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &format!("Settings/content/{tab_id}")) {
            tab.set_active(tab_id == id);
        }
        // Persistent "this is the open tab" indicator on the sidebar button itself -
        // direct style mutation (same pattern as e.g. play.rs's blinking altitude
        // alert), not on_hover/on_press, since it needs to stay showing regardless
        // of whether the mouse is currently over/down on the button at all.
        if let Some(button) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &format!("Settings/options/top_options/{tab_id}")) {
            button.style.border_color = if tab_id == id { Some([1.0, 1.0, 1.0, 1.0]) } else { None };
        }
    }
}

fn settings_ui(app: &mut App) -> UiNode {
    card()
        .set_background_color([0.0, 0.0, 0.0, 0.0])
        .set_size(SizeValue::Grow, SizeValue::Grow)
        .set_position(PositionValue::Center(0.0), PositionValue::Center(0.0))
        .set_orientation(Orientation::Horizontal)
        .set_gap(10.0)
        .set_child("options",
            card()
                .set_size(SizeValue::Percent(10.0), SizeValue::Grow)
                .set_child("top_options",
                    card()
                        .set_padding(0.0)
                        .set_background_color([0.0, 0.0, 0.0, 0.0])
                        .set_size(SizeValue::Fit, SizeValue::Grow)
                        .set_child("Controller", button(app, "Controller", |app| show_settings_tab(app, "Controller")).set_border_color([1.0, 1.0, 1.0, 1.0]).set_text_color(Color::rgba(255, 255, 255, 255)))
                        .set_child("Video", button(app, "Video", |app| show_settings_tab(app, "Video")))
                )
                .set_child("Back", button(app, "Back", |app| show_panel(app, "Main Menu")))
        )
        .set_child("content",
            card()
                .set_size(SizeValue::Grow, SizeValue::Grow)
                .set_child("Controller", UiNode::label(&mut app.ui.text.font_system, "Controller Settings", Some(380.0), Some(30.0))
                    .set_font_size(24.0)
                    .set_text_color(Color::rgba(200, 200, 200, 255))
                    .active(true))
                .set_child("Video", UiNode::label(&mut app.ui.text.font_system, "Video Settings", Some(380.0), Some(30.0))
                    .set_font_size(24.0)
                    .set_text_color(Color::rgba(200, 200, 200, 255))
                    .active(false))
                
        )
        .active(false)
}

impl GameLogic {
    // this is called once
    pub fn new(app: &mut App) -> Self {
        load_level(app, "./assets/scenes/main_menu".to_owned());

        app.window_manager.context.mouse().set_relative_mouse_mode(false);

        let main_menu = card()
        .set_position(PositionValue::Start(40.0), PositionValue::End(-40.0))
            .set_size(SizeValue::Fit, SizeValue::Fit)
            .set_child("Play", button(app, "Play", |app| app.scene_manager.open_scene("playing")))
            .set_child("Settings", button(app, "Settings", |app| show_panel(app, "Settings")))
            .set_child("Quit", button(app, "Quit", |_app| std::process::exit(0)));

        Layer::new(app)
            .set_child("Main Menu", main_menu)
            .set_child("Settings", settings_ui(app))
            .build(app);

        apply_environment(app, Environment::Skybox(SkyboxFaces {
            px: "skybox/px.png".to_owned(),
            nx: "skybox/nx.png".to_owned(),
            py: "skybox/py.png".to_owned(),
            ny: "skybox/ny.png".to_owned(),
            pz: "skybox/pz.png".to_owned(),
            nz: "skybox/nz.png".to_owned(),
        }));

        let base_position: Point3<f32> = [2.45, 24.58, -6.39].into();
        let base_yaw = 93.0;
        let base_pitch = -10.5;
        app.camera.create_camera(MENU_CAMERA_NAME, base_position, base_yaw, base_pitch, 70.0);
        app.camera.select_camera(MENU_CAMERA_NAME);

        // app.ui.load_ui("./assets/ui/game_ui.ron", app.renderer.config.width, app.renderer.config.height, &app.renderer.device, &app.renderer.queue);
        Self { base_position, base_yaw, base_pitch, elapsed: 0.0 }
    }

    // this is called every frame
    pub fn update(&mut self, app: &mut App, delta_time: f32) {
        self.drift_camera(app, delta_time);
        free_camera::update(app);

        // UI only rebuilds its buffers (and re-runs hover hit-testing) when marked
        // dirty - the panel is otherwise static, so nothing else would ever ask for
        // that again after the first frame. Same requirement free_camera's HUD has.
        app.ui.has_changed = true;
    }

    fn drift_camera(&mut self, app: &mut App, delta_time: f32) {
        self.elapsed += delta_time;
        let t = self.elapsed;

        let offset = Vector3::new(
            (t * 0.6).sin() * 0.4,
            (t * 0.9).sin() * 0.25,
            (t * 0.45).cos() * 0.35,
        );
        let yaw_drift = (t * 0.35).sin() * 1.5;
        let pitch_drift = (t * 0.5).sin() * 0.8;

        if let Some(camera) = app.camera.get_mut(MENU_CAMERA_NAME) {
            camera.camera.position = self.base_position + offset;
            camera.camera.yaw = (self.base_yaw + yaw_drift).to_radians();
            camera.camera.pitch = (self.base_pitch + pitch_drift).to_radians();
        }
    }
}

impl Scene for GameLogic {
    fn update(&mut self, app: &mut App, ctx: &mut FrameContext) {
        let _ = ctx;
        self.update(app, app.time.delta_time);
    }
}


