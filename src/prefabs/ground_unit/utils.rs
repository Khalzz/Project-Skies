//! What ground units share: shapes their models are built with (a wheel),
//! and the pieces their nodes are given (a sensor).

use nalgebra::{UnitQuaternion, Vector3};
use pankarta_software::prelude::*;

/// A wheel for a model's `part`: a cylinder (its axis along Y) laid on its
/// side - axle across the unit - its center at `at` (m, from the unit's
/// ground point).
pub fn wheel(at: Vector3<f32>) -> Mesh {
    let on_its_side = UnitQuaternion::from_axis_angle(&Vector3::z_axis(), std::f32::consts::FRAC_PI_2);
    Mesh::cylinder(0.5, 0.4).transform(Transform3D { position: at, rotation: on_its_side, ..Default::default() })
}

/// What a unit sees: a ball `range` (m) round it, 3 m up - the "sensor"
/// child. On a node of its own, so it's a body of its own that just
/// follows the unit (a ball that big on the unit's own body would count
/// towards its mass). It detects every moving body inside it.
pub fn sensor(range: f32) -> Node {
    Node::new("sensor")
        .with_property(Transform3D { position: Vector3::new(0.0, 3.0, 0.0), ..Default::default() })
        .with_property(Sensor::sphere(range))
}
