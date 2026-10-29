/// Makes a node a light - placed and turned by its `Transform3D`, like any
/// other node, so a behavior turning the node turns the light (a day going
/// by is a sun node being rotated).
///
///     Node::new("sun")
///         .with_property(Transform3D::facing(Vector3::new(-0.3, -1.0, -0.2)))
///         .with_property(Light::directional([1.0, 0.95, 0.9]))
///
/// Only directional lights for now: the scene is lit by its first one (the
/// sun), shining along its node's +Z - the way a camera looks. A scene
/// without one has no sunlight, and its procedural sky no sun disc.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Light {
    pub kind: LightKind,
    /// Linear RGB, 0..1.
    pub color: [f32; 3],
    /// How bright - multiplies `color`. 1 by default, more for a brighter
    /// light.
    pub intensity: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LightKind {
    /// Infinitely far away, the same direction everywhere - the sun. Where
    /// its node is doesn't matter, only where it's turned.
    Directional,
}

impl Light {
    /// A sun of `color`.
    pub fn directional(color: [f32; 3]) -> Self {
        Self { kind: LightKind::Directional, color, intensity: 1.0 }
    }

    pub fn intensity(mut self, intensity: f32) -> Self {
        self.intensity = intensity;
        self
    }

    /// The light it gives: `color` times `intensity`.
    pub(crate) fn radiance(&self) -> [f32; 3] {
        self.color.map(|channel| channel * self.intensity)
    }
}
