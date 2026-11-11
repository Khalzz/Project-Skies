//! Several named particle emitters on one node.
//!
//! The engine draws one `Emitter` per node ("more than one emitter on
//! something means more nodes"). The game puts many on one node - a plane's
//! wingtip vortices, tyre smoke, afterburner, wreck fire - each at its own
//! spot on it, reached by name. So the game keeps them as one
//! `ParticleEmitters` property, and `sync` mirrors every one of them, once a
//! frame, onto a node of its own carrying the engine's `Emitter`, placed on
//! its owner where it says. Removing an emitter, or its owner, removes its
//! node - what it already put out lives out its life.

use std::collections::HashSet;

use nalgebra::{UnitQuaternion, Vector3};
use pankarta_software::particles::{Emitter as EngineEmitter, EmitterKind, ParticleEffect, TrailEffect};
use pankarta_software::prelude::*;

/// One instance of an effect, placed on a node - build it on its own, keep
/// it, clone it, then attach it with `ParticleEmitters::add`:
///
/// ```ignore
/// let left = Emitter::trail(TrailEffect::new()).at(Vector3::new(-9.8, 0.4, 0.3));
/// let right = left.clone().at(Vector3::new(9.8, 0.4, 0.3));
/// ```
///
/// It follows the node every frame - `offset` and `rotation` are in the
/// node's own frame.
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
    /// See the engine's `Emitter::sort_offset`.
    pub sort_offset: f32,
}

impl Emitter {
    pub fn particles(effect: ParticleEffect) -> Self {
        Self::new(EmitterKind::Particles(effect))
    }

    pub fn trail(effect: TrailEffect) -> Self {
        Self::new(EmitterKind::Trail(effect))
    }

    fn new(kind: EmitterKind) -> Self {
        Self { kind, offset: Vector3::zeros(), rotation: UnitQuaternion::identity(), enabled: true, intensity: 1.0, sort_offset: 0.0 }
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

    pub fn sort_offset(mut self, meters: f32) -> Self {
        self.sort_offset = meters;
        self
    }

    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    fn engine(&self) -> EngineEmitter {
        let mut emitter = match &self.kind {
            EmitterKind::Particles(effect) => EngineEmitter::particles(effect.clone()),
            EmitterKind::Trail(effect) => EngineEmitter::trail(effect.clone()),
        };
        emitter.enabled = self.enabled;
        emitter.intensity = self.intensity;
        emitter.sort_offset = self.sort_offset;
        emitter
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
#[derive(Clone, Debug, Default)]
pub struct ParticleEmitters {
    emitters: Vec<(String, Emitter)>,
    /// Rebuilt since the last sync - every emitter node is made again (a
    /// new effect), not just moved.
    rebuilt: bool,
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
        self.rebuilt = true;
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

    /// Replaces every emitter - a plane's effects rebuilt from its file.
    pub fn replace(&mut self, other: ParticleEmitters) {
        self.emitters = other.emitters;
        self.rebuilt = true;
    }
}

/// Marks a node `sync` made for one of `owner`'s emitters.
struct EmitterOf {
    owner: String,
    name: String,
}

const SEPARATOR: &str = "#emitter:";

fn emitter_node_id(owner: &str, name: &str) -> String {
    format!("{owner}{SEPARATOR}{name}")
}

/// Mirrors every node's `ParticleEmitters` onto emitter nodes (see the
/// module doc) - once a frame, after the behaviors have driven them (the
/// scene's `late_update`).
pub fn sync(scene: &mut Scene, app: &mut App) {
    // Every emitter there should be, where, and as what.
    let mut wanted: Vec<(String, Transform3D, EngineEmitter, bool)> = Vec::new();
    let mut owners_rebuilt = Vec::new();
    for node in scene.content.nodes.iter() {
        let Some(emitters) = node.get_property::<ParticleEmitters>() else { continue };
        let Some(world) = scene.world_transform(&node.id) else { continue };
        if emitters.rebuilt {
            owners_rebuilt.push(node.id.clone());
        }
        for (name, emitter) in emitters.iter() {
            let transform = Transform3D {
                position: world.position + world.rotation * emitter.offset,
                rotation: world.rotation * emitter.rotation,
                scale: Vector3::new(1.0, 1.0, 1.0),
            };
            wanted.push((emitter_node_id(&node.id, name), transform, emitter.engine(), emitters.rebuilt));
        }
    }
    for owner in &owners_rebuilt {
        if let Some(emitters) = scene.get_node_mut(owner).and_then(|node| node.get_property_mut::<ParticleEmitters>()) {
            emitters.rebuilt = false;
        }
    }

    // The ones there are now that shouldn't be.
    let wanted_ids: HashSet<&str> = wanted.iter().map(|(id, ..)| id.as_str()).collect();
    let stale: Vec<String> = scene.content.nodes.iter()
        .filter(|node| node.get_property::<EmitterOf>().is_some() && !wanted_ids.contains(node.id.as_str()))
        .map(|node| node.id.clone())
        .collect();
    for id in stale {
        scene.despawn_node(app, &id);
    }

    for (id, transform, emitter, rebuilt) in wanted {
        match scene.get_node_mut(&id) {
            Some(node) => {
                node.set_property(transform);
                match node.get_property_mut::<EngineEmitter>() {
                    // Rebuilt: a new effect; otherwise only its knobs.
                    Some(existing) if !rebuilt => {
                        existing.enabled = emitter.enabled;
                        existing.intensity = emitter.intensity;
                        existing.sort_offset = emitter.sort_offset;
                    }
                    _ => node.set_property(emitter),
                }
            }
            None => {
                let (owner, name) = id.split_once(SEPARATOR).map(|(owner, name)| (owner.to_owned(), name.to_owned())).unwrap_or_default();
                let node = Node::new(id.clone())
                    .with_property(transform)
                    .with_property(emitter)
                    .with_property(EmitterOf { owner, name });
                if let Err(error) = scene.spawn_node(app, node) {
                    eprintln!("couldn't add emitter node '{id}': {error}");
                }
            }
        }
    }
}

/// The owner and emitter name of an emitter node `sync` made - None for any
/// other node.
pub fn emitter_of(node: &Node) -> Option<(&str, &str)> {
    node.get_property::<EmitterOf>().map(|of| (of.owner.as_str(), of.name.as_str()))
}
