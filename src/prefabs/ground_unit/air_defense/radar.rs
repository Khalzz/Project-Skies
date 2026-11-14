/*
    A towed search radar, built after the AN/MPQ-64 Sentinel: a single axle trailer with an
    electronics shelter, four outrigger legs and a turntable, with the rotating antenna on top.

    The antenna (array, IFF bar, back electronics, yoke) is its own "antenna" child node with one
    Model, so the whole head can be turned at once by changing that node's Transform3D (rotation
    around Y spins it like the real one, 30 rpm). The trailer is the unit's own Model.
*/

use nalgebra::{UnitQuaternion, Vector3};
use pankarta_software::prelude::*;
use pankarta_software::rendering::thick_lines::LineInstance;
use pankarta_software::scene_manager::node::properties::Parent;
use sdl2::sys::{KeyCode, SDL_Keycode};
use std::collections::HashMap;

use crate::prefabs::plane::plane::Plane;
use crate::prefabs::plane::transponder::Transponder;

use super::super::utils;

/// What the radar knows a contact is. The radar has no coalition of its own yet, so it can't
/// say friend or foe - only which coalition the contact's IFF answered with.
#[derive(Clone, Debug)]
enum ContactState {
    /// Painted once - no IFF yet.
    Unidentified,
    /// Its IFF answered: the coalition it flies for ("Blue", "Red"...).
    Identified(String),
}

#[derive(Clone, Debug)]
enum ContactAge {
    /// Painted once.
    New,
    /// Painted again on a later sweep - that's when it's interrogated (IFF).
    Old,
}

#[derive(Clone, Debug)]
pub struct Contact {
    /// Where the plane was when the beam last painted it - not where it is now.
    position: Vector3<f32>,
    state: ContactState,
    age: ContactAge,
    /// Seconds since the beam last painted it.
    since_painted: f32,
    /// Seconds since its line last flashed (a paint, or a pulse) - it fades out over
    /// `CONTACT_FADE`.
    since_flash: f32,
    /// Sweeps in a row the beam came round without finding it - one pulse each. Dropped after
    /// `LOST_PULSES`.
    pulses: u32,
}

#[derive(Clone, Debug)]
pub struct Radar {
    pub detection_range: f32,
    /// Draws the radar beam - where the antenna is looking, out to `detection_range`.
    pub debug: bool,
    contacts: HashMap<String, Contact>,
}

impl Radar {
    pub fn new(detection_range: f32, debug: bool) -> Self {
        Radar {
            detection_range,
            debug,
            contacts: HashMap::new(),
        }
    }

    pub fn prefab(self, node: Node) -> Node {
        node.with_property(Physics {
            rigidbody: RigidBodyData {
                center_of_mass: Vector3::new(0.0, 0.8, 0.0),
                ..RigidBodyData::dynamic(3_000.0)
            },
            colliders: vec![
                // Trailer, from the wheels' bottom to the frame
                Collider::new(ColliderShape::Cuboid {
                    half_extents: (1.1, 0.45, 1.8),
                })
                .offset(Vector3::new(0.0, 0.45, 0.0))
                .friction(0.8),
                // Electronics shelter
                Collider::new(ColliderShape::Cuboid {
                    half_extents: (0.9, 0.4, 1.1),
                })
                .offset(Vector3::new(0.0, 1.25, -0.2))
                .friction(0.8),
            ],
        })
        .with_property(trailer())
        .with_child(antenna())
        .with_child(utils::sensor(self.detection_range))
        .with_behavior(self)
    }

