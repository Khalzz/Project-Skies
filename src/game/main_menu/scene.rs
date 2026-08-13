use nalgebra::{Point3, Vector3};

use crate::app::App;
use crate::engine::scene_manager::scene::{FrameContext, Scene};
use crate::resources::{apply_environment, load_level};
use crate::engine::rendering::enviroment::environment::{Environment, SkyboxFaces};
use crate::game::tooling::free_camera;

use super::rebind_modal;
use super::ui;

const MENU_CAMERA_NAME: &str = "main_menu";

pub struct GameLogic {
    base_position: Point3<f32>,
    base_yaw: f32,
    base_pitch: f32,
    elapsed: f32,
}

impl GameLogic {
    // this is called once
    pub fn new(app: &mut App) -> Self {
        load_level(app, "./assets/scenes/main_menu".to_owned());

        app.window_manager.context.mouse().set_relative_mouse_mode(false);

        ui::build(app);

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

        Self { base_position, base_yaw, base_pitch, elapsed: 0.0 }
    }

    // this is called every frame
    pub fn update(&mut self, app: &mut App, delta_time: f32) {
        self.drift_camera(app, delta_time);
        free_camera::update(app);
        rebind_modal::update(app);

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


