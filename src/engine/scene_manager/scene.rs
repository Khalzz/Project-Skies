use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

use sdl2::EventPump;

use crate::app::{App, AppState};
use crate::engine::game_nodes::game_object::Physics;
use crate::engine::rendering::camera::handler::{CameraInstance, LookAtTarget, SceneCameras};
use crate::engine::rendering::enviroment::environment::SceneEnvironment;
use crate::engine::rendering::instance_management::InstanceData;
use crate::engine::physics::physics::{physics_handling, DebugPhysicsMessageType, PhysicsDataTransmission};
use crate::engine::physics::physics_handler::{PhysicsCommand, RenderMessage};
use crate::engine::physics::physics_resources::PhysicsObjectDef;
use crate::engine::scene_manager::node::Node;
use crate::engine::scene_manager::physics_bridge;
use crate::engine::scene_manager::properties::{CameraLens, Material, Model as NodeModelProperty, Parent, Space, SpawnedLive, Transform3D};
use crate::engine::scene_manager::render_bridge;
use crate::engine::scene_manager::scene_nodes::SceneNodes;
use crate::engine::rendering::models::model::Model as LoadedModel;
use crate::resources::ModelSource;
use crate::transform::Transform;


/// Everything a scene might need out of a single frame, bundled so `SceneBehaviour::update`
/// keeps one stable signature no matter which of these fields a given scene actually
/// uses (a menu scene only cares about event_pump, Playing only cares about
/// physics_command_tx/physics_data). Input state isn't here - query it directly via
/// crate::engine::input::input (a global singleton, which also owns controller
/// connect/disconnect - see that module).
pub struct FrameContext<'a> {
    pub app_state: &'a mut AppState,
    pub event_pump: &'a mut EventPump,
    // None whenever the active scene has no physics running (see
    // Scene::start_physics) - lets a scene pause/resume/teleport a
    // physics-driven object (see PhysicsCommand::SetTransform) - an animation
    // moving it, for one.
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
    // Scene::start_physics to seed the physics thread.
    pub physics_bodies: Vec<PhysicsObjectDef>,
    // Nodes something else is moving right now (an animation) - the
    // physics' pose isn't written onto them meanwhile (see App::run). Whoever
    // puts a node in here also holds its body still (PhysicsCommand::
    // SetKinematic) and lets it go again.
    pub kinematic: HashSet<String>,
}

impl SceneContent {
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.renderizable_instances.clear();
        self.physics_bodies.clear();
        self.kinematic.clear();
    }
}

/// The custom logic driving one `Scene` - what used to be a scene's own
/// `impl Scene for GameLogic` (e.g. `main_menu::scene::GameLogic`,
/// `play::scene::GameLogic`). Mirrors `Behavior` (see that trait's own doc
/// comment) - `on_spawn`/`update` both get `&mut Scene` instead
/// of `&mut Node`, so a scene's logic can reach its own `content` (spawn
/// nodes, read/write renderable instances) the same way a node's behavior
/// reaches its own properties.
pub trait SceneBehaviour {
    fn on_spawn(&mut self, scene: &mut Scene, app: &mut App) { let _ = (scene, app); }
    fn update(&mut self, scene: &mut Scene, app: &mut App, ctx: &mut FrameContext) { let _ = (scene, app, ctx); }
    /// After every node's behaviors have run this frame, before the
    /// hierarchy is resolved and the cameras placed - where an animation
    /// goes, so it has the last word on what it moves.
    fn late_update(&mut self, scene: &mut Scene, app: &mut App, ctx: &mut FrameContext) { let _ = (scene, app, ctx); }
    /// After this frame's cameras and renderable transforms are the ones
    /// that will be drawn. The collimated HUD is placed here so it shares
    /// that pose with the combiner glass.
    fn after_cameras(&mut self, scene: &mut Scene, app: &mut App) { let _ = (scene, app); }
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
    // Set once the scene's been built (its behaviour's on_spawn has run) -
    // nodes spawned from then on are marked SpawnedLive.
    started: bool,
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
            started: false,
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
    /// `engine::scene_manager::physics_bridge::register_physics_body`), or a
    /// `CameraLens` (a camera under its id, see `sync_cameras`) - in every
    /// case the property itself is what triggers instantiation, not a
    /// second call the spawner has to remember to make separately.
    /// Children given with `Node::with_child` are spawned right after it,
    /// each with its `Parent`.
    /// `Node`/`SceneNodes` stay unaware of rendering/physics either way -
    /// this just composes them at the one place that actually has both
    /// `&mut Scene` and `&mut App` available.
    pub fn spawn_node(&mut self, app: &mut App, node: Node) -> Result<(), String> {
        self.spawn_tree(app, node, None)
    }

