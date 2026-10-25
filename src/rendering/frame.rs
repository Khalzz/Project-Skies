//! The frame plan - what a frame draws, in order, as a list of steps instead
//! of a fixed sequence of passes written into the renderer.
//!
//! Each step is one kind of drawing:
//! - `Scene` - the models whose material draws in that pass ("opaque" starts
//!   the frame: it clears, and draws the sky; "transparent" is the see-through
//!   meshes; any other name is a pass for the game's own shaders - see
//!   engine::rendering::shader_library);
//! - `CopyDepth` - snapshots the depth so far, for passes after it whose
//!   shaders read it (`reads: [Depth]` - water fading with depth, say);
//! - `Particles` - smoke, flares, explosions, trails;
//! - `Nodes2D` - the scene's 2D nodes' visuals (see `Shape2D`);
//! - `Target`/`EndTarget` - the steps between draw into one of the game's
//!   own images (see Renderer::add_target) instead of the screen (cleared
//!   first if asked), for a later step to read - a HUD drawn apart, then
//!   blended over the world by a Fullscreen shader;
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
    Nodes2D,
    Target { name: String, clear: bool },
    EndTarget,
    Lines { set: String },
    Text { set: String },
    Ui { layer: UiLayer },
    Fullscreen { shader: String },
    Custom { name: &'static str, run: CustomStep },
}

impl FrameStep {
    /// A readable name for it - "Scene \"opaque\"", "Copy depth"...
    pub fn name(&self) -> String {
        match self {
            FrameStep::Scene { pass } => format!("Scene \"{pass}\""),
            FrameStep::CopyDepth => "Copy depth".to_owned(),
            FrameStep::Particles => "Particles".to_owned(),
            FrameStep::Nodes2D => "2D nodes".to_owned(),
            FrameStep::Target { name, .. } => format!("Target \"{name}\""),
            FrameStep::EndTarget => "End target".to_owned(),
            FrameStep::Lines { set } => format!("Lines \"{set}\""),
            FrameStep::Text { set } => format!("Text \"{set}\""),
            FrameStep::Ui { layer } => format!("UI ({layer:?})"),
            FrameStep::Fullscreen { shader } => format!("Fullscreen \"{shader}\""),
            FrameStep::Custom { name, .. } => format!("Custom \"{name}\""),
        }
    }
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
    /// see-through models, particles), copied onto the screen (engine/blit),
    /// the 2D nodes over it, and the UI over those. A game's own passes go in
    /// with `insert_after_scene` and the like.
    pub fn engine_default() -> Self {
        Self::new()
            .scene("opaque")
            .scene("transparent")
            .particles()
            .fullscreen(crate::rendering::shader_library::BLIT)
            .nodes_2d()
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

    /// The scene's 2D nodes' visuals, over the current target.
    pub fn nodes_2d(self) -> Self {
        self.step(FrameStep::Nodes2D)
    }

    /// The steps after this draw into the image `name` (added with
    /// Renderer::add_target) - cleared to transparent first if `clear` -
    /// until `end_target`.
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

    /// `steps` put in right after the scene pass `pass` - a game's own
    /// passes added to another plan (`engine_default`) without writing it
    /// out again:
    ///
    ///     FramePlan::engine_default().insert_after_scene("opaque", |steps| steps.copy_depth().scene("water"))
    ///
    /// A plan without that pass is left as it is - said in the console.
    pub fn insert_after_scene(self, pass: &str, steps: impl FnOnce(FramePlan) -> FramePlan) -> Self {
        let at = self.scene_index(pass).map(|index| index + 1);
        self.insert_at(at, pass, steps)
    }

    /// `insert_after_scene`, right before the scene pass `pass` instead.
    pub fn insert_before_scene(self, pass: &str, steps: impl FnOnce(FramePlan) -> FramePlan) -> Self {
        let at = self.scene_index(pass);
        self.insert_at(at, pass, steps)
    }

    /// Where the scene pass `pass` is in the plan.
    fn scene_index(&self, pass: &str) -> Option<usize> {
        self.steps.iter().position(|step| matches!(step, FrameStep::Scene { pass: own } if own == pass))
    }

    fn insert_at(mut self, at: Option<usize>, pass: &str, steps: impl FnOnce(FramePlan) -> FramePlan) -> Self {
        let Some(at) = at else {
            eprintln!("Frame plan: there's no scene pass \"{pass}\" to put steps next to - left as it is");
            return self;
        };
        let inserted = steps(FramePlan::new()).steps;
        self.steps.splice(at..at, inserted);
        self
    }

    /// Says once that this plan's step refers to `what`, which doesn't exist.
    pub(crate) fn report(&mut self, what: String) {
        if self.reported.insert(what.clone()) {
            eprintln!("Frame plan: {what} - skipped");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(plan: &FramePlan) -> Vec<String> {
        plan.steps.iter().map(FrameStep::name).collect()
    }

    #[test]
    fn a_game_pass_goes_in_next_to_the_one_it_names() {
        let plan = FramePlan::new().scene("opaque").scene("transparent")
            .insert_after_scene("opaque", |steps| steps.copy_depth().scene("water"))
            .insert_before_scene("opaque", |steps| steps.scene("sky"));
        assert_eq!(names(&plan), ["Scene \"sky\"", "Scene \"opaque\"", "Copy depth", "Scene \"water\"", "Scene \"transparent\""]);
        // A pass the plan doesn't have: nothing changes.
        let unchanged = plan.insert_after_scene("lava", |steps| steps.scene("glow"));
        assert_eq!(names(&unchanged).len(), 5);
    }
}
