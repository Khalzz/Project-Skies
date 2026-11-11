use nalgebra::{UnitQuaternion, Vector3};

use pankarta_software::prelude::*;
use pankarta_software::rendering::camera::camera::yaw_pitch_rotation;

use crate::camera::camera::looking_along;
use crate::prefabs::ocean::{self, Ocean, WakeSource};
use crate::scenes::follow_test::scene::sun_node;

use super::rebind_modal;
use super::ui;

/// The menu's camera node.
const CAMERA: &str = "main_menu";

// Resting pose the camera drifts around (see GameLogic::drift_camera).
const BASE_POSITION: Vector3<f32> = Vector3::new(2.20, 0.90, -3.60);
const BASE_YAW_DEG: f32 = 98.2;
const BASE_PITCH_DEG: f32 = -10.5;
const PLANE_POSITION: Vector3<f32> = Vector3::new(-5.0, -3.30, 25.0);
/// Roll, pitch, yaw in degrees (`from_euler_angles`: X, Y, Z).
const PLANE_ROTATION_DEG: Vector3<f32> = Vector3::new(-2.0, 180.0, -19.5);
const WATER_YAW_DEG: f32 = 90.0;
/// How fast the sea streams, metres per second, until the F10 panel changes it.
const FLIGHT_SPEED: f32 = 200.0;

/// The title shot, edited from the F10 panel. The camera drifts around
/// `camera` while the panel is closed; with it open, the camera and the
/// jet sit exactly where these say, and the sea travels along `water_yaw_deg`
/// (0 = +X, 90 = +Z, same as the camera's yaw).
#[derive(Clone, Copy)]
pub struct MenuShot {
    pub open: bool,
    pub camera: Vector3<f32>,
    pub camera_yaw_deg: f32,
    pub camera_pitch_deg: f32,
    pub plane: Vector3<f32>,
    /// Roll, pitch, yaw, degrees.
    pub plane_rotation_deg: Vector3<f32>,
    pub water_yaw_deg: f32,
    /// Metres per second the sea travels along `water_yaw_deg`.
    pub speed: f32,
}

impl Default for MenuShot {
    fn default() -> Self {
        Self {
            open: false,
            camera: BASE_POSITION,
            camera_yaw_deg: BASE_YAW_DEG,
            camera_pitch_deg: BASE_PITCH_DEG,
            plane: PLANE_POSITION,
            plane_rotation_deg: PLANE_ROTATION_DEG,
            water_yaw_deg: WATER_YAW_DEG,
            speed: FLIGHT_SPEED,
        }
    }
}

pub struct GameLogic {
    // Seconds of drift_camera runtime so far - drives the sin/cos wobble
    // below, not real elapsed scene time (same idea as its own delta_time
    // accumulation).
    elapsed: f32,
    /// How far the sea has streamed, metres.
    flown: f32,
    /// The sea - streaming past, with the jet's wake on it.
    ocean: Ocean,
    /// The title shot, edited from the F10 panel.
    menu_shot: MenuShot,
}

impl GameLogic {
    pub fn new(scene: &mut Scene, app: &mut App, environment: Environment) -> Self {
        app.renderer.set_environment(environment);
        app.window.set_cursor_locked(false);

        ui::build(scene, app);
        Self::spawn_world(scene, app);

        Self { elapsed: 0.0, flown: 0.0, ocean: Ocean::default(), menu_shot: MenuShot::default() }
    }

    fn spawn_world(scene: &mut Scene, app: &mut App) {
        let nodes = [
            sun_node([0.7, 0.7, 0.8]),
            Node::new("plane")
                .with_property(Transform3D {
                    position: PLANE_POSITION,
                    rotation: plane_rotation(PLANE_ROTATION_DEG),
                    scale: Vector3::new(1.0, 1.0, 1.0),
                })
                // Drawn bigger than the file (the model shrank to real size,
                // x0.4777) so the shot stays framed. Its wheels are hidden
                // in `update`.
                .with_property(title_f16()),
            Node::new("world")
                .with_property(Transform3D { position: Vector3::new(0.0, -10.0, 0.0), ..Default::default() })
                // The sea - kept centred under the camera by Ocean::update.
                .with_property(ocean::surface()),
            Node::new(CAMERA)
                .with_property(camera_transform(BASE_POSITION, BASE_YAW_DEG, BASE_PITCH_DEG))
                .with_property(CameraLens::new(70.0)),
        ];
        for node in nodes {
            if let Err(error) = scene.spawn_node(app, node) {
                eprintln!("{error}");
            }
        }
    }

