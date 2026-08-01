use std::collections::HashMap;
use std::sync::mpsc::Sender;

use sdl2::{controller::GameController, EventPump};

use crate::app::{App, AppState};
use crate::input::input::InputSubsystem;
use crate::physics::physics::DebugPhysicsMessageType;
use crate::physics::physics_handler::RenderMessage;

use super::plane::plane::PlaneControls;

/// Everything a scene might need out of a single frame, bundled so `Scene::tick`
/// keeps one stable signature no matter which of these fields a given scene actually
/// uses (a menu scene only cares about event_pump/controller, Playing only cares
/// about input_subsystem/plane_control_tx/physics_data).
pub struct FrameContext<'a> {
    pub app_state: &'a mut AppState,
    pub event_pump: &'a mut EventPump,
    pub controller: &'a mut Option<GameController>,
    pub input_subsystem: &'a InputSubsystem,
    pub plane_control_tx: &'a Sender<PlaneControls>,
    pub physics_data: &'a HashMap<String, RenderMessage>,
    pub debug_physics: &'a [DebugPhysicsMessageType],
}

/// A "screen" the game can be in (main menu, plane selection, playing, ...).
/// `AppState::state` holds the key of whichever entry of the `ScenePool` is active;
/// the app's update loop looks it up and drives it instead of hand-matching on state.
pub trait Scene {
    /// Runs once whenever this scene becomes the active one — on a state switch,
    /// or whenever AppState::reset is requested.
    fn reset(&mut self, app: &mut App);

    /// Runs every frame this scene is the active one.
    fn tick(&mut self, app: &mut App, ctx: &mut FrameContext);
}

/// All scenes the game knows about, keyed by the id stored in `AppState::state`.
/// Adding a new scene means implementing `Scene` for it and registering it here
/// (or wherever the pool is built) — no enum variant or match arm required.
pub type ScenePool = HashMap<String, Box<dyn Scene>>;
