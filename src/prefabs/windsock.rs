/*
    A windsock: a pole with a sock that swings downwind and stands out straighter the harder it blows,
    with the wind speed in knots over it.
*/

use nalgebra::{UnitQuaternion, UnitVector3, Vector3};
use pankarta_software::prelude::*;
use pankarta_software::rendering::gizmos::Icon;

use crate::prefabs::world::atmosphere::Atmosphere;

const POLE_HEIGHT: f32 = 6.0;
const SOCK_MODEL: &str = "models/Windsock/windsock.glb";
const SOCK_MESH: &str = "Cylinder"; // points along -Z from where it hangs
const FULL_WIND: f32 = 15.0; // knots, when a real sock stands straight out
const MAX_DROOP: f32 = 80.0; // degrees below level, in no wind
const SWING_RATE: f32 = 2.0;
const KNOTS_PER_MS: f32 = 1.943_844;

pub struct Windsock {
    grounded: bool,
    time: f32,
    heading: Vector3<f32>, // where the sock last pointed, kept while the wind is calm
    rotation: UnitQuaternion<f32>,
}

pub fn windsock(id: &str, position: Vector3<f32>) -> Node {
    let pole = Mesh::cylinder_shape(0.06, POLE_HEIGHT, 8)
        .material(Material::standard().color(UiColor::Rgb(170, 170, 170)))
        .transform(Transform3D {
            position: Vector3::y() * (POLE_HEIGHT / 2.0),
            ..Default::default()
        });

    Node::new(id)
        .with_property(Transform3D {
            position,
            ..Default::default()
        })
        .with_property(
            Model::file(SOCK_MODEL)
                .double_sided(DoubleSidedMeshes::Meshes(vec![SOCK_MESH.to_owned()]))
                .part("pole", pole),
        )
        .with_behavior(Windsock {
            grounded: false,
            time: 0.0,
            heading: -Vector3::z(),
            rotation: UnitQuaternion::identity(),
        })
}

impl Behavior for Windsock {
    fn update(&mut self, node: &mut Node, scene: &mut SceneAccess, app: &mut App) {
        let Some(mut transform) = node.get_property::<Transform3D>().copied() else {
            return;
        };

        if !self.grounded {
            let above = transform.position + Vector3::y() * 500.0;
            if let Some(hit) = app.physics.cast_ray(above, -Vector3::y(), 2000.0, Some(&node.id)) {
                transform.position = hit.point;
                node.set_property(transform);
                self.grounded = true;
            }
        }

        let top = transform.position + Vector3::y() * POLE_HEIGHT;
        let wind = Atmosphere::of(scene).wind_at(top);
        let knots = wind.norm() * KNOTS_PER_MS;

        let level = Vector3::new(wind.x, 0.0, wind.z);
        if let Some(heading) = level.try_normalize(1e-3) {
            self.heading = heading;
        }
        // Gusts make it fill and slacken; turbulence makes it flap, harder the more it blows
        self.time += app.time.delta_time;
        let t = self.time;
        let gust = 1.0 + 0.06 * ((t * 0.35).sin() * 0.6 + (t * 0.8 + 2.0).sin() * 0.4);
        let droop = ((1.0 - (knots * gust / FULL_WIND).min(1.0)) * MAX_DROOP).to_radians();
        let pointing = self.heading * droop.cos() - Vector3::y() * droop.sin();

        let target = UnitQuaternion::face_towards(&-pointing, &Vector3::y());
        self.rotation = self
            .rotation
            .slerp(&target, (SWING_RATE * app.time.delta_time).min(1.0));

        let blowing = (knots / 3.0).min(1.0);
        let strength = (knots / FULL_WIND).min(1.0);
        let flap = |a: f32, b: f32, phase: f32| (t * a + phase).sin() * 0.6 + (t * b).sin() * 0.4;
        let swing = flap(1.7, 3.1, 1.3) * (1.0 + 1.5 * strength).to_radians() * blowing;
        let bob = flap(2.3, 4.1, 0.7) * (0.8 + 1.2 * strength).to_radians() * blowing;
        let side = UnitVector3::new_normalize(self.heading.cross(&Vector3::y()));
        let flutter = UnitQuaternion::from_axis_angle(&Vector3::y_axis(), swing)
            * UnitQuaternion::from_axis_angle(&side, bob);

        if let Some(model) = node.get_property_mut::<Model>() {
            if let Some(sock) = model.mesh_mut(SOCK_MESH) {
                sock.transform.position = Vector3::y() * POLE_HEIGHT;
                sock.transform.rotation = flutter * self.rotation;
            }
        }

        app.gizmos.channel("wind", |g| {
            g.icon(top + Vector3::y() * 1.5, Icon::Wind)
                .on_top()
                .label(format!("{knots:.0} kt"));
        });
    }
}
