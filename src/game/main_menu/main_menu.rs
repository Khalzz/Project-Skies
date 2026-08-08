use glyphon::Color;
use nalgebra::{Point3, Vector3};

use crate::app::App;
use crate::engine::scene_manager::scene::{FrameContext, Scene};
use crate::engine::rendering::ui::ui::Ui;
use crate::engine::ui::ui_node::{Style, UiNode};
use crate::engine::ui::ui_transform::{Orientation, PositionValue, SizeValue};
use crate::engine::ui::layer::Layer;
use crate::game::play::event_handling::EventSystem;
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
        .on_hover(Style { text_color: Some(Color::rgba(255, 255, 255, 255)), border_color: Some([1.0, 1.0, 1.0, 1.0]), ..Default::default() })
        .set_transition(50.0)
        .on_click(on_click)
        .set_border_width(1.0)
}

// Switches which of the two top-level panels (see GameLogic::new) is
// visible/interactable - both were added to app.ui under these exact ids, and
// is_active is what UiNode::node_content_preparation checks to skip rendering and
// hit-testing a whole subtree, see UiNode::set_active.
fn show_panel(app: &mut App, id: &str) {
    for panel_id in ["Main Menu", "Settings"] {
        if let Some(panel) = Ui::get_ui_node(&mut app.ui.renderizable_elements, panel_id) {
            panel.set_active(panel_id == id);
        }
    }
}

// Retitles the content panel's header to whichever of Video/Controller was
// clicked - reuses Label::set_text (same call free_camera's HUD already makes)
// rather than rebuilding the node, via the same Ui::get_ui_node path lookup
// show_panel uses.
fn set_settings_header(app: &mut App, text: &str) {
    if let Some(header) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "Settings/content/header") {
        if let Some(label) = header.as_label_mut() {
            label.set_text(&mut app.ui.text.font_system, text, false);
        }
    }
}

fn ui(app: &mut App) -> UiNode {
    UiNode::container()
        .set_size(SizeValue::Fit, SizeValue::Fit)
        .set_position(PositionValue::Start(40.0), PositionValue::End(-40.0))
        .set_corner_radius(10.0)
        .set_padding(10.0)
        .set_gap(5.0)
        .set_background_color([0.0, 0.0, 0.0, 0.8])
        .set_text_color(Color::rgba(200, 200, 200, 255))
        .set_child("Play", button(app, "Play", |app| app.scene_manager.open_scene("playing")))
        .set_child("Settings", button(app, "Settings", |app| show_panel(app, "Settings")))
        .set_child("Quit", button(app, "Quit", |_app| std::process::exit(0)))
}

fn settings_ui(app: &mut App) -> UiNode {
    UiNode::container()
        .set_size(SizeValue::Fit, SizeValue::Fit)
        .set_position(PositionValue::Center(0.0), PositionValue::Center(0.0))
        .set_orientation(Orientation::Horizontal)
        .set_gap(20.0)
        .set_child("options",
            UiNode::container()
                .set_size(SizeValue::Pixels(200.0), SizeValue::Pixels(400.0))
                .set_corner_radius(10.0)
                .set_padding(10.0)
                .set_gap(5.0)
                .set_background_color([0.0, 0.0, 0.0, 0.8])
                .set_text_color(Color::rgba(200, 200, 200, 255))
                .set_child("Video", button(app, "Video", |app| set_settings_header(app, "Video")))
                .set_child("Controller", button(app, "Controller", |app| set_settings_header(app, "Controller")))
                .set_child("Back", button(app, "Back", |app| show_panel(app, "Main Menu")))
        )
        // Right side is deliberately empty for now - just the container itself,
        // ready for whichever settings page (Video/Controller/...) gets picked on
        // the left to eventually fill in.
        .set_child("content",
            UiNode::container()
                .set_size(SizeValue::Pixels(400.0), SizeValue::Pixels(400.0))
                .set_corner_radius(10.0)
                .set_padding(10.0)
                .set_background_color([0.0, 0.0, 0.0, 0.8])
                // Blank until Video/Controller is clicked - set_settings_header
                // fills it in via Label::set_text, no rebuild needed. Explicit
                // width/height (not auto-measured from the initial empty string) -
                // set_text only ever changes the buffer's text, never re-measures
                // the box afterward, so an auto-measured empty label would stay
                // zero-width forever regardless of what text gets set into it later.
                // set_text_color is explicit here (not inherited - style no longer
                // cascades from a container to its children, see Style's doc comment).
                .set_child("header", UiNode::label(&mut app.ui.text.font_system, "", Some(380.0), Some(30.0))
                    .set_font_size(24.0)
                    .set_text_color(Color::rgba(200, 200, 200, 255)))
        )
        .active(false)
}

impl GameLogic {
    // this is called once
    pub fn new(app: &mut App) -> Self {
        load_level(app, "./assets/scenes/main_menu".to_owned());

        app.window_manager.context.mouse().set_relative_mouse_mode(false);

        Layer::new(app)
            .set_child("Main Menu", ui(app))
            .set_child("Settings", settings_ui(app))
            .build(app);

        let event_system = match EventSystem::new(&app.scene_openned) {
            Ok(system) => Some(system),
            Err(error) => {
                eprintln!("Error: {}", error);
                None
            },
        };

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


