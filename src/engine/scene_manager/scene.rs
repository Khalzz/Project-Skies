use std::any::Any;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

use sdl2::EventPump;

use crate::app::{App, AppState};
use crate::engine::game_nodes::game_object::Physics;
use crate::engine::rendering::camera::handler::{CameraInstance, LookAtTarget, SceneCameras};
use crate::engine::rendering::enviroment::environment::SceneEnvironment;
use crate::engine::rendering::instance_management::InstanceData;
use crate::game::scenes::play::plane::controls::PlaneControls;
use crate::engine::physics::physics::DebugPhysicsMessageType;
use crate::engine::physics::physics_handler::{PhysicsCommand, PhysicsTick, RenderMessage};
use crate::engine::physics::physics_resources::PhysicsObjectDef;
use crate::engine::scene_manager::node::Node;
use crate::engine::scene_manager::physics_bridge;
use crate::engine::scene_manager::properties::Model as NodeModelProperty;
use crate::engine::scene_manager::render_bridge;
use crate::engine::scene_manager::scene_nodes::SceneNodes;
use crate::transform::Transform;


/// Everything a scene might need out of a single frame, bundled so `SceneBehaviour::update`
/// keeps one stable signature no matter which of these fields a given scene actually
/// uses (a menu scene only cares about event_pump, Playing only cares about
/// plane_control_tx/physics_data). Input state isn't here - query it directly via
/// crate::engine::input::input (a global singleton, which also owns controller
/// connect/disconnect - see that module).
pub struct FrameContext<'a> {
    pub app_state: &'a mut AppState,
    pub event_pump: &'a mut EventPump,
    // None whenever the active scene's SceneBehaviour::fixed_update doesn't want physics.
    pub plane_control_tx: Option<&'a Sender<PlaneControls>>,
    // Same None-ness as plane_control_tx - lets a scene pause/resume/teleport a
    // physics-driven object (see PhysicsCommand::SetTransform) for a scripted
    // cinematic, e.g. CameraTrack::LookAt.
    pub physics_command_tx: Option<&'a Sender<PhysicsCommand>>,
    pub physics_data: &'a HashMap<String, RenderMessage>,
    pub debug_physics: &'a [DebugPhysicsMessageType],
}

/// Everything that belongs to a given scene - nodes (the code-first entity
/// system) and the renderable instances that scene put up, whether from
/// `data.ron` or spawned via `Scene::spawn_node`. Cleared as one unit on
/// every scene switch (see `App::run`'s reset handling), rather than each
/// piece needing its own separately-remembered clear call - a new
/// scene-scoped system, added later, gets this for free just by becoming a
/// field here, instead of needing a new line added to the reset block by
/// hand (which is exactly how `renderizable_instances` ended up never being
/// cleared for `create_scene`-registered scenes until this was noticed - see
/// the design discussion this came out of).
#[derive(Default)]
pub struct SceneContent {
    pub nodes: SceneNodes,
    pub renderizable_instances: HashMap<String, InstanceData>,
    // Every node spawned with a Physics property (see
    // engine::scene_manager::physics_bridge), collected as plain, Send-safe
    // data - not a live reference back to the node itself, see
    // PhysicsObjectDef's own doc comment for why. Read once by
    // SceneBehaviour::fixed_update to seed the physics thread.
    pub physics_bodies: Vec<PhysicsObjectDef>,
}

impl SceneContent {
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.renderizable_instances.clear();
        self.physics_bodies.clear();
    }
}

/// The custom logic driving one `Scene` - what used to be a scene's own
/// `impl Scene for GameLogic` (e.g. `main_menu::scene::GameLogic`,
/// `play::scene::GameLogic`). Mirrors `Behavior` (see that trait's own doc
/// comment) - `on_spawn`/`update`/`fixed_update` all get `&mut Scene` instead
/// of `&mut Node`, so a scene's logic can reach its own `content` (spawn
/// nodes, read/write renderable instances) the same way a node's behavior
/// reaches its own properties.
pub trait SceneBehaviour {
    fn on_spawn(&mut self, scene: &mut Scene, app: &mut App) { let _ = (scene, app); }
    fn update(&mut self, scene: &mut Scene, app: &mut App, ctx: &mut FrameContext) { let _ = (scene, app, ctx); }

