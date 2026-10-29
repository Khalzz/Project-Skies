//! A free camera over a ground: WASD to move (Space up, Left Ctrl down, Left
//! Shift faster), the mouse to look around, the scroll wheel to zoom (the
//! field of view). Escape frees the mouse, and clicking takes it back.
//!
//! The ground is the engine's plane and the cubes its cube
//! (`ModelSource::Plane`, `ModelSource::Cube`). Their textures are tiled (`Material::tiled`) at a fixed size however big each
//! surface is - the ground's in the world, the cubes' on each cube, so it
//! stays put on them. All loaded with the scene, through the AssetManager.

use nalgebra::{UnitQuaternion, Vector2, Vector3};
use pankarta_software::prelude::*;
use pankarta_software::rendering::ui::ui::Ui;
use pankarta_software::ui::layer::Layer;
use pankarta_software::ui::ui_transform::SizeValue;
use pankarta_software::ui::widgets::text;
use pankarta_software::ui::TextStyle;

const GROUND_TEXTURE: &str = "textures/grid/Dark/texture_01.png";
const CUBE_TEXTURES: [&str; 4] = [
    "textures/grid/Orange/texture_01.png",
    "textures/grid/Green/texture_01.png",
    "textures/grid/Purple/texture_01.png",
    "textures/grid/Red/texture_01.png",
];

/// Radians the camera turns for one count of mouse movement (at the player's
/// sensitivity of 1).
const RADIANS_PER_COUNT: f32 = 0.0004;

/// Flies the camera node: the mouse turns it, WASD moves it where it looks,
/// the scroll wheel narrows or widens its view.
struct FreeCamera {
    /// Turned left and right, and up and down (radians).
    yaw: f32,
    pitch: f32,
    /// Meters a second.
    speed: f32,
    /// Seconds the mouse's movement is smoothed over - 0: off, the camera
    /// turns exactly as the mouse moves. A little (0.01 - 0.03) hides the
    /// mouse's uneven counts when it moves slowly, at a bit of lag.
    smoothing: f32,
    /// The look movement so far smoothed (see `smoothing`).
    smoothed_look: Vector2<f32>,
    /// The field of view the scroll wheel asks for (degrees) - the lens eases
    /// toward it. None until the first frame, which takes the lens's own.
    target_fov: Option<f32>,
    /// Seconds the zoom takes to get most of the way to `target_fov`.
    zoom_smoothing: f32,
    /// Temporary: frame times and mouse movement, logged with MOUSE_DIAG set
    /// - see `MouseDiagnostic`.
    diagnostic: Option<MouseDiagnostic>,
}

/// Temporary, to find where mouse-look stutter comes from: with the
/// MOUSE_DIAG environment variable set, prints frame-time stats every 3 s,
/// and - once the mouse moves - its movement over the next 240 frames.
#[derive(Default)]
struct MouseDiagnostic {
    frame_times: Vec<f32>,
    since_report: f32,
    /// Frame time (ms) and mouse movement (x, y) - collected once it moves.
    moving: Vec<(f32, i32, i32)>,
}

impl MouseDiagnostic {
    const MOVING_FRAMES: usize = 240;

    fn record(&mut self, dt: f32, rel_x: i32, rel_y: i32) {
        let ms = dt * 1000.0;
        self.frame_times.push(ms);
        self.since_report += dt;
        if self.since_report >= 3.0 {
            let mut sorted = self.frame_times.clone();
            sorted.sort_by(f32::total_cmp);
            let average = sorted.iter().sum::<f32>() / sorted.len() as f32;
            let p95 = sorted[(sorted.len() as f32 * 0.95) as usize - 1];
            eprintln!("[mouse-diag] frames {} | ms avg {:.2} min {:.2} max {:.2} p95 {:.2}", sorted.len(), average, sorted[0], sorted[sorted.len() - 1], p95);
            self.frame_times.clear();
            self.since_report = 0.0;
        }

        if self.moving.is_empty() && rel_x == 0 && rel_y == 0 {
            return;
        }
        self.moving.push((ms, rel_x, rel_y));
        if self.moving.len() == Self::MOVING_FRAMES {
            let still = self.moving.iter().filter(|(_, x, y)| *x == 0 && *y == 0).count();
            let total_x: i32 = self.moving.iter().map(|(_, x, _)| x.abs()).sum();
            eprintln!("[mouse-diag] {} frames while moving: {} with no mouse movement, |x| total {}", self.moving.len(), still, total_x);
            let first: Vec<String> = self.moving.iter().take(60).map(|(ms, x, y)| format!("{ms:.1}ms:{x},{y}")).collect();
            eprintln!("[mouse-diag] first 60 (frame ms : x,y): {}", first.join(" "));
            self.moving.clear();
        }
    }
}

