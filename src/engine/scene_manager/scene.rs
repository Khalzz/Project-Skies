use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender};

use sdl2::EventPump;

use crate::app::{App, AppState};
use crate::engine::rendering::enviroment::environment::Environment;
use crate::game::play::plane::plane::PlaneControls;
use crate::engine::physics::physics::DebugPhysicsMessageType;
use crate::engine::physics::physics_handler::{PhysicsTick, RenderMessage};
use crate::resources::PreparedSceneAssets;


/// Everything a scene might need out of a single frame, bundled so `Scene::tick`
/// keeps one stable signature no matter which of these fields a given scene actually
/// uses (a menu scene only cares about event_pump, Playing only cares about
/// plane_control_tx/physics_data). Input state isn't here - query it directly via
/// crate::engine::input::input (a global singleton, which also owns controller
/// connect/disconnect - see that module).
pub struct FrameContext<'a> {
    pub app_state: &'a mut AppState,
    pub event_pump: &'a mut EventPump,
    // None whenever the active scene's Scene::physics() doesn't want physics.
    pub plane_control_tx: Option<&'a Sender<PlaneControls>>,
    pub physics_data: &'a HashMap<String, RenderMessage>,
    pub debug_physics: &'a [DebugPhysicsMessageType],
}

/// A "screen" the game can be in (main menu, plane selection, playing, ...).
/// `SceneManager::active` holds the name of whichever scene is active; `App::run`'s
/// loop drives it instead of hand-matching on state. There's no `init`/construction
/// hook here - a scene's real constructor (its `new`, registered via
/// `SceneManager::create_scene`) only ever runs when it's actually becoming active,
/// so there's nothing left for a separate init step to do.
pub trait Scene {
    fn update(&mut self, app: &mut App, ctx: &mut FrameContext);

    fn fixed_update(&self, app: &App) -> Option<(String, Box<dyn PhysicsTick + Send>)> {
        let _ = app;
        None
    }
}

/// What a heavy scene needs to go through the background-load path instead of the
/// plain synchronous one - see `SceneManager::create_loaded_scene`.
pub(crate) struct LoadedSceneSpec {
    pub(crate) level_path: String,
    pub(crate) environment: Environment,
    pub(crate) finish: Rc<dyn Fn(&mut App, PreparedSceneAssets) -> Box<dyn Scene>>,
}

/// A background asset load currently in flight - set when `App::run`'s reset
/// handling kicks off a heavy scene's load, cleared once its `receiver` yields,
/// `LoadingScreenScene` has faded back out, and `finish` has been run. See
/// `App::run`.
pub struct PendingSceneLoad {
    pub receiver: Receiver<PreparedSceneAssets>,
    pub finish: Rc<dyn Fn(&mut App, PreparedSceneAssets) -> Box<dyn Scene>>,
    /// Set once `receiver` has actually yielded a result - holds it (plus how many
    /// seconds we've spent fading LoadingScreenScene's UI back out) until the fade
    /// finishes, at which point App::run does the real clear+finish+swap. None
    /// means still waiting on the background thread.
    pub ready: Option<(PreparedSceneAssets, f32)>,
}

/// Owns every registered scene's constructor plus whichever one is currently active -
/// lives on `App` as `app.scene_manager`. Register cheap scenes with `create_scene`,
/// heavy ones (that load models/textures) with `create_loaded_scene`, then activate
/// one with `open_scene` (see main.rs) from anywhere holding `&mut App`.
pub struct SceneManager {
    // Rc, not Box: App::run needs to call a factory with &mut App, which means
    // cloning it out of this map first (same reason `active` below is read via
    // .clone()) - a Box can't be cheaply cloned, an Rc can.
    factories: HashMap<String, Rc<dyn Fn(&mut App) -> Box<dyn Scene>>>,
    // Heavy scenes registered via create_loaded_scene - disjoint from `factories`,
    // a given name is only ever in one of the two maps.
    loaders: HashMap<String, Rc<LoadedSceneSpec>>,
    // Only one scene is ever meaningfully alive at a time (inactive scenes never
    // tick), so this is a single slot rather than a map of every registered scene's
    // instance - those don't exist until create_scene's factory actually runs for
    // whichever one becomes active (see App::run's reset handling).
    pub active_scene: Option<Box<dyn Scene>>,
    pub active: String,
    pub reset: bool,
    // Some while a create_loaded_scene scene's background prepare is in flight -
    // App::run polls this every non-reset frame and swaps it into active_scene once
    // ready (see App::run). A LoadingScreenScene occupies active_scene in the
    // meantime so rendering keeps happening.
    pub loading: Option<PendingSceneLoad>,
}

