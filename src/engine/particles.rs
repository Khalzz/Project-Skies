//! GPU particles and trails, emitted from nodes - see `emitter::Emitter`
//! (what goes on a node), `effect` (what an effect looks like and does) and
//! `renderer::ParticleRenderer` (the engine side that simulates and draws
//! them).

pub mod effect;
pub mod emitter;
pub mod renderer;

pub use effect::*;
pub use emitter::{Emitter, EmitterKind, ParticleEmitters};