    /// Every plane in range that the beam swept over this frame is painted: a new contact the
    /// first time, identified by its IFF the second. A contact the beam comes round to and
    /// doesn't find pulses at where it was last seen, once a sweep, and is dropped after
    /// `LOST_PULSES`. The beam is only `BEAM_WIDTH` wide and turns `step` (rad) a frame - more
    /// than that at low frame rates - so it checks the whole slice it swept, not just where it
    /// ended.
    fn scan(&mut self, node: &Node, scene: &SceneAccess, step: f32, delta_time: f32) {
        for contact in self.contacts.values_mut() {
            contact.since_painted += delta_time;
            contact.since_flash += delta_time;
        }

        if let (Some((tip, rotation)), Some(unit)) = (
            self.antenna_pose(node, scene),
            node.get_property::<Transform3D>(),
        ) {
            // Which way the antenna points now, around the unit's Y axis (rad).
            let forward = unit.rotation.inverse() * (rotation * Vector3::z());
            let heading = forward.x.atan2(forward.z);
            let half_width = (BEAM_WIDTH / 2.0).to_radians();

            for plane in scene
                .nodes()
                .filter(|other| other.get_behavior::<Plane>().is_some())
            {
                let Some(position) = plane
                    .get_property::<Transform3D>()
                    .map(|transform| transform.position)
                else {
                    continue;
                };
                // Where the plane is from the antenna, in the unit's own frame.
                let offset = unit.rotation.inverse() * (position - tip);
                if offset.norm() > self.detection_range {
                    continue;
                }
                let elevation = offset.y.atan2(offset.x.hypot(offset.z)).to_degrees();
                if !(0.0..=BEAM_HEIGHT).contains(&elevation) {
                    continue;
                }
                // How far the plane is round from where the beam started this frame (0..2π).
                let behind =
                    (offset.x.atan2(offset.z) - (heading - step)).rem_euclid(std::f32::consts::TAU);
                let swept =
                    behind <= step + half_width || behind >= std::f32::consts::TAU - half_width;
                if !swept {
                    continue;
                }

                match self.contacts.get_mut(&plane.id) {
                    // Seen before: painted again - and, the first time that happens on a later
                    // sweep, its IFF. The beam stays on a plane for a few frames as it passes
                    // (it's about as wide as it turns in one), so paints less than half a turn
                    // apart are the same pass, not a second one.
                    Some(contact) => {
                        let new_pass = contact.since_painted >= 180.0 / ANTENNA_SPEED;
                        if new_pass && matches!(contact.age, ContactAge::New) {
                            contact.age = ContactAge::Old;
                            if let Some(transponder) = plane.get_property::<Transponder>() {
                                contact.state =
                                    ContactState::Identified(transponder.coalition.clone());
                            }
                        }
                        contact.position = position;
                        contact.since_painted = 0.0;
                        contact.since_flash = 0.0;
                        contact.pulses = 0;
                    }
                    None => {
                        self.contacts.insert(
                            plane.id.clone(),
                            Contact {
                                position,
                                state: ContactState::Unidentified,
                                age: ContactAge::New,
                                since_painted: 0.0,
                                since_flash: 0.0,
                                pulses: 0,
                            },
                        );
                    }
                }
            }
        }

        // Not found where the beam came round to: one pulse a sweep (a little after the beam
        // passed, in case the plane's just a bit further round), then dropped.
        let sweep = 360.0 / ANTENNA_SPEED;
        for contact in self.contacts.values_mut() {
            if contact.since_painted >= sweep * (contact.pulses + 1) as f32 + LOST_GRACE {
                contact.pulses += 1;
                contact.since_flash = 0.0;
            }
        }
        self.contacts.retain(|_, contact| {
            contact.pulses < LOST_PULSES || contact.since_flash < CONTACT_FADE
        });
    }

    /// The array's center, and which way the antenna is turned - the unit, then the antenna's
    /// turn on it (not its lean back).
    fn antenna_pose(
        &self,
        node: &Node,
        scene: &SceneAccess,
    ) -> Option<(Vector3<f32>, UnitQuaternion<f32>)> {
        let unit = node.get_property::<Transform3D>()?;
        let antenna = scene.child(&node.id, "antenna")?.get_property::<Parent>()?;
        let rotation = unit.rotation * antenna.local.rotation;
        let tip = unit.position
            + unit.rotation * antenna.local.position
            + rotation * Vector3::new(0.0, 1.1, 0.25);
        Some((tip, rotation))
    }