    /// `spawn_node` for a scene whose physics thread is already running (see
    /// `start_physics` - a plain `spawn_node` then gets no body): the node's
    /// body and physics halves (its children's too) are sent to the running
    /// thread through `physics_command_tx`. Without one (no physics running),
    /// the same as `spawn_node`.
    pub fn spawn_node_live(&mut self, app: &mut App, node: Node, physics_command_tx: Option<&Sender<PhysicsCommand>>) -> Result<(), String> {
        self.spawn_tree(app, node, physics_command_tx)
    }

    fn spawn_tree(&mut self, app: &mut App, mut node: Node, physics_command_tx: Option<&Sender<PhysicsCommand>>) -> Result<(), String> {
        let children = node.take_children();
        let id = node.id.clone();
        self.spawn_one(app, node, physics_command_tx)?;
        for mut child in children {
            let (space, inherit_rotation) = child.placement();
            let given = child.get_property::<Transform3D>().copied().unwrap_or_default();
            let local = match space {
                Space::Local => given,
                Space::Global => Transform3D::default(),
            };
            child.set_property(Parent { id: id.clone(), space, local, inherit_rotation });
            self.spawn_tree(app, child, physics_command_tx)?;
        }
        Ok(())
    }

    fn spawn_one(&mut self, app: &mut App, mut node: Node, physics_command_tx: Option<&Sender<PhysicsCommand>>) -> Result<(), String> {
        let id = node.id.clone();
        // A local child starts where its parent is now - its renderer
        // instance, body and camera are all made from this.
        if let Some(parent) = node.get_property::<Parent>().filter(|parent| parent.space == Space::Local).cloned() {
            if let Some(parent_world) = self.content.nodes.world_transform(&parent.id) {
                node.set_property(parent_world.compose(&parent.local, parent.inherit_rotation));
            }
        }
        if self.started {
            node.set_property(SpawnedLive);
        }
        let has_model = node.get_property::<NodeModelProperty>().is_some();
        let has_physics = node.get_property::<Physics>().is_some();
        let lens = node.get_property::<CameraLens>().copied();
        let transform = node.get_property::<Transform3D>().copied().unwrap_or_default();

        self.content.nodes.spawn(node, &mut self.cameras, app)?;

        if has_model {
            render_bridge::register_static_model(self, app, &id)?;
        }

        if has_physics {
            physics_bridge::register_physics_body(self, &id)?;
        }

        if let Some(lens) = lens {
            self.cameras.place(&id, transform.position, transform.rotation, lens.fovy, lens.znear, lens.viewport);
        }

        let Some(tx) = physics_command_tx else { return Ok(()) };
        let def = self.content.physics_bodies.iter().rposition(|def| def.id == id)
            .map(|index| self.content.physics_bodies.remove(index));
        let behaviors = self.content.nodes.get_mut(&id).map(|node| node.take_physics_behaviors()).unwrap_or_default();
        if let Some(def) = def {
            tx.send(PhysicsCommand::AddBody { def, behaviors })
                .map_err(|error| format!("node '{id}': couldn't reach the physics thread: {error}"))?;
        }
        Ok(())
    }

    /// Removes the node `id` and its children (and theirs) - their
    /// behaviors, their instances on the renderer (see
    /// `render_bridge::unregister_model`), their cameras and, with physics
    /// running, their bodies (sent through `physics_command_tx`). Nothing
    /// happens for an id with no node. Like `SceneNodes::clear`, anything
    /// their behaviors did outside their own node isn't undone.
    pub fn despawn_node(&mut self, app: &mut App, id: &str, physics_command_tx: Option<&Sender<PhysicsCommand>>) {
        if self.content.nodes.get(id).is_none() {
            return;
        }
        let mut ids = vec![id.to_owned()];
        ids.extend(self.content.nodes.descendants_of(id));
        for id in ids {
            if self.content.nodes.remove(&id).is_none() {
                continue;
            }
            render_bridge::unregister_model(self, app, &id);
            self.cameras.remove_camera(&id);
            self.content.kinematic.remove(&id);
            self.content.physics_bodies.retain(|def| def.id != id);
            if let Some(tx) = physics_command_tx {
                let _ = tx.send(PhysicsCommand::RemoveBody { name: id.clone() });
            }
        }
    }