impl SceneManager {
    pub fn new() -> Self {
        Self { factories: HashMap::new(), loaders: HashMap::new(), active_scene: None, active: String::new(), reset: false, loading: None }
    }

    /// Registers a scene's constructor under `name` - nothing runs yet, `factory`
    /// only gets called once this scene actually becomes active (see `open_scene`).
    /// Since a scene's own `new(app: &mut App) -> Self` already has exactly this
    /// shape, it can be passed directly: `create_scene("main_menu", main_menu::GameLogic::new)`.
    /// Overwrites whatever was previously registered under the same name. Use this
    /// only for scenes cheap enough to construct synchronously with no visible
    /// hitch (no model/texture loading) - see `create_loaded_scene` otherwise.
    pub fn create_scene<S: Scene + 'static>(&mut self, name: impl Into<String>, factory: impl Fn(&mut App) -> S + 'static) {
        self.factories.insert(name.into(), Rc::new(move |app: &mut App| Box::new(factory(app)) as Box<dyn Scene>));
    }

    /// Registers a scene that needs `resources::prepare_level_assets`/
    /// `prepare_environment`-style loading. Unlike `create_scene`, construction is
    /// split in two: `level_path`/`environment` get loaded on a background thread
    /// (see `App::run`'s reset handling), and only once that finishes does `finish`
    /// run on the main thread with the result - `finish` should do whatever the
    /// scene's old `new()` did after its own `load_level`/`apply_environment` calls
    /// (call `PreparedSceneAssets::apply` first thing to merge the loaded assets in).
    pub fn create_loaded_scene<S: Scene + 'static>(
        &mut self,
        name: impl Into<String>,
        level_path: impl Into<String>,
        environment: Environment,
        finish: impl Fn(&mut App, PreparedSceneAssets) -> S + 'static,
    ) {
        self.loaders.insert(name.into(), Rc::new(LoadedSceneSpec {
            level_path: level_path.into(),
            environment,
            finish: Rc::new(move |app: &mut App, assets: PreparedSceneAssets| Box::new(finish(app, assets)) as Box<dyn Scene>),
        }));
    }

    /// Switches the active scene and requests its `reset` for the next frame - use
    /// this instead of setting `active`/`reset` separately, since forgetting
    /// `reset = true` leaves the new scene running with the old one's leftover data.
    /// Logs an error and leaves the current scene active if `name` isn't registered,
    /// rather than switching to a scene that doesn't exist.
    pub fn open_scene(&mut self, name: &str) {
        if self.factories.contains_key(name) || self.loaders.contains_key(name) {
            self.active = name.to_owned();
            self.reset = true;
        } else {
            eprintln!("SceneManager::open_scene: no scene registered named '{}'", name);
        }
    }

    /// Clones out the factory registered for `name`, if any - used by App::run's
    /// reset handling, which needs to call it with `&mut App` while `self` (and thus
    /// this map) is itself reachable through that same `&mut App`.
    pub(crate) fn factory_for(&self, name: &str) -> Option<Rc<dyn Fn(&mut App) -> Box<dyn Scene>>> {
        self.factories.get(name).cloned()
    }

    /// Level path/environment/finish fn for a `create_loaded_scene`-registered
    /// scene, if any - see `factory_for`, same cloning rationale.
    pub(crate) fn loader_for(&self, name: &str) -> Option<Rc<LoadedSceneSpec>> {
        self.loaders.get(name).cloned()
    }
}