    /// A line from the radar to where each contact was last seen, full when it flashes (the
    /// beam painted it, or it pulsed while lost) and fading out over `CONTACT_FADE`: white while
    /// unidentified, its coalition's color once its IFF answered.
    fn draw_contacts(&self, node: &Node, scene: &SceneAccess, app: &mut App) {
        let Some((tip, _)) = self.antenna_pose(node, scene) else {
            return;
        };
        let Some(lines) = app.renderer.render_sets.get_lines_mut(RADAR_LINES) else {
            return;
        };
        for contact in self.contacts.values() {
            let opacity = 1.0 - contact.since_flash / CONTACT_FADE;
            if opacity <= 0.0 {
                continue;
            }
            let [red, green, blue] = match &contact.state {
                ContactState::Unidentified => [1.0, 1.0, 1.0],
                ContactState::Identified(coalition) if coalition == "Red" => [1.0, 0.1, 0.1],
                ContactState::Identified(coalition) if coalition == "Blue" => [0.2, 0.5, 1.0],
                // Any other coalition, for now.
                ContactState::Identified(_) => [1.0, 0.8, 0.1],
            };
            lines.dynamic.push(LineInstance {
                start: tip.into(),
                end: contact.position.into(),
                color: [red, green, blue, opacity],
                width: 3.0,
            });
        }
    }

    /// The radar beam: a thin, tall fan from the array out to the detection range, pointing
    /// where the antenna faces - `BEAM_WIDTH` wide, from the horizon up to `BEAM_HEIGHT`.
    /// Lines from the tip to the fan's four corners, plus its far edge.
    fn draw_beam(&self, node: &Node, scene: &SceneAccess, app: &mut App) {
        let Some((tip, rotation)) = self.antenna_pose(node, scene) else {
            return;
        };

        let range = self.detection_range;
        let half_width = (BEAM_WIDTH / 2.0).to_radians();
        // A point on the far edge: `side` (rad) left or right, `up` (rad) above the horizon.
        let far = |side: f32, up: f32| {
            tip + rotation
                * Vector3::new(side.sin() * up.cos(), up.sin(), side.cos() * up.cos())
                * range
        };
        let color = [1.0, 0.8, 0.1];

        // The far edge: up the left side, down the right side.
        let steps = 12;
        let mut edge = Vec::new();
        for step in 0..=steps {
            edge.push(far(
                -half_width,
                (BEAM_HEIGHT * step as f32 / steps as f32).to_radians(),
            ));
        }
        for step in (0..=steps).rev() {
            edge.push(far(
                half_width,
                (BEAM_HEIGHT * step as f32 / steps as f32).to_radians(),
            ));
        }
        app.renderer.lines.outline(&edge, color);
        // From the tip to the four corners.
        for (side, up) in [
            (-half_width, 0.0),
            (half_width, 0.0),
            (-half_width, BEAM_HEIGHT),
            (half_width, BEAM_HEIGHT),
        ] {
            app.renderer
                .lines
                .line(tip, far(side, up.to_radians()), color);
        }
    }
}