    /// Where the node `id` is in the world (see `SceneNodes::world_transform`).
    pub fn world_transform(&self, id: &str) -> Option<Transform3D> {
        self.content.nodes.world_transform(id)
    }

    /// Moves the node `id` to `world` - for a `Local` child, by changing its
    /// offset from the parent so it lands there; for anything else, its own
    /// transform. Shown right away (its renderer instance too), not just
    /// once the frame's syncs run.
    pub fn set_world_transform(&mut self, id: &str, world: Transform3D) {
        let parent = self.content.nodes.get(id).and_then(|node| node.get_property::<Parent>()).cloned();
        if let Some(mut parent) = parent.filter(|parent| parent.space == Space::Local) {
            if let Some(parent_world) = self.content.nodes.world_transform(&parent.id) {
                parent.local = parent_world.relative(&world, parent.inherit_rotation);
                if let Some(node) = self.content.nodes.get_mut(id) {
                    node.set_property(parent);
                }
            }
        }
        let Some(node) = self.content.nodes.get_mut(id) else { return };
        node.set_property(world);
        let render_scale = node.get_property::<NodeModelProperty>().map(|model| model.render_scale(world.scale)).unwrap_or(world.scale);
        if let Some(instance) = self.content.renderizable_instances.get_mut(id) {
            instance.instance.transform.position = world.position;
            instance.instance.transform.rotation = world.rotation;
            instance.instance.transform.scale = render_scale;
        }
    }

    /// Re-parents the node `id` - under `parent` (in `space`, turning with
    /// it or not), or out to the top with `None` - keeping it exactly where
    /// it is in the world, so it doesn't jump: a `Local` child's offset is
    /// worked out from where it is now. False (nothing changed) for a
    /// missing node or a parent that's it or one of its own children.
    pub fn set_parent(&mut self, id: &str, parent: Option<(&str, Space, bool)>) -> bool {
        let Some(world) = self.content.nodes.world_transform(id) else { return false };
        let new_parent = match parent {
            None => None,
            Some((parent_id, space, inherit_rotation)) => {
                if parent_id == id || self.content.nodes.descendants_of(id).iter().any(|child| child == parent_id) {
                    return false;
                }
                let Some(parent_world) = self.content.nodes.world_transform(parent_id) else { return false };
                let local = match space {
                    Space::Local => parent_world.relative(&world, inherit_rotation),
                    Space::Global => Transform3D::default(),
                };
                Some(Parent { id: parent_id.to_owned(), space, local, inherit_rotation })
            }
        };
        let Some(node) = self.content.nodes.get_mut(id) else { return false };
        match new_parent {
            Some(parent) => node.set_property(parent),
            None => { node.remove_property::<Parent>(); }
        }
        true
    }

    /// Every node's renderer instance to where the node is - position,
    /// rotation, and scale (the node's times its model's, see
    /// `Model::render_scale`) - and to its `Material`. Called every frame by
    /// `App::run` after the hierarchy's resolved, so a node's `Transform3D`
    /// is the one place its pose is set from.
    pub fn sync_render_instances(&mut self) {
        let SceneContent { nodes, renderizable_instances, .. } = &mut self.content;
        for node in nodes.iter() {
            let Some(transform) = node.get_property::<Transform3D>() else { continue };
            let Some(instance) = renderizable_instances.get_mut(&node.id) else { continue };
            let render_scale = node.get_property::<NodeModelProperty>().map(|model| model.render_scale(transform.scale)).unwrap_or(transform.scale);
            instance.instance.transform.position = transform.position;
            instance.instance.transform.rotation = transform.rotation;
            instance.instance.transform.scale = render_scale;
            let material = node.get_property::<Material>().map_or(crate::engine::rendering::shader_library::STANDARD, |material| material.shader.as_str());
            if instance.material != material {
                instance.material = material.to_owned();
            }
        }
    }

