use nalgebra::{UnitQuaternion, Vector3};

use super::effect::{ParticleEffect, TrailEffect};

/// One instance of an effect, placed on a node - build it on its own, keep
/// it, clone it, then attach it with `ParticleEmitters::add`:
///
/// ```ignore
/// let left = Emitter::trail(TrailEffect::new()).at(Vector3::new(-9.8, 0.4, 0.3));
/// let right = left.clone().at(Vector3::new(9.8, 0.4, 0.3));
/// ```
///
/// It follows the node's rendered transform every frame - `offset` and
/// `rotation` are in the node's own frame.
#[derive(Clone, Debug)]
pub struct Emitter {
    pub kind: EmitterKind,
    /// Where on the node, in its local frame, in meters (not multiplied by
    /// the node's scale - that's the model's size, not the world's).
    pub offset: Vector3<f32>,
    /// Which way the emitter faces relative to the node - its forward (+Z)
    /// is where `Motion::direction` / emission shapes are measured from.
    pub rotation: UnitQuaternion<f32>,
    /// Off = stops spawning; what's already out lives out its life.
    pub enabled: bool,
    /// 0..1 master knob - scales whatever the effect's `intensity_scales`
    /// say (rate, alpha, size, speed).
    pub intensity: f32,
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
        Self { kind, offset: Vector3::zeros(), rotation: UnitQuaternion::identity(), enabled: true, intensity: 1.0 }
    }

    pub fn at(mut self, offset: Vector3<f32>) -> Self {
        self.offset = offset;
        self
    }

    pub fn facing(mut self, rotation: UnitQuaternion<f32>) -> Self {
        self.rotation = rotation;
        self
    }

    pub fn intensity(mut self, intensity: f32) -> Self {
        self.intensity = intensity;
        self
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }
}

/// A node property holding its named emitters - see `Emitter`. Behaviors
/// reach them by name to drive them:
///
/// ```ignore
/// if let Some(emitters) = node.get_property_mut::<ParticleEmitters>() {
///     emitters.get_mut("left_vortex").map(|e| e.intensity = strength);
/// }
/// ```
///
/// Removing an emitter (or the node) stops it spawning; whatever it already
/// put out lives out its life.
#[derive(Clone, Debug, Default)]
pub struct ParticleEmitters {
    emitters: Vec<(String, Emitter)>,
}

impl ParticleEmitters {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds (or replaces) an emitter, builder-style.
    pub fn add(mut self, name: impl Into<String>, emitter: Emitter) -> Self {
        self.insert(name, emitter);
        self
    }

    /// Adds (or replaces) an emitter on an existing property - e.g. from a
    /// behavior, at runtime.
    pub fn insert(&mut self, name: impl Into<String>, emitter: Emitter) {
        let name = name.into();
        match self.emitters.iter_mut().find(|(existing, _)| *existing == name) {
            Some((_, slot)) => *slot = emitter,
            None => self.emitters.push((name, emitter)),
        }
    }

    pub fn remove(&mut self, name: &str) -> Option<Emitter> {
        let index = self.emitters.iter().position(|(existing, _)| existing == name)?;
        Some(self.emitters.remove(index).1)
    }

    pub fn get(&self, name: &str) -> Option<&Emitter> {
        self.emitters.iter().find(|(existing, _)| existing == name).map(|(_, emitter)| emitter)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Emitter> {
        self.emitters.iter_mut().find(|(existing, _)| existing == name).map(|(_, emitter)| emitter)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Emitter)> {
        self.emitters.iter().map(|(name, emitter)| (name.as_str(), emitter))
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&str, &mut Emitter)> {
        self.emitters.iter_mut().map(|(name, emitter)| (name.as_str(), emitter))
    }
}