/// The trailer: frame, drawbar, axle with two wheels and fenders, the electronics shelter,
/// four outrigger legs and the turntable the antenna sits on. Facing +Z (the tow side).
fn trailer() -> Model {
    let dark = Material::standard().color(UiColor::Rgb(30, 30, 30));
    let steel = Material::standard().color(UiColor::Rgb(60, 65, 55));
    let mut model = Model::new()
        .material(Material::standard().color(UiColor::Rgb(90, 105, 70))) // default for every part
        .part(
            "frame",
            Mesh::cube()
                .transform(Transform3D {
                    position: Vector3::new(0.0, 0.75, 0.0),
                    scale: Vector3::new(1.9, 0.2, 3.6),
                    ..Default::default()
                })
                .material(steel.clone()),
        )
        .part(
            "drawbar",
            Mesh::cube()
                .transform(Transform3D {
                    position: Vector3::new(0.0, 0.7, 2.4),
                    scale: Vector3::new(0.15, 0.15, 1.2),
                    ..Default::default()
                })
                .material(steel.clone()),
        )
        .part(
            "tow_eye",
            Mesh::cylinder(0.08, 0.05)
                .transform(Transform3D {
                    position: Vector3::new(0.0, 0.7, 3.05),
                    ..Default::default()
                })
                .material(dark.clone()),
        )
        .part(
            "axle",
            Mesh::cube()
                .transform(Transform3D {
                    position: Vector3::new(0.0, 0.5, -0.3),
                    scale: Vector3::new(2.1, 0.12, 0.12),
                    ..Default::default()
                })
                .material(dark.clone()),
        )
        .part(
            "wheel_left",
            utils::wheel(Vector3::new(1.15, 0.5, -0.3)).material(dark.clone()),
        )
        .part(
            "wheel_right",
            utils::wheel(Vector3::new(-1.15, 0.5, -0.3)).material(dark.clone()),
        )
        .part(
            "fender_left",
            Mesh::cube().transform(Transform3D {
                position: Vector3::new(1.15, 1.1, -0.3),
                scale: Vector3::new(0.5, 0.06, 1.2),
                ..Default::default()
            }),
        )
        .part(
            "fender_right",
            Mesh::cube().transform(Transform3D {
                position: Vector3::new(-1.15, 1.1, -0.3),
                scale: Vector3::new(0.5, 0.06, 1.2),
                ..Default::default()
            }),
        )
        .part(
            "shelter",
            Mesh::cube().transform(Transform3D {
                position: Vector3::new(0.0, 1.25, -0.2),
                scale: Vector3::new(1.8, 0.8, 2.2),
                ..Default::default()
            }),
        )
        .part(
            "cooling_unit",
            Mesh::cube()
                .transform(Transform3D {
                    position: Vector3::new(0.0, 1.15, -1.55),
                    scale: Vector3::new(1.2, 0.6, 0.5),
                    ..Default::default()
                })
                .material(steel.clone()),
        )
        .part(
            "cabinet_front",
            Mesh::cube().transform(Transform3D {
                position: Vector3::new(0.0, 1.1, 1.35),
                scale: Vector3::new(1.4, 0.5, 0.7),
                ..Default::default()
            }),
        )
        .part(
            "turntable",
            Mesh::cylinder(0.55, 0.25)
                .transform(Transform3D {
                    position: Vector3::new(0.0, 1.775, 0.0),
                    ..Default::default()
                })
                .material(steel.clone()),
        );
    // Four outrigger legs, one per corner: an arm out from the frame and a jack down to its pad.
    for (corner, x, z) in [
        ("front_left", 1.35, 1.6),
        ("front_right", -1.35, 1.6),
        ("back_left", 1.35, -1.6),
        ("back_right", -1.35, -1.6),
    ] {
        model = model
            .part(
                format!("outrigger_arm_{corner}"),
                Mesh::cube()
                    .transform(Transform3D {
                        position: Vector3::new(x * 0.7, 0.75, z),
                        scale: Vector3::new(0.9, 0.12, 0.12),
                        ..Default::default()
                    })
                    .material(steel.clone()),
            )
            .part(
                format!("outrigger_jack_{corner}"),
                Mesh::cube()
                    .transform(Transform3D {
                        position: Vector3::new(x, 0.4, z),
                        scale: Vector3::new(0.1, 0.75, 0.1),
                        ..Default::default()
                    })
                    .material(steel.clone()),
            )
            .part(
                format!("outrigger_pad_{corner}"),
                Mesh::cube()
                    .transform(Transform3D {
                        position: Vector3::new(x, 0.025, z),
                        scale: Vector3::new(0.35, 0.05, 0.35),
                        ..Default::default()
                    })
                    .material(dark.clone()),
            );
    }
    model
}

