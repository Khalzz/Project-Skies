use nalgebra::{Point3, UnitQuaternion, Vector3};

use crate::app::App;
use crate::engine::game_nodes::game_object::Lighting;
use crate::engine::rendering::enviroment::environment::Environment;
use crate::engine::scene_manager::node::Node;
use crate::engine::scene_manager::properties::{Model, Transform3D};
use crate::engine::rendering::camera::camera::yaw_pitch_rotation;
use crate::engine::input::input;
use crate::engine::rendering::render_pipeline::water_renderer::WakeSource;
use crate::engine::scene_manager::scene::{FrameContext, Scene, SceneBehaviour};
#[allow(unused_imports)]
use crate::game::camera::camera::{Camera, CameraConfig, Fov, Speed};
use crate::resources;
use crate::transform::Transform;

use super::rebind_modal;
use super::ui;

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
}

impl GameLogic {
    // this is called once, when the scene becomes active. Synchronous - no
    // level to background-load any more (see main.rs's own comment on why
    // this scene moved off create_loaded_scene).
    pub fn new(scene: &mut Scene, app: &mut App, environment: Environment) -> Self {
        resources::apply_environment(scene, app, environment);

        app.window_manager.context.mouse().set_relative_mouse_mode(false);

        ui::build(app);

        Self::spawn_world(scene, app);

        let camera = scene.create_camera("main_menu", Transform::new(BASE_POSITION, yaw_pitch_rotation(BASE_YAW_DEG, BASE_PITCH_DEG), Vector3::new(1.0, 1.0, 1.0)), 70.0);
        // camera.look_at = Some(LookAtTarget::Node("plane".to_owned()));
        scene.select_camera("main_menu");

        app.menu_shot = MenuShot::default();

        Self { elapsed: 0.0, flown: 0.0 }
    }

    fn spawn_world(scene: &mut Scene, app: &mut App) {
        scene.spawn_node(app,
            Node::new("sun")
                .add_property(Transform3D {
                    // Only its direction from the world origin matters - the
                    // sun is treated as infinitely far away (see App::run's
                    // lighting update). 40 degrees above the horizon.
                    position: Vector3::new(445_200.0, 642_800.0, 623_300.0),
                    rotation: UnitQuaternion::identity(),
                    scale: Vector3::new(1.0, 1.0, 1.0),
                })
                .add_property(Model::new(crate::MARKER_MODEL))
        ).expect("main_menu should only spawn 'sun' once");
        if let Some(sun) = scene.content.renderizable_instances.get_mut("sun") {
            sun.instance.metadata.lighting = Some(Lighting { intensity: 1.0, color: Vector3::new(0.7, 0.7, 0.8) });
        }

        scene.spawn_node(app,
            Node::new("plane")
                .add_property(Transform3D {
                    position: PLANE_POSITION,
                    rotation: plane_rotation(PLANE_ROTATION_DEG),
                    scale: Vector3::new(1.0, 1.0, 1.0),
                })
                // Its own copy of the jet, so hiding the wheels here doesn't
                // hide them on the one you fly. Drawn bigger than the file
                // (the model shrank to real size, x0.4777) so the shot stays framed.
                .add_property(title_f16(app))
        ).expect("main_menu should only spawn 'plane' once");
        if let Some(source) = app.model_sources.get("F16") {
            hide_wheels(app, &source.clone().variant("title").key());
        }

        scene.spawn_node(app,
            Node::new("world")
                .add_property(Transform3D {
                    position: Vector3::new(0.0, -10.0, 0.0),
                    rotation: UnitQuaternion::identity(),
                    // The ocean ring mesh is in world units (see main.rs's
                    // OCEAN_SURFACE_MODEL) - centered here, near the menu's
                    // slowly drifting camera, its finest rings are in view.
                    scale: Vector3::new(1.0, 1.0, 1.0),
                })
                .add_property(Model::new(crate::OCEAN_SURFACE_MODEL))
                .add_property(crate::engine::scene_manager::properties::Material::shader(crate::engine::rendering::shader_library::WATER))
        ).expect("main_menu should only spawn 'world' once");
    }

