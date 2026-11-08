//! Input helpers on top of the engine's `Input` (`app.input`).

use pankarta_software::prelude::Input;

/// What the game read from its own input module that the engine's `Input`
/// says differently.
pub trait InputExt {
    /// `positive`'s strength minus `negative`'s - two actions as one axis,
    /// -1..1 (the engine's `get_axis` takes input sources, not actions).
    fn action_axis(&self, negative: &str, positive: &str) -> f32;
}

impl InputExt for Input {
    fn action_axis(&self, negative: &str, positive: &str) -> f32 {
        self.action_strength(positive) - self.action_strength(negative)
    }
}
