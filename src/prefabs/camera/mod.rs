//! The flight cameras - the player's view and what decides which camera is
//! on screen:
//! - `behavior`: the `Camera` behavior on the player's camera node - its
//!   views (cockpit, third person, free, front), the tactical map's orbit
//!   and chart, F10's editor camera, looking around and zooming;
//! - `head_motion`: the cockpit camera's head, thrown around by the G's;
//! - `control`: `CameraControl` - what the player's camera follows, and
//!   which camera is on screen.
//!
//! Everything is re-exported here: `crate::prefabs::camera::{Camera,
//! CameraControl, PLAYER_CAMERA, ...}`.

mod behavior;
mod control;
mod head_motion;

pub use behavior::*;
pub use control::*;