    pub fn update(&mut self, scene: &mut Scene, app: &mut App, delta_time: f32) {
        if app.input.is_action_just_pressed("toggle_editor") {
            self.menu_shot.open = !self.menu_shot.open;
            app.debug_ui.visible = self.menu_shot.open;
        }

        self.flown += delta_time * self.menu_shot.speed;
        let heading = horizontal_from_yaw(self.menu_shot.water_yaw_deg);
        // The wave field shifts opposite the heading, so the sea travels
        // along the heading itself.
        self.ocean.set_flow(-heading.x * self.flown, -heading.z * self.flown);
        // The same foam wake the player's jet leaves when it's fast and low.
        // The jet stays put, so the trail is carried along with the sea.
        let speed = self.menu_shot.speed;
        self.ocean.set_wake_source(Some(WakeSource {
            position: self.menu_shot.plane,
            velocity: -heading * speed,
        }));
        self.ocean.scroll_wake(heading.x * speed * delta_time, heading.z * speed * delta_time);

        if let Some(plane) = scene.get_node_mut("plane").and_then(|node| node.transform_3d_mut()) {
            plane.position = self.menu_shot.plane;
            plane.rotation = plane_rotation(self.menu_shot.plane_rotation_deg);
        }
        // The title jet's flying: its wheels hidden (on this node alone - see
        // Model::mesh_mut), once its file has loaded.
        if let Some(model) = scene.get_node_mut("plane").and_then(|node| node.get_property_mut::<Model>()) {
            for mesh in model.meshes_mut().filter(|mesh| mesh.name().to_lowercase().contains("wheel")) {
                mesh.visible = false;
            }
        }
        if self.menu_shot.open {
            let shot = self.menu_shot;
            Self::place_camera(scene, shot.camera, shot.camera_yaw_deg, shot.camera_pitch_deg);
        } else {
            self.drift_camera(scene, app);
        }
        rebind_modal::update(scene, app);
        ui::update_settings_sliders(scene, app);
        ui::refresh_controller_states(scene, app);
        crate::head_tracking::poll(app);
        ui::refresh_head_tracking_if_needed(scene, app);
    }

    fn drift_camera(&mut self, scene: &mut Scene, app: &App) {
        self.elapsed += app.time.delta_time;
        let t = self.elapsed;
        let shot = self.menu_shot;

        let offset = Vector3::new(
            (t * 0.6).sin() * 0.4,
            (t * 0.9).sin() * 0.25,
            (t * 0.45).cos() * 0.35,
        );
        let yaw_drift = (t * 0.35).sin() * 1.5;
        let pitch_drift = (t * 0.5).sin() * 0.8;

        Self::place_camera(scene, shot.camera + offset, shot.camera_yaw_deg + yaw_drift, shot.camera_pitch_deg + pitch_drift);
    }

    fn place_camera(scene: &mut Scene, position: Vector3<f32>, yaw_deg: f32, pitch_deg: f32) {
        if let Some(node) = scene.get_node_mut(CAMERA) {
            node.set_property(camera_transform(position, yaw_deg, pitch_deg));
        }
    }
}

/// The menu camera's node transform at `position`, looking along
/// `yaw_deg`/`pitch_deg` (yaw 0 = +X, 90 = +Z).
fn camera_transform(position: Vector3<f32>, yaw_deg: f32, pitch_deg: f32) -> Transform3D {
    let forward = yaw_pitch_rotation(yaw_deg, pitch_deg) * Vector3::x();
    Transform3D { position, rotation: looking_along(forward), ..Default::default() }
}

