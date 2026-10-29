//! The particle system: a row of effects on a ground, and a spinner above
//! them drawing two trails. A/D turn the camera around them, W/S bring it
//! closer or further, Space turns the spinner's trails on and off, Q blows
//! wind across everything.
//!
//! Every effect is an `Emitter` property on its own node - placed and
//! turned by the node's `Transform3D` (an emitter sprays along its node's
//! +Z). The fire's smoke is a child of the fire ("fire/smoke"), and the
//! spinner's trails children of the spinner ("spinner/left" and
//! "spinner/right"), which its behavior reaches with `scene.child_mut`.

use pankarta_software::particles::{Emitter, EmissionShape, Facing, Forces, ParticleEffect, TrailEffect};
use pankarta_software::prelude::*;
use pankarta_software::rendering::ui::ui::Ui;
use pankarta_software::ui::layer::Layer;
use pankarta_software::ui::ui_transform::{PositionValue, SizeValue};
use pankarta_software::ui::widgets::text;
use pankarta_software::ui::TextStyle;

const GROUND_TEXTURE: &str = "textures/grid/Dark/texture_01.png";

/// Turns the camera around the "focus" node (it looks at it - see `LookAt`).
struct Orbit {
    /// Around the focus, radians.
    angle: f32,
    /// From the focus, meters.
    distance: f32,
}

impl Behavior for Orbit {
    fn update(&mut self, node: &mut Node, _scene: &mut SceneAccess, app: &mut App) {
        let dt = app.time.delta_time;
        self.angle += input::get_axis("orbit_left", "orbit_right") * 1.2 * dt;
        self.distance = (self.distance + input::get_axis("closer", "further") * 12.0 * dt).clamp(6.0, 60.0);
        if let Some(transform) = node.get_property_mut::<Transform3D>() {
            transform.position = Vector3::new(self.angle.sin() * self.distance, self.distance * 0.35, -self.angle.cos() * self.distance);
        }
    }
}

/// Spins its node around Y, and turns its trails on and off.
struct Spinner {
    /// Radians a second.
    speed: f32,
}

impl Behavior for Spinner {
    fn update(&mut self, node: &mut Node, scene: &mut SceneAccess, app: &mut App) {
        if let Some(transform) = node.get_property_mut::<Transform3D>() {
            transform.rotation = UnitQuaternion::from_axis_angle(&Vector3::y_axis(), self.speed * app.time.delta_time) * transform.rotation;
        }
        if input::is_just_pressed("toggle_trails") {
            for name in ["left", "right"] {
                if let Some(emitter) = scene.child_mut(&node.id, name).and_then(|child| child.get_property_mut::<Emitter>()) {
                    emitter.enabled = !emitter.enabled;
                }
            }
        }
    }
}

/// A node at `x` on the ground, facing up - so its emitter sprays upward.
fn emitter_node(id: &str, x: f32, emitter: Emitter) -> Node {
    let mut transform = Transform3D::facing(Vector3::y());
    transform.position = Vector3::new(x, 0.0, 0.0);
    Node::new(id).with_property(transform).with_property(emitter)
}

fn fountain() -> Emitter {
    Emitter::particles(ParticleEffect::new()
        .rate(400.0)
        .lifetime(1.6, 2.0)
        .cone(10.0)
        .speed(9.0, 11.0)
        .gravity(1.0)
        .drag(0.1)
        .facing(Facing::Velocity { stretch: 0.04 })
        .size_over_life(0.15, 0.08)
        .color_over_life([0.6, 0.8, 1.0, 0.9], [0.8, 0.9, 1.0, 0.0])
        .lit()
        .max_particles(1000))
}

fn fire() -> Emitter {
    Emitter::particles(ParticleEffect::new()
        .rate(150.0)
        .lifetime(0.6, 1.2)
        // Flat, facing the node's +Z - lying on the ground.
        .shape(EmissionShape::Disk { radius: 0.5 })
        .cone(10.0)
        .speed(1.5, 3.0)
        // Rises.
        .gravity(-0.3)
        .size_over_life(1.2, 0.3)
        .color_over_life([3.0, 1.2, 0.3, 1.0], [0.6, 0.1, 0.0, 0.0])
        .brightness(2.0)
        .additive())
        // Drawn over the smoke rising off it, though the smoke's emitter is
        // above it - nearer the camera looking down.
        .sort_offset(-2.0)
}

fn smoke() -> Emitter {
    Emitter::particles(ParticleEffect::new()
        .rate(30.0)
        .lifetime(3.0, 5.0)
        // Always up, however its node is turned.
        .world_direction(Vector3::y())
        .cone(15.0)
        .speed(0.8, 1.5)
        .forces(Forces { gravity: -0.05, drag: 0.5, wind: 1.0, turbulence: 1.5 })
        .spin(-30.0, 30.0)
        .size_over_life(0.8, 4.0)
        .color_over_life([0.25, 0.25, 0.25, 0.6], [0.4, 0.4, 0.4, 0.0])
        .lit())
}

fn sparks() -> Emitter {
    Emitter::particles(ParticleEffect::new()
        // Bursts only: 120 at once, every 1.5 s.
        .rate(0.0)
        .burst(120, Some(1.5))
        .lifetime(0.6, 1.4)
        .cone(60.0)
        .speed(6.0, 12.0)
        .gravity(1.0)
        .drag(0.3)
        .facing(Facing::Velocity { stretch: 0.03 })
        .size_over_life(0.06, 0.02)
        .color_over_life([4.0, 2.5, 0.8, 1.0], [1.0, 0.3, 0.0, 0.0])
        .additive()
        .max_particles(500))
}