impl Behavior for FreeCamera {
    fn update(&mut self, node: &mut Node, _scene: &mut SceneAccess, app: &mut App) {
        if let Some(diagnostic) = &mut self.diagnostic {
            diagnostic.record(app.time.unscaled_delta_time, input::mouse_rel_x(), input::mouse_rel_y());
        }
        if input::is_just_pressed(InputSource::key(Key::Escape)) {
            app.window.set_cursor_locked(false);
        } else if input::mouse_left_button_just_pressed() {
            app.window.set_cursor_locked(true);
        }

        if app.window.is_cursor_locked() {
            // The player's sensitivity is in mouse_look already - this
            // camera only says what one count of it turns.
            let look = input::mouse_look() * RADIANS_PER_COUNT;
            let dt = app.time.unscaled_delta_time;
            self.smoothed_look = if self.smoothing > 0.0 {
                self.smoothed_look + (look - self.smoothed_look) * (1.0 - (-dt / self.smoothing).exp())
            } else {
                look
            };
            // The camera looks along +Z: turning toward its right is a
            // negative turn about Y, looking down a positive one about X.
            self.yaw -= self.smoothed_look.x;
            self.pitch = (self.pitch + self.smoothed_look.y).clamp(-1.5, 1.5);
        } else {
            self.smoothed_look = Vector2::zeros();
        }
        let rotation = UnitQuaternion::from_axis_angle(&Vector3::y_axis(), self.yaw) * UnitQuaternion::from_axis_angle(&Vector3::x_axis(), self.pitch);

        let forward = rotation * Vector3::z();
        let right = forward.cross(&Vector3::y()).normalize();
        let walk = input::get_vector("left", "right", "forward", "back").normalize();
        let climb = input::get_axis("down", "up");
        let speed = if input::is_pressed("fast") { self.speed * 4.0 } else { self.speed };
        let movement = (forward * -walk.0.y + right * walk.0.x + Vector3::y() * climb) * speed * app.time.unscaled_delta_time;

        if let Some(transform) = node.get_property_mut::<Transform3D>() {
            transform.position += movement;
            transform.rotation = rotation;
        }
        if let Some(lens) = node.get_property_mut::<CameraLens>() {
            let target = self.target_fov.get_or_insert(lens.fovy);
            *target = (*target - input::mouse_scroll_y() * 3.0).clamp(15.0, 110.0);
            // Eased the same at any frame rate.
            let ease = 1.0 - (-app.time.unscaled_delta_time / self.zoom_smoothing.max(0.0001)).exp();
            lens.fovy += (*target - lens.fovy) * ease;
        }
    }
}

struct FreeCameraScene;

impl FreeCameraScene {
    fn new(scene: &mut Scene, app: &mut App) -> Self {
        app.renderer.set_environment(Environment::ProceduralSky { sun: true });

        let ground = Node::new("ground")
            .with_property(Transform3D::default())
            .with_property(Model::new(ModelSource::Plane { subdivisions: 0 }).with_scale(Vector3::new(200.0, 1.0, 200.0)))
            .with_property(Material::standard().texture(GROUND_TEXTURE).tiled(4.0, TextureSpace::World));

        let camera = Node::new("camera")
            .with_property(Transform3D { position: Vector3::new(0.0, 2.0, -10.0), ..Default::default() })
            .with_property(CameraLens::new(70.0))
            .with_behavior(FreeCamera { yaw: 0.0, pitch: 0.0, speed: 6.0, smoothing: 0.0, smoothed_look: Vector2::zeros(), target_fov: None, zoom_smoothing: 0.08, diagnostic: std::env::var("MOUSE_DIAG").is_ok().then(MouseDiagnostic::default) });

        let sun = Node::new("sun")
            .with_property(Transform3D::facing(Vector3::new(-0.3, -1.0, -0.2)))
            .with_property(Light::directional([1.0, 1.0, 1.0]));

        let mut nodes = vec![ground, camera, sun];
        // Landmarks: cubes of different heights in a ring.
        for index in 0..12 {
            let angle = index as f32 / 12.0 * std::f32::consts::TAU;
            let height = 1.0 + (index % 4) as f32;
            nodes.push(Node::new(format!("cube_{index}"))
                .with_property(Transform3D { position: Vector3::new(angle.sin() * 20.0, height / 2.0, angle.cos() * 20.0), ..Default::default() })
                .with_property(Model::new(ModelSource::Cube).with_scale(Vector3::new(2.0, height, 2.0)))
                .with_property(Material::standard().texture(CUBE_TEXTURES[index % 4]).tiled(1.0, TextureSpace::Object)));
        }
        for node in nodes {
            if let Err(error) = scene.spawn_node(app, node) {
                eprintln!("{error}");
            }
        }

        // The frame rate, top left.
        let framerate = text(app, "- fps", &TextStyle::default())
            .set_padding((10.0, 10.0))
            .set_size(SizeValue::Pixels(200.0), SizeValue::Fit);
        Layer::new(app).set_child("framerate", framerate).build(app);

        app.window.set_cursor_locked(true);
        Self
    }
}

impl SceneBehaviour for FreeCameraScene {
    fn update(&mut self, _scene: &mut Scene, app: &mut App, _ctx: &mut FrameContext) {
        if let Some(label) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "framerate").and_then(|node| node.as_label_mut()) {
            let fps = app.time.fps().round();
            label.set_text(&mut app.ui.text.font_system, &format!("{fps} fps"), true);
        }
    }
}

#[tokio::main]
async fn main() {
    let mut app = match App::new().await {
        Ok(app) => app,
        Err(err) => {
            eprintln!("Couldn't create the app: {err}");
            return;
        }
    };

    input::set_mouse_sensitivity(2.0, 2.0);
    app.window.set_title("Free camera");
    app.renderer.set_vsync(false);

    for (action, key) in [("forward", Key::W), ("back", Key::S), ("left", Key::A), ("right", Key::D), ("up", Key::Space), ("down", Key::LCtrl), ("fast", Key::LShift)] {
        input::bind(action, InputSource::key(key));
    }

    app.scene_manager.create_scene("main", FreeCameraScene::new);
    app.scene_manager.open_scene("main");
    app.run();
}