/// The rotating head, on top of the turntable: the yoke and the phased array leaning back
/// 12°, with the IFF bar along its top and its electronics behind it. Turn the whole thing
/// with the node's rotation; the lean is the model's own transform.
fn antenna() -> Node {
    let dark = Material::standard().color(UiColor::Rgb(30, 30, 30));
    let steel = Material::standard().color(UiColor::Rgb(60, 65, 55));
    let radome = Material::standard().color(UiColor::Rgb(150, 155, 135));
    let model = Model::new()
        .material(Material::standard().color(UiColor::Rgb(90, 105, 70))) // default for every part
        .transform(Transform3D {
            rotation: UnitQuaternion::from_axis_angle(&Vector3::x_axis(), -12f32.to_radians()),
            ..Default::default()
        })
        .part(
            "yoke",
            Mesh::cube()
                .transform(Transform3D {
                    position: Vector3::new(0.0, 0.2, 0.0),
                    scale: Vector3::new(0.8, 0.4, 0.6),
                    ..Default::default()
                })
                .material(steel.clone()),
        )
        .part(
            "array",
            Mesh::cube().transform(Transform3D {
                position: Vector3::new(0.0, 1.1, 0.1),
                scale: Vector3::new(2.0, 1.4, 0.25),
                ..Default::default()
            }),
        )
        .part(
            "array_face",
            Mesh::cube()
                .transform(Transform3D {
                    position: Vector3::new(0.0, 1.1, 0.235),
                    scale: Vector3::new(1.9, 1.3, 0.02),
                    ..Default::default()
                })
                .material(radome),
        )
        .part(
            "iff_bar",
            Mesh::cube()
                .transform(Transform3D {
                    position: Vector3::new(0.0, 1.92, 0.12),
                    scale: Vector3::new(2.0, 0.2, 0.12),
                    ..Default::default()
                })
                .material(dark.clone()),
        )
        .part(
            "back_electronics",
            Mesh::cube().transform(Transform3D {
                position: Vector3::new(0.0, 1.0, -0.2),
                scale: Vector3::new(1.4, 1.0, 0.35),
                ..Default::default()
            }),
        );
    Node::new("antenna")
        .with_property(Transform3D {
            position: Vector3::new(0.0, 1.9, 0.0), // top of the turntable
            ..Default::default()
        })
        .with_property(model)
}

const ANTENNA_SPEED: f32 = 180.0;
const BEAM_WIDTH: f32 = 3.0;
const BEAM_HEIGHT: f32 = 55.0;
const CONTACT_FADE: f32 = 1.0;
const LOST_PULSES: u32 = 3;
const LOST_GRACE: f32 = 0.25;
pub const RADAR_LINES: &str = "radar_lines";

impl Behavior for Radar {
    fn update(&mut self, node: &mut Node, scene: &mut SceneAccess, app: &mut App) {
        if app
            .input
            .is_pressed(InputSource::key(pankarta_software::prelude::Key::SPACE))
        {
            self.contacts = HashMap::new();
        }

        let step = (ANTENNA_SPEED * app.time.delta_time).to_radians();
        if let Some(antenna) = scene.child_mut(&node.id, "antenna") {
            if let Some(parent) = antenna.get_property_mut::<Parent>() {
                parent.local.rotation = UnitQuaternion::from_axis_angle(&Vector3::y_axis(), step)
                    * parent.local.rotation;
            }
        }

        self.scan(node, scene, step, app.time.delta_time);

        if self.debug {
            self.draw_beam(node, scene, app);
            self.draw_contacts(node, scene, app);
        }
    }
}
