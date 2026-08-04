use std::collections::HashMap;
use std::sync::mpsc::Sender;

use sdl2::{controller::GameController, EventPump};

use crate::app::{App, AppState};
use crate::game::play::plane::plane::PlaneControls;
use crate::engine::input::input::InputSubsystem;
use crate::engine::physics::physics::DebugPhysicsMessageType;
use crate::engine::physics::physics_handler::{PhysicsTick, RenderMessage};


/// Everything a scene might need out of a single frame, bundled so `Scene::tick`
/// keeps one stable signature no matter which of these fields a given scene actually
/// uses (a menu scene only cares about event_pump/controller, Playing only cares
/// about input_subsystem/plane_control_tx/physics_data).
pub struct FrameContext<'a> {
    pub app_state: &'a mut AppState,
    pub event_pump: &'a mut EventPump,
    pub controller: &'a mut Option<GameController>,
    pub input_subsystem: &'a InputSubsystem,
    // None whenever the active scene's Scene::physics() doesn't want physics.
    pub plane_control_tx: Option<&'a Sender<PlaneControls>>,
    pub physics_data: &'a HashMap<String, RenderMessage>,
    pub debug_physics: &'a [DebugPhysicsMessageType],
}

/// Identifies a "screen" the game can be in. `AppState::state` holds whichever
/// variant is active; the `ScenePool` is keyed by it. Adding a new scene means
/// adding a variant here, implementing `Scene` for it, and registering it wherever
/// the pool is built (see main.rs).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GameState {
    Playing,
    SelectingPlane,
}

/// A "screen" the game can be in (main menu, plane selection, playing, ...).
/// `SceneManager::active` holds the key of whichever entry of the `ScenePool` is
/// active; `App::run`'s loop looks it up and drives it instead of hand-matching on state.
pub trait Scene {
    /// Runs once whenever this scene becomes the active one — on a state switch,
    /// or whenever SceneManager::reset is requested.
    fn reset(&mut self, app: &mut App);

    /// Runs every frame this scene is the active one.
    fn tick(&mut self, app: &mut App, ctx: &mut FrameContext);

    /// Level to load and the per-tick physics update to run against it while this
    /// scene is active, or None (the default) if this scene doesn't use physics -
    /// App::run() starts/stops the physics thread automatically based on this, once
    /// per scene switch, right after calling reset(). Nothing to override unless a
    /// scene actually wants physics. Takes `app` so the level path can be read back
    /// from whatever reset() (e.g. resources::load_level) already loaded, instead of
    /// needing its own separately-maintained copy of the path.
    fn physics(&self, app: &App) -> Option<(String, Box<dyn PhysicsTick + Send>)> {
        let _ = app;
        None
    }
}

/// All scenes the game knows about, keyed by `GameState`. Adding a new scene means
/// implementing `Scene` for it and registering it here (or wherever the pool is
/// built) — no match arm elsewhere required.
pub type ScenePool = HashMap<GameState, Box<dyn Scene>>;

/// Owns the registered scenes and tracks which one is active - lives on `App` as
/// `app.scene_manager`. Configure it once with the pool and a starting scene (see
/// main.rs), then drive scene switches through `switch_to` from anywhere holding `&mut App`.
pub struct SceneManager {
    pub scenes: ScenePool,
    pub active: GameState,
    pub reset: bool,
}

impl SceneManager {
    pub fn new(scenes: ScenePool, starting_scene: GameState) -> Self {
        Self { scenes, active: starting_scene, reset: true }
    }

    /// Switches the active scene and requests its `reset` for the next frame -
    /// use this instead of setting `active`/`reset` separately, since forgetting
    /// `reset = true` leaves the new scene running with the old one's leftover data.
    pub fn switch_to(&mut self, state: GameState) {
        self.active = state;
        self.reset = true;
    }
}
