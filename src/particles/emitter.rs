use super::effect::{ParticleEffect, TrailEffect};

/// A node property: one instance of an effect, emitted from wherever the
/// node is - its `Transform3D` places and turns it, and its forward (+Z) is
/// where `Motion::direction` / emission shapes are measured from. More than
/// one emitter on something means more nodes - children of it, each with
/// its own:
///
/// ```ignore
/// Node::new("plane")
///     .with_property(Model::new(..))
///     .with_child(Node::new("left_wing")
///         .with_property(Transform3D { position: Vector3::new(-9.8, 0.4, 0.3), ..Default::default() })
///         .with_property(Emitter::trail(TrailEffect::new())))
/// ```
///
/// Behaviors reach it on its node to drive it:
///
/// ```ignore
/// if let Some(emitter) = scene.child_mut(&node.id, "left_wing").and_then(|wing| wing.get_property_mut::<Emitter>()) {
///     emitter.intensity = strength;
/// }
/// ```
///
/// Removing it (or the node) stops it spawning; whatever it already put out
/// lives out its life.
#[derive(Clone, Debug)]
pub struct Emitter {
    pub kind: EmitterKind,
    /// Off = stops spawning; what's already out lives out its life.
    pub enabled: bool,
    /// 0..1 master knob - scales whatever the effect's `intensity_scales`
    /// say (rate, alpha, size, speed).
    pub intensity: f32,
    /// Meters added to its distance from the camera when emitters are put
    /// in drawing order (farthest first) - negative counts it as nearer, so
    /// it's drawn over emitters close behind it: fire over the smoke rising
    /// off it, whose clouds overlap. Something truly nearer still covers it.
    pub sort_offset: f32,
}

#[derive(Clone, Debug)]
pub enum EmitterKind {
    Particles(ParticleEffect),
    Trail(TrailEffect),
}

impl Emitter {
    pub fn particles(effect: ParticleEffect) -> Self {
        Self::new(EmitterKind::Particles(effect))
    }

    pub fn trail(effect: TrailEffect) -> Self {
        Self::new(EmitterKind::Trail(effect))
    }

    fn new(kind: EmitterKind) -> Self {
        Self { kind, enabled: true, intensity: 1.0, sort_offset: 0.0 }
    }

    pub fn intensity(mut self, intensity: f32) -> Self {
        self.intensity = intensity;
        self
    }

    /// See `sort_offset`.
    pub fn sort_offset(mut self, meters: f32) -> Self {
        self.sort_offset = meters;
        self
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }
}