fn debris() -> Emitter {
    Emitter::particles(ParticleEffect::new()
        .rate(20.0)
        .lifetime(1.4, 1.6)
        .cone(35.0)
        .speed(6.0, 8.0)
        .gravity(1.0)
        .drag(0.0)
        .spin(-180.0, 180.0)
        .faceted(6)
        .size_over_life(0.4, 0.4)
        .color_over_life([0.35, 0.25, 0.15, 1.0], [0.35, 0.25, 0.15, 1.0])
        .lit())
}

fn orb() -> Emitter {
    Emitter::particles(ParticleEffect::new()
        .rate(200.0)
        .lifetime(1.0, 2.0)
        .shape(EmissionShape::Sphere { radius: 1.0 })
        // Every way.
        .cone(180.0)
        .speed(0.2, 0.5)
        .forces(Forces { gravity: 0.0, drag: 1.0, wind: 0.3, turbulence: 2.0 })
        .size_over_life(0.2, 0.0)
        .color_over_life([1.5, 0.4, 2.5, 1.0], [0.4, 0.1, 1.0, 0.0])
        .additive())
}

fn trail(color: [f32; 3]) -> Emitter {
    let [r, g, b] = color;
    Emitter::trail(TrailEffect::new()
        .lifetime(1.2)
        .segment_spacing(0.2)
        .width_over_life(0.4, 0.0)
        .color_over_life([r, g, b, 1.0], [r, g, b, 0.0])
        .brightness(2.0)
        .additive()
        .unlit())
}

struct ParticlesScene {
    windy: bool,
}

impl ParticlesScene {
    fn new(scene: &mut Scene, app: &mut App) -> Self {
        app.renderer.set_environment(Environment::ProceduralSky { sun: true });

        let ground = Node::new("ground")
            .with_property(Transform3D::default())
            .with_property(Model::new(ModelSource::Plane { subdivisions: 0 }).with_scale(Vector3::new(200.0, 1.0, 200.0)))
            .with_property(Material::standard().texture(GROUND_TEXTURE).tiled(4.0, TextureSpace::World));

        let sun = Node::new("sun")
            .with_property(Transform3D::facing(Vector3::new(-0.3, -1.0, -0.2)))
            .with_property(Light::directional([1.0, 1.0, 1.0]));

        let focus = Node::new("focus").with_property(Transform3D { position: Vector3::new(0.0, 2.0, 0.0), ..Default::default() });
        let camera = Node::new("camera")
            .with_property(Transform3D::default())
            .with_property(CameraLens::new(70.0))
            .with_property(LookAt { target: "focus".into() })
            .with_behavior(Orbit { angle: 0.0, distance: 24.0 });

        // The smoke 1.2 m above the fire - along the fire node's +Z, which
        // points up.
        let fire = emitter_node("fire", -6.0, fire())
            .with_child(Node::new("smoke")
                .with_property(Transform3D { position: Vector3::new(0.0, 0.0, 1.2), ..Default::default() })
                .with_property(smoke()));

        let spinner = Node::new("spinner")
            .with_property(Transform3D { position: Vector3::new(0.0, 7.0, 0.0), ..Default::default() })
            .with_behavior(Spinner { speed: 4.0 })
            .with_child(Node::new("left")
                .with_property(Transform3D { position: Vector3::new(-3.0, 0.0, 0.0), ..Default::default() })
                .with_property(trail([0.3, 0.8, 1.0])))
            .with_child(Node::new("right")
                .with_property(Transform3D { position: Vector3::new(3.0, 0.0, 0.0), ..Default::default() })
                .with_property(trail([1.0, 0.5, 0.2])));

        let nodes = vec![
            ground, sun, focus, camera, fire, spinner,
            emitter_node("fountain", -12.0, fountain()),
            emitter_node("sparks", 0.0, sparks()),
            emitter_node("debris", 6.0, debris()),
            {
                let mut orb = emitter_node("orb", 12.0, orb());
                if let Some(transform) = orb.get_property_mut::<Transform3D>() {
                    transform.position.y = 2.0;
                }
                orb
            },
        ];
        for node in nodes {
            if let Err(error) = scene.spawn_node(app, node) {
                eprintln!("{error}");
            }
        }

        // The frame rate top left, the controls bottom left.
        let framerate = text(app, "- fps", &TextStyle::default())
            .set_padding((10.0, 10.0))
            .set_size(SizeValue::Pixels(200.0), SizeValue::Fit);
        let help = text(app, "A/D orbit   W/S zoom   Space trails   Q wind", &TextStyle::default())
            .set_padding((10.0, 10.0))
            .set_size(SizeValue::Pixels(500.0), SizeValue::Fit)
            .set_position(PositionValue::Start(0.0), PositionValue::End(0.0));
        Layer::new(app).set_child("framerate", framerate).set_child("help", help).build(app);

        Self { windy: false }
    }
}

impl SceneBehaviour for ParticlesScene {
    fn update(&mut self, _scene: &mut Scene, app: &mut App, _ctx: &mut FrameContext) {
        if input::is_just_pressed("wind") {
            self.windy = !self.windy;
            app.particles.set_wind(if self.windy { Vector3::new(6.0, 0.0, 0.0) } else { Vector3::zeros() });
        }
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

    app.window.set_title("Particles");
    app.renderer.set_vsync(false);

    for (action, key) in [("orbit_left", Key::A), ("orbit_right", Key::D), ("closer", Key::W), ("further", Key::S), ("toggle_trails", Key::Space), ("wind", Key::Q)] {
        input::bind(action, InputSource::key(key));
    }

    app.scene_manager.create_scene("main", ParticlesScene::new);
    app.scene_manager.open_scene("main");
    app.run();
}