    // Not a per-frame tick despite the name - called exactly once, right
    // after this scene becomes active (see App::run's reset handling), to
    // optionally register this scene's own PhysicsTick with the physics
    // thread, seeded from whatever `PhysicsObjectDef`s spawning already
    // collected on `scene.content.physics_bodies` (see
    // `engine::scene_manager::physics_bridge`). Keeps the
    // misleading-but-established name from the old `Scene` trait this was
    // lifted from, rather than renaming it in the same pass as this
    // refactor.
    fn fixed_update(&self, scene: &Scene, app: &App) -> Option<(Vec<PhysicsObjectDef>, Box<dyn PhysicsTick + Send>)> {
        let _ = (scene, app);
        None
    }
}

/// A "screen" the game can be in (main menu, plane selection, playing, ...) -
/// the generic container every scene shares: whatever nodes/renderable
/// instances it's spawned (`content`), whatever cameras it's registered
/// (`cameras`), whatever skybox/clear color it's applied (`environment`),
/// plus the `SceneBehaviour` actually driving it. `SceneManager::active_scene`
/// holds whichever one is currently active; `App::run`'s loop drives it
/// instead of hand-matching on state.
pub struct Scene {
    pub content: SceneContent,
    pub cameras: SceneCameras,
    pub environment: SceneEnvironment,
    behaviour: Option<Box<dyn SceneBehaviour>>,
}

impl Scene {
    // Takes &App (not width/height directly) so a fresh scene's cameras
    // start at the real current screen size (see CameraResources::width/
    // height) without every caller needing to know where that comes from.
    pub fn new(app: &App) -> Self {
        Self {
            content: SceneContent::default(),
            cameras: SceneCameras::new(app.camera_resources.width(), app.camera_resources.height()),
            environment: SceneEnvironment::default(),
            behaviour: None,
        }
    }

    pub fn set_scene_behaviour(mut self, behaviour: impl SceneBehaviour + 'static) -> Self {
        self.behaviour = Some(Box::new(behaviour));
        self
    }

    /// Spawns `node` into this scene's own content and, if it carries a
    /// `Model` property, immediately registers it with the renderer too (see
    /// `engine::scene_manager::render_bridge::register_static_model`); same
    /// deal if it carries a `Physics` property (see
    /// `engine::scene_manager::physics_bridge::register_physics_body`) - in
    /// both cases the property itself is what triggers instantiation, not a
    /// second call the spawner has to remember to make separately.
    /// `Node`/`SceneNodes` stay unaware of rendering/physics either way -
    /// this just composes them at the one place that actually has both
    /// `&mut Scene` and `&mut App` available.
    pub fn spawn_node(&mut self, app: &mut App, node: Node) -> Result<(), String> {
        let id = node.id.clone();
        let has_model = node.get_property::<NodeModelProperty>().is_some();
        let has_physics = node.get_property::<Physics>().is_some();

        self.content.nodes.spawn(node, &mut self.cameras, app)?;

        if has_model {
            render_bridge::register_static_model(self, app, &id)?;
        }

        if has_physics {
            physics_bridge::register_physics_body(self, &id)?;
        }

        Ok(())
    }

    /// Creates (or replaces) a named camera directly on this scene, with no
    /// node/behaviour needed - the plain path for a camera that just sits at
    /// a fixed pose (see `SceneCameras::create_camera`). If a camera needs
    /// its own per-frame logic (e.g. a free-fly controller), spawn a node
    /// with a `Behavior` instead - `Behavior`'s hooks get a `&mut
    /// SceneCameras` of their own to drive this same list from, so both
    /// paths write into exactly one place.
    pub fn create_camera(&mut self, name: impl Into<String>, transform: Transform, fovy: f32) -> &mut CameraInstance {
        self.cameras.create_camera(name, transform, fovy)
    }

    pub fn select_camera(&mut self, name: &str) -> bool {
        self.cameras.select_camera(name)
    }

    /// Sets (or clears) `name`'s `look_at` - see `LookAtTarget`/
    /// `CameraInstance::look_at`'s own doc comments. Re-applied every frame
    /// by `App::run` (via `SceneCameras::update`), so `LookAtTarget::Node`
    /// keeps tracking a moving target rather than aiming at wherever it was
    /// the instant this was called.
    pub fn set_camera_look_at(&mut self, name: &str, target: Option<LookAtTarget>) -> bool {
        self.cameras.set_look_at(name, target)
    }

    // Take-then-restore, same reasoning as SceneManager::factory_for/
    // App::run's own active_scene dance: `behaviour` can't be called with
    // `&mut self` while it's still sitting borrowed inside `self.behaviour`.
    pub(crate) fn run_on_spawn(&mut self, app: &mut App) {
        if let Some(mut behaviour) = self.behaviour.take() {
            behaviour.on_spawn(self, app);
            self.behaviour = Some(behaviour);
        }
    }

