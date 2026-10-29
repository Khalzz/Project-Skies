//! Water, made entirely by the game: a sea with waves, wooden crates that
//! fall into it and float, posts sticking out of it with foam around them.
//! Space drops another crate.
//!
//! Nothing here is the engine's: the water is a game shader (water.wgsl,
//! water.ron) drawn in a pass the game adds to its frame plan, and the
//! floating is a game `Behavior` (`Buoyancy`) pushing each crate up where
//! it's under the waves, every physics step (its `fixed_update`). The waves are defined once, in `WAVES` - the
//! shader gets them as params, so what's drawn is what things float on.

use nalgebra::{Point3, Vector3};
use pankarta_software::app::App;
use pankarta_software::game_nodes::game_object::{Collider, ColliderShape, Physics, RigidBodyData};
use pankarta_software::input::input;
use pankarta_software::input::source::{InputSource, Key};
use pankarta_software::rendering::frame::FramePlan;
use pankarta_software::rendering::ui::ui::Ui;
use pankarta_software::resources::ModelSource;
use pankarta_software::scene_manager::behavior::Behavior;
use pankarta_software::scene_manager::node::properties::{rotation_from_degrees, CameraLens, Light, LookAt, Material, Model, Transform3D};
use pankarta_software::scene_manager::node::Node;
use pankarta_software::scene_manager::scene::{FrameContext, Scene, SceneBehaviour};
use pankarta_software::scene_manager::scene_access::SceneAccess;
use pankarta_software::ui::layer::Layer;
use pankarta_software::ui::ui_transform::SizeValue;
use pankarta_software::ui::widgets::text;
use pankarta_software::ui::TextStyle;

// The sea

/// The waves: direction x, direction z, wavelength (m), height (m). The
/// shader's `params.waves` - keep its array size the same.
const WAVES: [[f32; 4]; 3] = [
    [1.0, 0.2, 9.0, 0.22],
    [-0.4, 1.0, 5.5, 0.12],
    [0.7, -0.7, 3.1, 0.05],
];
const GRAVITY: f32 = 9.81;
/// Kilograms per cubic meter.
const WATER_DENSITY: f32 = 1000.0;
/// How far down the seabed is.
const SEABED: f32 = -6.0;
/// How wide the sea is, on each side.
const SEA_SIZE: f32 = 120.0;

/// The water's height at world `x`, `z` at `time` (seconds - the shaders'
/// `engine.time`) - water.wgsl's `wave_height`, the same sum.
fn wave_height(x: f32, z: f32, time: f32) -> f32 {
    WAVES.iter().map(|[dx, dz, wavelength, height]| {
        let direction = nalgebra::Vector2::new(*dx, *dz).normalize();
        let k = std::f32::consts::TAU / wavelength;
        let phase = k * (direction.x * x + direction.y * z) - (GRAVITY * k).sqrt() * time;
        height * phase.sin()
    }).sum()
}

// Floating

/// Floats a box on the waves, every physics step: the box is split into a
/// grid of small cells, and each one under the water is pushed up by the
/// weight of the water it displaces and slowed by the water's drag - pushed
/// where it is, so a box tips and settles the way it lies in the water.
struct Buoyancy {
    half_extents: Vector3<f32>,
    /// How much a cell under water is slowed, per second.
    drag: f32,
    /// Where each cell under water was pushed last step, and how hard - for
    /// the physics debug view.
    pushes: Vec<(Vector3<f32>, Vector3<f32>)>,
}

impl Buoyancy {
    /// Cells along each side of the box.
    const CELLS: usize = 3;

    fn new(half_extents: Vector3<f32>) -> Self {
        Self { half_extents, drag: 1.5, pushes: Vec::new() }
    }
}

