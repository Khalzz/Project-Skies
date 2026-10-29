use nalgebra::Vector2;

use crate::ui::color::UiColor;

use super::offset_2d::Offset2D;

/// What a `Shape2D` is - sizes in UI units (see `Transform2D`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShapeKind {
    Rect { width: f32, height: f32 },
    Circle { radius: f32 },
}

/// A flat shape drawn on a 2D node (one with a `Transform2D`), centered on
/// the node's position - plus its `offset`:
///
///     Node::new("box")
///         .with_property(Transform2D { position, ..Default::default() })
///         .with_property(Shape2D::square(80.0).color(UiColor::Hex("#3B82F6")))
///
/// It scales with the node (a circle by the larger of the node's two
/// scales). Drawn after the world and before the UI (see
/// `FramePlan::nodes_2d`). Which of a node's visuals is in front: the one
/// put on last - unless its `z_index` says otherwise, which replaces the
/// node's own `Transform2D::z_index` for it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shape2D {
    pub kind: ShapeKind,
    pub color: UiColor,
    pub offset: Offset2D,
    pub z_index: Option<i32>,
}

impl Shape2D {
    /// A white `width` x `height` rectangle.
    pub fn rect(width: f32, height: f32) -> Self {
        Self::new(ShapeKind::Rect { width, height })
    }

    /// A white square, `size` on each side.
    pub fn square(size: f32) -> Self {
        Self::rect(size, size)
    }

    /// A white circle.
    pub fn circle(radius: f32) -> Self {
        Self::new(ShapeKind::Circle { radius })
    }

    fn new(kind: ShapeKind) -> Self {
        Self { kind, color: UiColor::WHITE, offset: Offset2D::default(), z_index: None }
    }

    pub fn color(mut self, color: UiColor) -> Self {
        self.color = color;
        self
    }

    /// Moved `x`, `y` from the node's position (see `Offset2D`).
    pub fn offset(mut self, x: f32, y: f32) -> Self {
        self.offset.position = Vector2::new(x, y);
        self
    }

    /// Turned `radians` (clockwise) on top of the node's rotation.
    pub fn rotated(mut self, radians: f32) -> Self {
        self.offset.rotation = radians;
        self
    }

    /// Doesn't turn with the node - neither the shape nor its offset (see
    /// `Offset2D::inherit_rotation`).
    pub fn upright(mut self) -> Self {
        self.offset.inherit_rotation = false;
        self
    }

    /// Drawn at `z_index` instead of the node's (see `Transform2D::z_index`).
    pub fn z_index(mut self, z_index: i32) -> Self {
        self.z_index = Some(z_index);
        self
    }
}
