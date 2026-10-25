use nalgebra::{Vector3};
use pankarta_software::app::App;
use pankarta_software::input::source::{InputSource, Key};
use pankarta_software::input::input;
use pankarta_software::rendering::ui::ui::Ui;
use pankarta_software::scene_manager::behavior::Behavior;
use pankarta_software::scene_manager::node::Node;
use pankarta_software::scene_manager::node::properties::{rotation_from_degrees, CameraLens, LookAt, Model, Shape2D, Transform2D, Transform3D};
use pankarta_software::scene_manager::scene::SceneBehaviour;
use pankarta_software::ui::TextStyle;
use pankarta_software::ui::layer::Layer;
use pankarta_software::ui::ui_transform::SizeValue;
use pankarta_software::ui::widgets::text;

// Orbits the camera around a node: moving the mouse turns it around
// (left/right) and over (up/down) it, always `distance` away. Facing it is
// LookAt's job (see the camera node below).
struct CameraMovement {
  target: String,
  distance: f32,
  // Around the target (radians): 0 = behind it (-Z), turning to its right.
  yaw: f32,
  // Over the target (radians): 0 = level, up to straight above/below.
  pitch: f32,
  // Radians per pixel the mouse moves.
  sensitivity: f32,
}

impl Behavior for CameraMovement {
  fn update(&mut self, node: &mut Node, scene: &mut pankarta_software::scene_manager::scene_access::SceneAccess, _app: &mut App) {
    self.yaw += input::mouse_rel_x() as f32 * self.sensitivity;
    self.pitch = (self.pitch + input::mouse_rel_y() as f32 * self.sensitivity).clamp(-1.5, 1.5);

    let center = scene.get_node("plane").unwrap().get_property::<Transform3D>().unwrap().position;

    // calculate offset from the object to the camera
    let offset = Vector3::new(
      self.yaw.sin() * self.pitch.cos(),
      self.pitch.sin(),
      -self.yaw.cos() * self.pitch.cos(),
    ) * self.distance;

    // apply transform to the camera
    if let Some(transform) = node.get_property_mut::<Transform3D>() {
      transform.position = center + offset;
    }
  }
}

// Scene and behaviour

#[derive(Default)]
struct ModelViewer {
    frames: u32,
    elapsed: f32,
}

impl SceneBehaviour for ModelViewer {
  fn on_spawn(&mut self, scene: &mut pankarta_software::scene_manager::scene::Scene, app: &mut App) {

    let framerate_label = text(app, "600 fps", &TextStyle::default())
        .set_padding((10.0, 10.0))
        .set_size(SizeValue::Pixels(200.0), SizeValue::Fit);

    Layer::new(app).set_child("framerate", framerate_label).build(app);

    app.window.set_cursor_locked(true);

    let model_node = Node::new("plane")
      .add_property(Model::new("models/f16.glb").scaled(15.0)) // the file is ~1 unit long; an F-16 is ~15 m
      .add_property(Transform3D { position: Vector3::new(0.0,0.0,0.0), rotation: rotation_from_degrees(Vector3::new(0.0, 0.0, 0.0)), scale: Vector3::new(1.0,1.0,1.0), });

    scene.spawn_node(app, model_node).unwrap();

    let camera = Node::new("camera")
      .add_property(Transform3D { position: Vector3::new(0.0, 5.0, -25.0), ..Default::default() })
      .add_property(CameraLens::new(60.0))
      .add_property(LookAt { target: "plane".into() })
      .add_behavior(CameraMovement { target: "plane".into(), distance: 25.0, yaw: 0.0, pitch: 0.2, sensitivity: 0.005 });

    scene.spawn_node(app, camera).unwrap();
  }

  fn update(&mut self, scene: &mut pankarta_software::prelude::Scene, app: &mut App, ctx: &mut pankarta_software::prelude::FrameContext) {
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

    // window setup
    app.window.set_title("2D Example");
    app.renderer.set_vsync(false);


    // Input binding setup
    input::bind("Left", InputSource::key(Key::A));
    input::bind("Right", InputSource::key(Key::D));
    input::bind("Up", InputSource::key(Key::W));
    input::bind("Down", InputSource::key(Key::S));

    // creating and openning the scene
    app.scene_manager.create_scene("main", |_scene, _app| {      
      ModelViewer::default()
    });

    app.scene_manager.open_scene("main");

    // running the game
    app.run();
}