    /// Every camera node's camera (see `CameraLens`) to where its node is
    /// and to its lens - called every frame by `App::run`, after the
    /// hierarchy's resolved.
    pub fn sync_cameras(&mut self) {
        for node in self.content.nodes.iter() {
            let Some(lens) = node.get_property::<CameraLens>() else { continue };
            let transform = node.get_property::<Transform3D>().copied().unwrap_or_default();
            self.cameras.place(&node.id, transform.position, transform.rotation, lens.fovy, lens.znear, lens.viewport);
        }
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
        self.started = true;
    }

    pub(crate) fn run_update(&mut self, app: &mut App, ctx: &mut FrameContext) {
        if let Some(mut behaviour) = self.behaviour.take() {
            behaviour.update(self, app, ctx);
            self.behaviour = Some(behaviour);
        }
    }

    pub(crate) fn run_late_update(&mut self, app: &mut App, ctx: &mut FrameContext) {
        if let Some(mut behaviour) = self.behaviour.take() {
            behaviour.late_update(self, app, ctx);
            self.behaviour = Some(behaviour);
        }
    }

    pub(crate) fn run_after_cameras(&mut self, app: &mut App) {
        if let Some(mut behaviour) = self.behaviour.take() {
            behaviour.after_cameras(self, app);
            self.behaviour = Some(behaviour);
        }
    }

    /// Starts this scene's physics thread, if it has anything to simulate -
    /// any node spawned with a `Physics` property (see `physics_bridge`) or a
    /// `PhysicsBehavior` (see `Node::add_physics_behavior`). Called once,
    /// right after the scene becomes active: rigidbodies and node physics
    /// halves are both moved onto the new thread here, so a node spawned
    /// later gets neither.
    pub(crate) fn start_physics(&mut self, app: &App) -> Option<PhysicsDataTransmission> {
        let node_behaviors = self.content.nodes.take_physics_behaviors();
        if self.content.physics_bodies.is_empty() && node_behaviors.is_empty() {
            return None;
        }

        Some(physics_handling(&app.renderer.device, &app.renderer.config, &app.camera_resources, self.content.physics_bodies.clone(), node_behaviors))
    }
}

/// What a heavy scene needs to go through the background-load path instead of the
/// plain synchronous one - see `SceneManager::create_loaded_scene`. `prepare` has
/// to be `Arc`, not `Rc` like every other stored closure here: it's the one that
/// actually gets moved into the background `std::thread::spawn` (see `App::run`'s
/// reset handling), so it has to be `Send` itself, not just its output.
pub(crate) struct LoadedSceneSpec {
    pub(crate) prepare: Arc<dyn Fn(&wgpu::Device, &wgpu::Queue, &wgpu::BindGroupLayout, &wgpu::SurfaceConfiguration, &LoadReporter) -> Box<dyn Any + Send> + Send + Sync>,
    pub(crate) finish: Rc<dyn Fn(&mut App, Box<dyn Any + Send>) -> Scene>,
}

/// A background asset load currently in flight - set when `App::run`'s reset
/// handling kicks off a heavy scene's load, cleared once its `receiver` yields,
/// `LoadingScreenScene` has faded back out, and `finish` has been run. See
/// `App::run`.
/// Handed to a heavy scene's background load (its `prepare`, then its
/// models): each `step` names what's being loaded now - shown on the
/// loading screen - and times the one before it. `finish` logs every step's
/// time, slowest first, to find what makes loading slow.
pub struct LoadReporter {
    scene: String,
    progress: std::sync::mpsc::Sender<String>,
    started: std::time::Instant,
    current: std::cell::RefCell<Option<(String, std::time::Instant)>>,
    timings: std::cell::RefCell<Vec<(String, std::time::Duration)>>,
}

impl LoadReporter {
    pub(crate) fn new(scene: &str, progress: std::sync::mpsc::Sender<String>) -> Self {
        Self {
            scene: scene.to_owned(),
            progress,
            started: std::time::Instant::now(),
            current: std::cell::RefCell::new(None),
            timings: std::cell::RefCell::new(Vec::new()),
        }
    }