/// F10's title-shot panel. `Some` is text for the clipboard.
pub fn draw_menu_shot(ctx: &egui::Context, shot: &mut MenuShot) -> Option<String> {
    let mut copied = None;
    let mut open = shot.open;
    egui::Window::new("Title shot")
        .open(&mut open)
        .default_pos(egui::pos2(20.0, 20.0))
        .resizable(false)
        .show(ctx, |ui| {
            ui.label("F10 closes this. Drag a number, or click it and type.");
            ui.separator();

            ui.label("Water");
            ui.add(egui::Slider::new(&mut shot.water_yaw_deg, -180.0..=180.0).text("heading").suffix(" deg"));
            ui.add(egui::Slider::new(&mut shot.speed, 0.0..=200.0).text("speed").suffix(" m/s"));
            let heading = horizontal_from_yaw(shot.water_yaw_deg);
            ui.label(format!("travels ({:+.2}, {:+.2}) on XZ", heading.x, heading.z));
            ui.separator();

            ui.label("Plane");
            drag_vec(ui, &mut shot.plane);
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut shot.plane_rotation_deg.x).speed(0.1).fixed_decimals(1).prefix("roll ").suffix(" deg"));
                ui.add(egui::DragValue::new(&mut shot.plane_rotation_deg.y).speed(0.1).fixed_decimals(1).prefix("pitch ").suffix(" deg"));
                ui.add(egui::DragValue::new(&mut shot.plane_rotation_deg.z).speed(0.1).fixed_decimals(1).prefix("yaw ").suffix(" deg"));
            });
            ui.separator();

            ui.label("Camera position");
            drag_vec(ui, &mut shot.camera);
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut shot.camera_yaw_deg).speed(0.1).fixed_decimals(1).prefix("yaw ").suffix(" deg"));
                ui.add(egui::DragValue::new(&mut shot.camera_pitch_deg).speed(0.1).fixed_decimals(1).prefix("pitch ").suffix(" deg"));
            });
            ui.separator();

            ui.horizontal(|ui| {
                if ui.button("Reset").clicked() {
                    let open = shot.open;
                    *shot = MenuShot::default();
                    shot.open = open;
                }
                if ui.button("Copy values").clicked() {
                    copied = Some(format!(
                        "camera: Vector3::new({:.2}, {:.2}, {:.2}), yaw {:.1}, pitch {:.1}\nplane: Vector3::new({:.2}, {:.2}, {:.2}), roll {:.1}, pitch {:.1}, yaw {:.1}\nwater heading: {:.1} deg\nspeed: {:.0} m/s",
                        shot.camera.x, shot.camera.y, shot.camera.z, shot.camera_yaw_deg, shot.camera_pitch_deg,
                        shot.plane.x, shot.plane.y, shot.plane.z,
                        shot.plane_rotation_deg.x, shot.plane_rotation_deg.y, shot.plane_rotation_deg.z,
                        shot.water_yaw_deg,
                        shot.speed,
                    ));
                }
            });
        });
    shot.open = open;
    copied
}

fn drag_vec(ui: &mut egui::Ui, value: &mut Vector3<f32>) {
    ui.horizontal(|ui| {
        for (axis, component) in ["X", "Y", "Z"].into_iter().zip(value.iter_mut()) {
            ui.add(egui::DragValue::new(component).speed(0.05).fixed_decimals(2).prefix(format!("{axis} ")));
        }
    });
}

/// The title jet: the F-16, drawn at the menu's size.
fn title_f16() -> Model {
    let scale = 29.3;
    Model::file("planes/f16/model/f16.glb").double_sided(DoubleSidedMeshes::Meshes(vec!["Afterburner".to_owned()])).transform(Transform3D { scale: Vector3::new(scale, scale, scale), ..Default::default() })
}

/// Roll, pitch, yaw in degrees, about X, Y, and Z.
fn plane_rotation(degrees: Vector3<f32>) -> UnitQuaternion<f32> {
    UnitQuaternion::from_euler_angles(degrees.x.to_radians(), degrees.y.to_radians(), degrees.z.to_radians())
}

/// Heading on the water. 0 = +X, 90 = +Z, matching the camera's yaw.
fn horizontal_from_yaw(yaw_deg: f32) -> Vector3<f32> {
    let forward = yaw_pitch_rotation(yaw_deg, 0.0) * Vector3::new(1.0, 0.0, 0.0);
    Vector3::new(forward.x, 0.0, forward.z).try_normalize(1e-6).unwrap_or_else(Vector3::x)
}

impl SceneBehaviour for GameLogic {
    fn update(&mut self, scene: &mut Scene, app: &mut App, ctx: &mut FrameContext) {
        let _ = ctx;
        self.update(scene, app, app.time.delta_time);
    }

    fn after_cameras(&mut self, scene: &mut Scene, app: &mut App) {
        // The sea's waves and wake, around where the camera is drawn from.
        self.ocean.update(scene, "world", app.time.delta_time);
    }

    fn debug_ui(&mut self, ctx: &egui::Context, _scene: &mut Scene, app: &mut App) {
        if !self.menu_shot.open {
            return;
        }
        if let Some(text) = draw_menu_shot(ctx, &mut self.menu_shot) {
            ctx.copy_text(text);
        }
        // Closed from its own button.
        if !self.menu_shot.open {
            app.debug_ui.visible = false;
        }
    }
}
