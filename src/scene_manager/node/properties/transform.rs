use nalgebra::{UnitQuaternion, Vector2, Vector3};

/// A 3D position/rotation/scale - a node in the 3D world. A `Node`
/// property, like any other:
///
///     Node::new("plane").with_property(Transform3D { position, ..Default::default() })
///
/// A node is in the 3D world or on the 2D plane, never both: putting this
/// on takes a `Transform2D` off.
///
/// Deliberately independent of `crate::transform::Transform` and
/// `engine::game_nodes::game_object::Transform` (both tied to the existing
/// `data.ron`-loaded object pipeline, the second via a custom `Deserialize`
/// impl converting Euler angles to a quaternion) - this property system is
/// code-first by design and doesn't go through either's deserialization path.
#[derive(Debug, Clone, Copy)]
pub struct Transform3D {
    pub position: Vector3<f32>,
    pub rotation: UnitQuaternion<f32>,
    pub scale: Vector3<f32>,
}

impl Default for Transform3D {
    fn default() -> Self {
        Transform3D {
            position: Vector3::zeros(),
            rotation: UnitQuaternion::identity(),
            scale: Vector3::new(1.0, 1.0, 1.0),
        }
    }
}

impl Transform3D {
    /// At the origin, turned so its +Z points along `direction` (+Y as close
    /// to up as it can) - for a node whose facing is what matters: a
    /// directional `Light`, a camera. No direction: not turned.
    pub fn facing(direction: Vector3<f32>) -> Self {
        let rotation = crate::scene_manager::scene_nodes::facing(direction).unwrap_or_else(UnitQuaternion::identity);
        Self { rotation, ..Default::default() }
    }

    /// Where a child sitting at `local` from this transform (its parent) is
    /// in the world - see `Parent`. The offset is in meters, turned with the
    /// parent when `inherit_rotation` (otherwise along the world's axes, and
    /// the child keeps its own rotation as is); the parent's scale touches
    /// neither, and the child keeps its own scale.
    pub fn compose(&self, local: &Transform3D, inherit_rotation: bool) -> Transform3D {
        if inherit_rotation {
            Transform3D {
                position: self.position + self.rotation * local.position,
                rotation: self.rotation * local.rotation,
                scale: local.scale,
            }
        } else {
            Transform3D { position: self.position + local.position, rotation: local.rotation, scale: local.scale }
        }
    }

    /// The reverse of `compose`: what `local` puts a child at `world`, from
    /// this transform.
    pub fn relative(&self, world: &Transform3D, inherit_rotation: bool) -> Transform3D {
        if inherit_rotation {
            let inverse = self.rotation.inverse();
            Transform3D {
                position: inverse * (world.position - self.position),
                rotation: inverse * world.rotation,
                scale: world.scale,
            }
        } else {
            Transform3D { position: world.position - self.position, rotation: world.rotation, scale: world.scale }
        }
    }
}

/// A 2D position/rotation/scale - a node on the screen or a 2D plane
/// rather than in the 3D world. A `Node` property - putting it on takes a
/// `Transform3D` off (see `Transform3D`). Position in UI units
/// from the top left (see `Ui::scale_factor`), rotation in radians,
/// clockwise.
#[derive(Debug, Clone, Copy)]
pub struct Transform2D {
    pub position: Vector2<f32>,
    pub rotation: f32,
    pub scale: Vector2<f32>,
    /// Which draws over which - higher over lower.
    pub z_index: i32,
}

impl Default for Transform2D {
    fn default() -> Self {
        Transform2D { position: Vector2::zeros(), rotation: 0.0, scale: Vector2::new(1.0, 1.0), z_index: 0 }
    }
}

/// Degrees about X, Y and Z (the map file's convention) to a rotation.
pub fn rotation_from_degrees(degrees: Vector3<f32>) -> UnitQuaternion<f32> {
    UnitQuaternion::from_euler_angles(degrees.x.to_radians(), degrees.y.to_radians(), degrees.z.to_radians())
}

/// The reverse of `rotation_from_degrees`.
pub fn degrees_from_rotation(rotation: &UnitQuaternion<f32>) -> Vector3<f32> {
    let (x, y, z) = rotation.euler_angles();
    Vector3::new(x.to_degrees(), y.to_degrees(), z.to_degrees())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parent() -> Transform3D {
        Transform3D { position: Vector3::new(100.0, 50.0, -20.0), rotation: UnitQuaternion::from_euler_angles(0.2, 1.1, -0.4), scale: Vector3::new(3.0, 3.0, 3.0) }
    }

    #[test]
    fn a_local_offset_turns_with_the_parent_and_ignores_its_scale() {
        let local = Transform3D { position: Vector3::new(0.0, 0.0, -10.0), ..Default::default() };
        let world = parent().compose(&local, true);
        assert!(((world.position - parent().position).magnitude() - 10.0).abs() < 1e-4);
        assert!((world.position - (parent().position + parent().rotation * local.position)).magnitude() < 1e-4);
    }

    #[test]
    fn relative_undoes_compose() {
        let world = Transform3D { position: Vector3::new(7.0, -3.0, 12.0), rotation: UnitQuaternion::from_euler_angles(-0.5, 0.3, 2.0), scale: Vector3::new(1.0, 1.0, 1.0) };
        for inherit_rotation in [true, false] {
            let local = parent().relative(&world, inherit_rotation);
            let back = parent().compose(&local, inherit_rotation);
            assert!((back.position - world.position).magnitude() < 1e-3, "{inherit_rotation}");
            assert!(back.rotation.angle_to(&world.rotation) < 1e-4, "{inherit_rotation}");
        }
    }
}