    /// Now loading `what` - the previous step, if any, is done.
    pub fn step(&self, what: impl Into<String>) {
        self.end_current();
        let what = what.into();
        let _ = self.progress.send(what.clone());
        *self.current.borrow_mut() = Some((what, std::time::Instant::now()));
    }

    fn end_current(&self) {
        if let Some((what, started)) = self.current.borrow_mut().take() {
            let took = started.elapsed();
            println!("[load {}] {what}: {:.0} ms", self.scene, took.as_secs_f64() * 1000.0);
            self.timings.borrow_mut().push((what, took));
        }
    }

    /// The load's done - logs the total and the slowest steps.
    pub(crate) fn finish(&self) {
        self.end_current();
        let mut timings = self.timings.borrow().clone();
        timings.sort_by(|a, b| b.1.cmp(&a.1));
        println!("[load {}] background load done in {:.0} ms - slowest:", self.scene, self.started.elapsed().as_secs_f64() * 1000.0);
        for (what, took) in timings.iter().take(5) {
            println!("[load {}]   {:>7.0} ms  {what}", self.scene, took.as_secs_f64() * 1000.0);
        }
    }
}

/// What a heavy scene's background load hands back: its own `prepare`
/// output, and the models it said it uses (see `SceneManager::
/// preload_models`), each loaded or the reason it couldn't be, by key.
pub struct LoadedAssets {
    pub prepared: Box<dyn Any + Send>,
    pub models: Vec<(String, Result<LoadedModel, String>)>,
}

pub struct PendingSceneLoad {
    pub receiver: Receiver<LoadedAssets>,
    /// What the background load is on now (see `LoadReporter::step`).
    pub progress: Receiver<String>,
    pub finish: Rc<dyn Fn(&mut App, Box<dyn Any + Send>) -> Scene>,
    /// Set once `receiver` has actually yielded a result - holds it (plus how many
    /// seconds we've spent fading LoadingScreenScene's UI back out) until the fade
    /// finishes, at which point App::run does the real clear+finish+swap. None
    /// means still waiting on the background thread.
    pub ready: Option<(LoadedAssets, f32)>,
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
    // The models each heavy scene uses - see preload_models.
    preloads: HashMap<String, Rc<dyn Fn(&App) -> Vec<ModelSource>>>,
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
        Self { factories: HashMap::new(), loaders: HashMap::new(), preloads: HashMap::new(), active_scene: None, active: String::new(), reset: false, loading: None }
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
        prepare: impl Fn(&wgpu::Device, &wgpu::Queue, &wgpu::BindGroupLayout, &wgpu::SurfaceConfiguration, &LoadReporter) -> P + Send + Sync + 'static,
        finish: impl Fn(&mut Scene, &mut App, P) -> B + 'static,
    ) {
        self.loaders.insert(name.into(), Rc::new(LoadedSceneSpec {
            prepare: Arc::new(move |device, queue, layout, config, reporter| Box::new(prepare(device, queue, layout, config, reporter)) as Box<dyn Any + Send>),
            finish: Rc::new(move |app: &mut App, prepared: Box<dyn Any + Send>| {
                let prepared = *prepared.downcast::<P>().expect("SceneManager: prepared asset type mismatch for a create_loaded_scene scene");
                let mut scene = Scene::new(app);
                let behaviour = finish(&mut scene, app, prepared);
                scene.set_scene_behaviour(behaviour)
            }),
        }));
    }

    /// The models a `create_loaded_scene` scene spawns, so they load on the
    /// background thread during its loading screen (with its `prepare`)
    /// instead of one by one on the main thread as its `finish` spawns the
    /// nodes - which froze the window after the loading screen was gone.
    /// `models` runs on the main thread when the scene opens (it can read
    /// `app.model_sources` to turn declared names into files). Anything it
    /// leaves out still loads the old way, when first spawned.
    pub fn preload_models(&mut self, name: impl Into<String>, models: impl Fn(&App) -> Vec<ModelSource> + 'static) {
        self.preloads.insert(name.into(), Rc::new(models));
    }

    /// See `preload_models` - cloned out for the same reason as `factory_for`.
    pub(crate) fn preload_for(&self, name: &str) -> Option<Rc<dyn Fn(&App) -> Vec<ModelSource>>> {
        self.preloads.get(name).cloned()
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