    pub fn update(&mut self, scene: &mut Scene, app: &mut App, delta_time: f32) {
        if input::is_action_just_pressed("toggle_editor") {
            app.menu_shot.open = !app.menu_shot.open;
        }

        self.flown += delta_time * app.menu_shot.speed;
        let heading = horizontal_from_yaw(app.menu_shot.water_yaw_deg);
        // The wave field shifts opposite the heading, so the sea travels
        // along the heading itself.
        app.water.set_flow(-heading.x * self.flown, -heading.z * self.flown);
        // The same foam wake the player's jet leaves when it's fast and low.
        // The jet stays put, so the trail is carried along with the sea.
        let speed = app.menu_shot.speed;
        app.water.set_wake_source(Some(WakeSource {
            position: app.menu_shot.plane,
            velocity: -heading * speed,
        }));
        app.water.scroll_wake(heading.x * speed * delta_time, heading.z * speed * delta_time);

        if let Some(plane) = scene.content.nodes.get_mut("plane").and_then(|node| node.get_property_mut::<Transform3D>()) {
            plane.position = app.menu_shot.plane;
            plane.rotation = plane_rotation(app.menu_shot.plane_rotation_deg);
        }
        if app.menu_shot.open {
            self.place_camera(scene, app.menu_shot.camera, app.menu_shot.camera_yaw_deg, app.menu_shot.camera_pitch_deg);
        } else {
            self.drift_camera(scene, app);
        }
        rebind_modal::update(app);
        ui::update_settings_sliders(app);
        ui::refresh_controller_states(app);
        crate::game::head_tracking::poll(app);
        ui::refresh_head_tracking_if_needed(app);

        app.ui.has_changed = true;
    }

    fn drift_camera(&mut self, scene: &mut Scene, app: &App) {
        self.elapsed += app.time.delta_time;
        let t = self.elapsed;
        let shot = app.menu_shot;

        let offset = Vector3::new(
            (t * 0.6).sin() * 0.4,
            (t * 0.9).sin() * 0.25,
            (t * 0.45).cos() * 0.35,
        );
        let yaw_drift = (t * 0.35).sin() * 1.5;
        let pitch_drift = (t * 0.5).sin() * 0.8;

        self.place_camera(scene, shot.camera + offset, shot.camera_yaw_deg + yaw_drift, shot.camera_pitch_deg + pitch_drift);
    }

    fn place_camera(&self, scene: &mut Scene, position: Vector3<f32>, yaw_deg: f32, pitch_deg: f32) {
        if let Some(camera) = scene.cameras.get_mut("main_menu") {
            camera.camera.set_position(Point3::from(position));
            camera.camera.set_yaw_pitch(yaw_deg.to_radians(), pitch_deg.to_radians());
        }
    }
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

/// The title jet: the declared F-16, its own mesh copy, drawn at the menu's size.
fn title_f16(app: &App) -> Model {
    match app.model_sources.get("F16") {
        Some(source) => Model::from_source(source.clone().variant("title")).scaled(29.3),
        None => Model::new("F16").scaled(29.3),
    }
}

/// Drops the wheel meshes from the title jet. `num_elements` of 0 draws nothing.
fn hide_wheels(app: &mut App, model_ref: &str) {
    let Some(model) = app.game_models.get_mut(model_ref) else { return };
    for meshes in model.model.mesh_lists.values_mut() {
        for (key, mesh) in meshes.iter_mut() {
            let key = key.to_ascii_lowercase();
            let name = mesh.name.to_ascii_lowercase();
            if key.contains("wheel") || name.contains("wheel") {
                mesh.num_elements = 0;
            }
        }
    }
}

/// Roll, pitch, yaw in degrees, about X, Y, and Z.
fn plane_rotation(degrees: Vector3<f32>) -> UnitQuaternion<f32> {
    UnitQuaternion::from_euler_angles(degrees.x.to_radians(), degrees.y.to_radians(), degrees.z.to_radians())
}

/// Heading on the water. 0 = +X, 90 = +Z, matching the camera's yaw.
fn horizontal_from_yaw(yaw_deg: f32) -> Vector3<f32> {
    let forward = UnitQuaternion::from_quaternion(yaw_pitch_rotation(yaw_deg, 0.0)) * Vector3::new(1.0, 0.0, 0.0);
    Vector3::new(forward.x, 0.0, forward.z).try_normalize(1e-6).unwrap_or_else(Vector3::x)
}

impl SceneBehaviour for GameLogic {
    fn update(&mut self, scene: &mut Scene, app: &mut App, ctx: &mut FrameContext) {
        let _ = ctx;
        self.update(scene, app, app.time.delta_time);
    }
}