impl Behavior for Buoyancy {
    fn fixed_update(&mut self, node: &mut Node, _scene: &mut SceneAccess, app: &mut App) {
        // The waves as they're drawn (the shaders' `engine.time`).
        let time = app.time.shader_time();
        let dt = app.physics.timestep();
        let Some(body) = app.physics.body_mut(&node.id) else { return };

        let cells = Self::CELLS as f32;
        let size = self.half_extents * 2.0;
        let cell = size / cells;
        let cell_volume = cell.x * cell.y * cell.z;
        let cell_mass = body.mass() / (cells * cells * cells);
        let pose = *body.position();

        self.pushes.clear();
        for i in 0..Self::CELLS {
            for j in 0..Self::CELLS {
                for k in 0..Self::CELLS {
                    let local = Point3::new(
                        -self.half_extents.x + cell.x * (i as f32 + 0.5),
                        -self.half_extents.y + cell.y * (j as f32 + 0.5),
                        -self.half_extents.z + cell.z * (k as f32 + 0.5),
                    );
                    let point = pose * local;
                    // How much of the cell is under the surface: 0 above it,
                    // 1 once its top is (a cell's height around its center).
                    let depth = wave_height(point.x, point.z, time) - point.y;
                    let under = (depth / cell.y + 0.5).clamp(0.0, 1.0);
                    if under <= 0.0 {
                        continue;
                    }
                    let lift = Vector3::y() * WATER_DENSITY * GRAVITY * cell_volume * under;
                    let drag = -body.velocity_at_point(&point) * self.drag * cell_mass * under;
                    body.apply_impulse_at_point((lift + drag) * dt, point, true);
                    self.pushes.push((point.coords, lift));
                }
            }
        }
    }

    fn update(&mut self, _node: &mut Node, _scene: &mut SceneAccess, app: &mut App) {
        if let Some(lines) = app.physics.debug_lines() {
            for (point, lift) in &self.pushes {
                lines.ray(*point, lift * 0.0002, [0.2, 0.6, 1.0]);
            }
        }
    }
}

// The scene

/// A wooden crate - 600 kg a cubic meter, so it floats a little over half
/// under - falling from `position`, tipped by `tilt` (degrees).
fn crate_node(id: &str, position: Vector3<f32>, tilt: Vector3<f32>) -> Node {
    let half = Vector3::new(0.5, 0.5, 0.5);
    let volume = 8.0 * half.x * half.y * half.z;
    Node::new(id)
        .with_property(Transform3D { position, rotation: rotation_from_degrees(tilt), ..Default::default() })
        .with_property(Model::new(ModelSource::Cube).with_scale(half * 2.0))
        .with_property(Physics {
            rigidbody: RigidBodyData::dynamic(600.0 * volume),
            colliders: vec![Collider::new(ColliderShape::Cuboid { half_extents: (half.x, half.y, half.z) })],
        })
        .with_behavior(Buoyancy::new(half))
}

/// A post standing on the seabed, sticking out of the water.
fn post_node(id: &str, x: f32, z: f32) -> Node {
    let half = Vector3::new(0.4, 4.5, 0.4);
    Node::new(id)
        .with_property(Transform3D { position: Vector3::new(x, SEABED + half.y, z), ..Default::default() })
        .with_property(Model::new(ModelSource::Cube).with_scale(half * 2.0))
        .with_property(Physics {
            rigidbody: RigidBodyData::fixed(),
            colliders: vec![Collider::new(ColliderShape::Cuboid { half_extents: (half.x, half.y, half.z) })],
        })
}

/// Circles the camera slowly around the middle of the sea.
struct Orbit {
    angle: f32,
}

impl Behavior for Orbit {
    fn update(&mut self, node: &mut Node, _scene: &mut SceneAccess, app: &mut App) {
        self.angle += app.time.delta_time * 0.12;
        if let Some(transform) = node.get_property_mut::<Transform3D>() {
            transform.position = Vector3::new(self.angle.sin() * 16.0, 6.0, self.angle.cos() * 16.0);
        }
    }
}

struct WaterScene {
    crates: u32,
}