    pub(crate) fn run_update(&mut self, app: &mut App, ctx: &mut FrameContext) {
        if let Some(mut behaviour) = self.behaviour.take() {
            behaviour.update(self, app, ctx);
            self.behaviour = Some(behaviour);
        }
    }

    pub(crate) fn run_fixed_update(&self, app: &App) -> Option<(Vec<PhysicsObjectDef>, Box<dyn PhysicsTick + Send>)> {
        self.behaviour.as_ref().and_then(|behaviour| behaviour.fixed_update(self, app))
    }
}

/// What a heavy scene needs to go through the background-load path instead of the
/// plain synchronous one - see `SceneManager::create_loaded_scene`. `prepare` has
/// to be `Arc`, not `Rc` like every other stored closure here: it's the one that
/// actually gets moved into the background `std::thread::spawn` (see `App::run`'s
/// reset handling), so it has to be `Send` itself, not just its output.
pub(crate) struct LoadedSceneSpec {
    pub(crate) prepare: Arc<dyn Fn(&wgpu::Device, &wgpu::Queue, &wgpu::BindGroupLayout, &wgpu::SurfaceConfiguration) -> Box<dyn Any + Send> + Send + Sync>,
    pub(crate) finish: Rc<dyn Fn(&mut App, Box<dyn Any + Send>) -> Scene>,
}

/// A background asset load currently in flight - set when `App::run`'s reset
/// handling kicks off a heavy scene's load, cleared once its `receiver` yields,
/// `LoadingScreenScene` has faded back out, and `finish` has been run. See
/// `App::run`.
pub struct PendingSceneLoad {
    pub receiver: Receiver<Box<dyn Any + Send>>,
    pub finish: Rc<dyn Fn(&mut App, Box<dyn Any + Send>) -> Scene>,
    /// Set once `receiver` has actually yielded a result - holds it (plus how many
    /// seconds we've spent fading LoadingScreenScene's UI back out) until the fade
    /// finishes, at which point App::run does the real clear+finish+swap. None
    /// means still waiting on the background thread.
    pub ready: Option<(Box<dyn Any + Send>, f32)>,
}

