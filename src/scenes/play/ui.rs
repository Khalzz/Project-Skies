//! The play scene's debug overlay, top left: the frame rate, where the camera is and, toggled with F4, the flight
//! data of the aircraft the camera is looking at, read from its own sensors and systems.

use pankarta_software::prelude::*;
use pankarta_software::ui::ui_node::UiNode;
use pankarta_software::ui::ui_transform::SizeValue;
use pankarta_software::ui::widgets::text;
use pankarta_software::ui::TextStyle;

use crate::prefabs::aircraft::aircraft::Aircraft;
use crate::prefabs::camera::camera::Camera;
use crate::scenes::play::battleground::battleground::Player;

const FLIGHT_LINES: usize = 8;
const KNOTS_PER_MS: f32 = 1.943_844;
const FEET_PER_M: f32 = 3.280_84;

/// The UI's node, following the camera node `camera` (its id).
pub fn node(camera: &str) -> Node {
    let line = |placeholder: &str| text(placeholder, &TextStyle::default()).set_size(SizeValue::Pixels(520.0), SizeValue::Fit);
    let mut root = UiNode::container()
        .set_padding((10.0, 10.0))
        .set_child("framerate", line("- fps"))
        .set_child("camera_position", line("x - y - z -"));
    for index in 0..FLIGHT_LINES {
        root = root.set_child(format!("flight_{index}"), line(""));
    }
    Node::new("UI")
        .with_property(UiLayer::new(root))
        .with_behavior(SandboxUi { camera: camera.to_owned(), show_flight: true })
}

/// Fills the UI's text in, every frame.
pub struct SandboxUi {
    /// The camera whose position it shows, by id.
    camera: String,
    show_flight: bool,
}

impl Behavior for SandboxUi {
    fn update(&mut self, node: &mut Node, scene: &mut SceneAccess, app: &mut App) {
        if app.input.is_just_pressed(InputSource::key(Key::F4)) {
            self.show_flight = !self.show_flight;
        }

        let camera = scene.get_node(&self.camera);
        let position = camera.and_then(|camera| camera.get_property::<Transform3D>()).map(|transform| app.origin.to_world(transform.position));
        let target = camera
            .and_then(|camera| camera.get_behavior::<Camera>())
            .and_then(|camera| camera.following())
            .and_then(|id| scene.get_node(id));
        let lines = match target {
            Some(target) if self.show_flight => flight_lines(target, app),
            _ => vec![],
        };

        let Some(ui) = node.get_property_mut::<UiLayer>() else { return };
        if let Some(label) = ui.label_mut("framerate") {
            label.set_text(format!("{} fps", app.time.fps().round()));
        }
        if let (Some(label), Some(position)) = (ui.label_mut("camera_position"), position) {
            label.set_text(format!("x {:.1}  y {:.1}  z {:.1}", position.x, position.y, position.z));
        }
        for index in 0..FLIGHT_LINES {
            if let Some(label) = ui.label_mut(&format!("flight_{index}")) {
                label.set_text(lines.get(index).cloned().unwrap_or_default());
            }
        }
    }
}

// What the aircraft `target` knows about itself, one line each
fn flight_lines(target: &Node, app: &App) -> Vec<String> {
    let Some(aircraft) = target.get_behavior::<Aircraft>() else {
        return vec![format!("{}: not an aircraft", target.id)];
    };
    let sensors = &aircraft.sensors;
    let rates = sensors.rates.map(f32::to_degrees);
    let ground_speed = app.physics.body(&target.id).map_or(0.0, |body| body.linvel().norm());
    let gear = &aircraft.airframe.landing_gear;
    let engines = aircraft.powerplant.engines();
    let thrust: f32 = engines.iter().map(|engine| engine.thrust).sum();
    let lever = engines.first().map_or(0.0, |engine| engine.lever);
    let afterburner = engines.iter().map(|engine| engine.afterburner()).fold(0.0, f32::max);
    let role = if target.get_property::<Player>().is_some() { "  (player)" } else { "" };

    vec![
        format!("--- {}{} ---", target.id, role),
        format!(
            "speed {:.0} kt  ground {:.0} kt  mach {:.2}",
            sensors.airspeed * KNOTS_PER_MS,
            ground_speed * KNOTS_PER_MS,
            sensors.mach
        ),
        format!(
            "altitude {:.0} ft ({:.0} m)  q {:.1} kPa",
            sensors.altitude * FEET_PER_M,
            sensors.altitude,
            sensors.dynamic_pressure / 1000.0
        ),
        format!("aoa {:+.1}°  sideslip {:+.1}°  load {:.2} g", sensors.alpha, sensors.beta, sensors.load_factor),
        format!("pitch {:+.1}  roll {:+.1}  yaw {:+.1} °/s", rates.x, rates.y, rates.z),
        format!(
            "throttle {:.0}%  thrust {:.1} kN  afterburner {:.0}%",
            lever * 100.0,
            thrust / 1000.0,
            afterburner * 100.0
        ),
        format!(
            "gear {:?} ({:?})  weight on wheels {}",
            gear.lever(),
            gear.get_state(),
            if sensors.weight_on_wheels { "yes" } else { "no" }
        ),
        format!(
            "mass {:.0} kg",
            app.physics.body(&target.id).map_or(0.0, |body| body.mass())
        ),
    ]
}