impl WaterScene {
    fn drop_crate(&mut self, scene: &mut Scene, app: &mut App) {
        self.crates += 1;
        let position = Vector3::new(rand::random::<f32>() * 10.0 - 5.0, 8.0, rand::random::<f32>() * 10.0 - 5.0);
        let tilt = Vector3::new(rand::random::<f32>() * 60.0, rand::random::<f32>() * 360.0, rand::random::<f32>() * 60.0);
        if let Err(error) = scene.spawn_node(app, crate_node(&format!("crate_{}", self.crates), position, tilt)) {
            eprintln!("{error}");
        }
    }
}

impl SceneBehaviour for WaterScene {
    fn on_spawn(&mut self, scene: &mut Scene, app: &mut App) {
        // The sea: a fine grid for the waves to move, drawn with water.wgsl,
        // its waves from WAVES.
        let waves: Vec<f32> = WAVES.iter().flatten().copied().collect();
        let sea = Node::new("sea")
            .with_property(Transform3D::default())
            .with_property(Model::new(ModelSource::Plane { subdivisions: 239 }).with_scale(Vector3::new(SEA_SIZE, 1.0, SEA_SIZE)))
            .with_property(Material::shader("water").param("waves", waves));

        // The seabed - a body for things to land on, and something for the
        // water to be deep over.
        let seabed = Node::new("seabed")
            .with_property(Transform3D { position: Vector3::new(0.0, SEABED, 0.0), ..Default::default() })
            .with_property(Model::new(ModelSource::Plane { subdivisions: 0 }).with_scale(Vector3::new(SEA_SIZE, 1.0, SEA_SIZE)))
            .with_property(Physics {
                rigidbody: RigidBodyData::fixed(),
                colliders: vec![Collider::new(ColliderShape::HalfSpace { normal: Vector3::y() })],
            });

        let focus = Node::new("focus").with_property(Transform3D::default());
        let camera = Node::new("camera")
            .with_property(Transform3D::default())
            .with_property(CameraLens::new(60.0))
            .with_property(LookAt { target: "focus".into() })
            .with_behavior(Orbit { angle: 0.0 });

        let sun = Node::new("sun")
            .with_property(Transform3D::facing(Vector3::new(-0.3, -1.0, -0.2)))
            .with_property(Light::directional([1.0, 1.0, 1.0]));

        let mut nodes = vec![sea, seabed, focus, camera, sun, post_node("post_a", -3.0, 2.0), post_node("post_b", 4.0, -3.5)];
        for (index, x) in [-2.0, 0.5, 3.0].into_iter().enumerate() {
            self.crates += 1;
            nodes.push(crate_node(&format!("crate_{}", self.crates), Vector3::new(x, 5.0 + index as f32 * 2.0, 0.0), Vector3::new(25.0, index as f32 * 40.0, 10.0)));
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
    }

    fn update(&mut self, scene: &mut Scene, app: &mut App, _ctx: &mut FrameContext) {
        if let Some(label) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "framerate").and_then(|node| node.as_label_mut()) {
            let fps = app.time.fps().round();
            label.set_text(&mut app.ui.text.font_system, &format!("{fps} fps"), true);
        }

        if input::is_just_pressed(InputSource::key(Key::Space)) {
            self.drop_crate(scene, app);
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
    app.window.set_title("Water");

    app.renderer.set_vsync(false);

    // The water's shader - from its file, so saving it while this runs
    // rebuilds it.
    app.renderer.shaders.add_file("water", concat!(env!("CARGO_MANIFEST_DIR"), "/examples/3D/water/water.wgsl"));

    // The engine's frame, with the water put in after the solid world: a
    // copy of its depth for the water to read, then the water over it.
    app.renderer.default_frame_plan = FramePlan::engine_default()
        .insert_after_scene("opaque", |steps| steps.copy_depth().scene("water"));

    app.scene_manager.create_scene("main", |_scene, _app| WaterScene { crates: 0 });
    app.scene_manager.open_scene("main");
    app.run();
}