/// Owns every registered scene's constructor plus whichever one is currently active -
/// lives on `App` as `app.scene_manager`. Register cheap scenes with `create_scene`,
/// heavy ones (that load models/textures) with `create_loaded_scene`, then activate
/// one with `open_scene` (see main.rs) from anywhere holding `&mut App`.
pub struct SceneManager {
    // Rc, not Box: App::run needs to call a factory with &mut App, which means
    // cloning it out of this map first (same reason `active` below is read via
    // .clone()) - a Box can't be cheaply cloned, an Rc can.
    factories: HashMap<String, Rc<dyn Fn(&mut App) -> Scene>>,
    // Heavy scenes registered via create_loaded_scene - disjoint from `factories`,
    // a given name is only ever in one of the two maps.
    loaders: HashMap<String, Rc<LoadedSceneSpec>>,
    // Only one scene is ever meaningfully alive at a time (inactive scenes never
    // tick), so this is a single slot rather than a map of every registered scene's
    // instance - those don't exist until create_scene's factory actually runs for
    // whichever one becomes active (see App::run's reset handling).
    pub active_scene: Option<Scene>,
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
    /// `factory` gets a fresh, empty `Scene` to spawn into (`scene.spawn_node`,
    /// `scene.content` directly) alongside `&mut App`, and returns just the
    /// `SceneBehaviour` driving it - wrapping that behaviour into the `Scene` it
    /// was just handed (`Scene::set_scene_behaviour`) happens here, not in the
    /// scene's own constructor, so a scene's `new` never has to repeat that
    /// boilerplate. Since a scene's own `new(scene: &mut Scene, app: &mut App) ->
    /// Self` already has exactly this shape, it can be passed directly:
    /// `create_scene("main_menu", main_menu::GameLogic::new)`.
    /// Overwrites whatever was previously registered under the same name. Use this
    /// only for scenes cheap enough to construct synchronously with no visible
    /// hitch (no model/texture loading) - see `create_loaded_scene` otherwise.
    pub fn create_scene<B: SceneBehaviour + 'static>(&mut self, name: impl Into<String>, factory: impl Fn(&mut Scene, &mut App) -> B + 'static) {
        self.factories.insert(name.into(), Rc::new(move |app: &mut App| {
            let mut scene = Scene::new(app);
            let behaviour = factory(&mut scene, app);
            scene.set_scene_behaviour(behaviour)
        }));
    }

    /// Registers a scene whose construction needs real disk/GPU work first (skybox
    /// textures, glTF geometry, ...) - unlike `create_scene`, construction is split
    /// in two: `prepare` runs on a background thread (see `App::run`'s reset
    /// handling; `device`/`queue`/`camera_bind_group_layout`/`config` are all cheap,
    /// `Arc`-backed clones, so building real GPU resources there is safe), and only
    /// once that finishes does `finish` run on the main thread with whatever
    /// `prepare` produced - same `(&mut Scene, &mut App) -> B: SceneBehaviour` shape
    /// as `create_scene`'s `factory`, plus the prepared value.
    ///
    /// `P` crosses the thread boundary as a type-erased `Box<dyn Any + Send>`
    /// internally - `SceneManager` can't be generic over every registered scene's
    /// own prepared-asset type at once, since they're all stored in the same
    /// `loaders` map. Downcast back happens automatically inside `finish` below;
    /// callers of `create_loaded_scene`/the scene's own `prepare`/`finish` never see
    /// the erasure themselves.
    pub fn create_loaded_scene<P: Send + 'static, B: SceneBehaviour + 'static>(
        &mut self,
        name: impl Into<String>,
        prepare: impl Fn(&wgpu::Device, &wgpu::Queue, &wgpu::BindGroupLayout, &wgpu::SurfaceConfiguration) -> P + Send + Sync + 'static,
        finish: impl Fn(&mut Scene, &mut App, P) -> B + 'static,
    ) {
        self.loaders.insert(name.into(), Rc::new(LoadedSceneSpec {
            prepare: Arc::new(move |device, queue, layout, config| Box::new(prepare(device, queue, layout, config)) as Box<dyn Any + Send>),
            finish: Rc::new(move |app: &mut App, prepared: Box<dyn Any + Send>| {
                let prepared = *prepared.downcast::<P>().expect("SceneManager: prepared asset type mismatch for a create_loaded_scene scene");
                let mut scene = Scene::new(app);
                let behaviour = finish(&mut scene, app, prepared);
                scene.set_scene_behaviour(behaviour)
            }),
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
    pub(crate) fn factory_for(&self, name: &str) -> Option<Rc<dyn Fn(&mut App) -> Scene>> {
        self.factories.get(name).cloned()
    }

    /// Level path/environment/finish fn for a `create_loaded_scene`-registered
    /// scene, if any - see `factory_for`, same cloning rationale.
    pub(crate) fn loader_for(&self, name: &str) -> Option<Rc<LoadedSceneSpec>> {
        self.loaders.get(name).cloned()
    }

    /// This scene manager's currently active scene's content, if any - `None`
    /// only very briefly at startup, before `App::run`'s reset handling has
    /// constructed the first scene. Render-time code (instance buffer writes,
    /// lighting) reads through this instead of a shared field on
    /// `SceneManager` itself, since content now lives on whichever `Scene` is
    /// actually active.
    pub fn content(&self) -> Option<&SceneContent> {
        self.active_scene.as_ref().map(|scene| &scene.content)
    }

    pub fn content_mut(&mut self) -> Option<&mut SceneContent> {
        self.active_scene.as_mut().map(|scene| &mut scene.content)
    }

    /// Same reasoning as `content`/`content_mut` - cameras live on whichever
    /// `Scene` is active now, not on `SceneManager` itself.
    pub fn cameras(&self) -> Option<&SceneCameras> {
        self.active_scene.as_ref().map(|scene| &scene.cameras)
    }

    pub fn cameras_mut(&mut self) -> Option<&mut SceneCameras> {
        self.active_scene.as_mut().map(|scene| &mut scene.cameras)
    }

    /// Same reasoning as `content`/`cameras` - the applied skybox/clear color
    /// live on whichever `Scene` is active now, not on `SceneManager` itself.
    /// Render-time code falls back to `App`'s own `skybox`/`clear_color`
    /// fields when this is `None` - that fallback only ever applies before
    /// the first scene has reset (the splash screen phase, which sets those
    /// App-level fields directly since no `Scene` exists yet to hold them).
    pub fn environment(&self) -> Option<&SceneEnvironment> {
        self.active_scene.as_ref().map(|scene| &scene.environment)
    }

    pub fn environment_mut(&mut self) -> Option<&mut SceneEnvironment> {
        self.active_scene.as_mut().map(|scene| &mut scene.environment)
    }
}
