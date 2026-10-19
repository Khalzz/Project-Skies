//! The frame plan - what a frame draws, in order, as a list of steps instead
//! of a fixed sequence of passes written into the renderer.
//!
//! Each step is one kind of drawing:
//! - `Scene` - the models whose material draws in that pass ("opaque" starts
//!   the frame: it clears, and draws the sky; then "water", "transparent";
//!   any other name is a pass for the game's own shaders - see
//!   engine::rendering::shader_library);
//! - `CopyDepth` - snapshots the depth so far, for passes after it that
//!   read it (the water fading with depth);
//! - `Particles` - smoke, flares, explosions, trails;
//! - `Target`/`EndTarget` - the steps between draw into a named image
//!   instead of the screen (cleared first if asked), for a later step to
//!   read - like the flight HUD's own image, which the G-force effects blend
//!   over the world;
//! - `Lines`/`Text` - a named set of thick lines or SDF text;
//! - `Ui` - one layer of the UI;
//! - `Fullscreen` - a Fullscreen shader run over the whole of the current
//!   target, by material name (see engine::rendering::shader_library) - how
//!   the world gets onto the screen (engine/blit, or a game's own);
//! - `Custom` - anything else, as code given the encoder and the target.
//!
//! Each scene can set its own (`App::frame_plan`, in its start); on a scene
//! change it goes back to `App::default_frame_plan`, which the game sets
//! once (otherwise `engine_default`). Line and text sets are the scene's own
//! too (see engine::rendering::render_sets); passes, targets and full-screen
//! shaders are still the engine's, by name.

use crate::app::App;
use crate::rendering::render_pipeline::render_pass::UiLayer;

/// A step's code, for `FrameStep::Custom`: the app, this frame's encoder,
/// and the image the plan is drawing into at that point.
pub type CustomStep = fn(&mut App, &mut wgpu::CommandEncoder, &wgpu::TextureView);

/// One step of a `FramePlan` - see the module doc.
#[derive(Clone)]
pub enum FrameStep {
    Scene { pass: String },
    CopyDepth,
    Particles,
    Target { name: String, clear: bool },
    EndTarget,
    Lines { set: String },
    Text { set: String },
    Ui { layer: UiLayer },
    Fullscreen { shader: String },
    Custom { name: &'static str, run: CustomStep },
}

/// What a frame draws, in order - see the module doc.
#[derive(Clone, Default)]
pub struct FramePlan {
    pub steps: Vec<FrameStep>,
    /// Names a step referred to that don't exist, already reported - so a
    /// mistake is said once, not every frame.
    pub(crate) reported: std::collections::HashSet<String>,
}

impl FramePlan {
    pub fn new() -> Self {
        Self::default()
    }

    /// What a scene draws when nothing says otherwise: the world (models,
    /// a "water" pass for a sea, see-through models, particles), copied onto
    /// the screen (engine/blit), and the UI over it.
    pub fn engine_default() -> Self {
        Self::new()
            .scene("opaque")
            .copy_depth()
            .scene("water")
            .scene("transparent")
            .particles()
            .fullscreen(crate::rendering::shader_library::BLIT)
            .ui(UiLayer::All)
    }

    fn step(mut self, step: FrameStep) -> Self {
        self.steps.push(step);
        self
    }

    pub fn scene(self, pass: impl Into<String>) -> Self {
        self.step(FrameStep::Scene { pass: pass.into() })
    }

    pub fn copy_depth(self) -> Self {
        self.step(FrameStep::CopyDepth)
    }

    pub fn particles(self) -> Self {
        self.step(FrameStep::Particles)
    }

    /// The steps after this draw into the image `name` - cleared to
    /// transparent first if `clear` - until `end_target`.
    pub fn target(self, name: impl Into<String>, clear: bool) -> Self {
        self.step(FrameStep::Target { name: name.into(), clear })
    }

    pub fn end_target(self) -> Self {
        self.step(FrameStep::EndTarget)
    }

    pub fn lines(self, set: impl Into<String>) -> Self {
        self.step(FrameStep::Lines { set: set.into() })
    }

    pub fn text(self, set: impl Into<String>) -> Self {
        self.step(FrameStep::Text { set: set.into() })
    }

    pub fn ui(self, layer: UiLayer) -> Self {
        self.step(FrameStep::Ui { layer })
    }

    pub fn fullscreen(self, shader: impl Into<String>) -> Self {
        self.step(FrameStep::Fullscreen { shader: shader.into() })
    }

    pub fn custom(self, name: &'static str, run: CustomStep) -> Self {
        self.step(FrameStep::Custom { name, run })
    }

    /// Says once that this plan's step refers to `what`, which doesn't exist.
    pub(crate) fn report(&mut self, what: String) {
        if self.reported.insert(what.clone()) {
            eprintln!("Frame plan: {what} - skipped");
        }
    }
}
