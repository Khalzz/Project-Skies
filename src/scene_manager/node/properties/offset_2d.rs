use nalgebra::Vector2;

use super::transform::Transform2D;

/// Where a 2D visual (see `Shape2D`) sits on its node: moved by `position`
/// (UI units) from the node's position, and turned by `rotation` (radians,
/// clockwise) on top of the node's own. With `inherit_rotation` (the
/// default) both turn with the node; without it the visual stays upright -
/// a name above an enemy's head follows it around without spinning with it.
/// The node's scale scales the offset too.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Offset2D {
    pub position: Vector2<f32>,
    pub rotation: f32,
    pub inherit_rotation: bool,
}

impl Default for Offset2D {
    fn default() -> Self {
        Self { position: Vector2::zeros(), rotation: 0.0, inherit_rotation: true }
    }
}

impl Offset2D {
    /// Where a visual offset by this from a node at `transform` is, and how
    /// far it's turned.
    pub fn place(&self, transform: &Transform2D) -> (Vector2<f32>, f32) {
        let offset = self.position.component_mul(&transform.scale);
        if self.inherit_rotation {
            let (sin, cos) = transform.rotation.sin_cos();
            let turned = Vector2::new(offset.x * cos - offset.y * sin, offset.x * sin + offset.y * cos);
            (transform.position + turned, transform.rotation + self.rotation)
        } else {
            (transform.position + offset, self.rotation)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node() -> Transform2D {
        Transform2D { position: Vector2::new(100.0, 50.0), rotation: std::f32::consts::FRAC_PI_2, scale: Vector2::new(2.0, 2.0), z_index: 0 }
    }

    #[test]
    fn an_offset_turns_and_scales_with_its_node() {
        let offset = Offset2D { position: Vector2::new(10.0, 0.0), rotation: 0.25, ..Default::default() };
        let (position, rotation) = offset.place(&node());
        // A quarter turn clockwise (y down): +x becomes +y.
        assert!((position - Vector2::new(100.0, 70.0)).magnitude() < 1e-4);
        assert!((rotation - (std::f32::consts::FRAC_PI_2 + 0.25)).abs() < 1e-6);
    }

    #[test]
    fn an_upright_offset_ignores_its_nodes_rotation() {
        let offset = Offset2D { position: Vector2::new(0.0, -20.0), rotation: 0.0, inherit_rotation: false };
        let (position, rotation) = offset.place(&node());
        assert!((position - Vector2::new(100.0, 10.0)).magnitude() < 1e-4);
        assert_eq!(rotation, 0.0);
    }
}
