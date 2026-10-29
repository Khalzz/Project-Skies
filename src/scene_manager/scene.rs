use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::time::Duration;

use sdl2::EventPump;

use crate::app::{App, AppState};
use crate::game_nodes::game_object::Physics;
use crate::rendering::camera::handler::{LookAtTarget, SceneCameras, DEFAULT_CAMERA};
use crate::rendering::enviroment::environment::{Environment, PreparedEnvironment};
use crate::rendering::instance_management::InstanceData;
use crate::physics::physics_resources::PhysicsObjectDef;
use crate::scene_manager::node::Node;
use crate::scene_manager::physics_bridge::{self, TrimeshCache};
use crate::scene_manager::node::properties::{CameraLens, Material, Model as NodeModelProperty, Parent, Space, SpawnedLive, Transform3D};
use crate::scene_manager::render_bridge;
use crate::scene_manager::scene_access::SceneChanges;
use crate::scene_manager::scene_nodes::SceneNodes;
use crate::rendering::models::model::Model as LoadedModel;
use crate::resources::{ModelLoad, ModelSource};


/// Everything a scene might need out of a single frame, bundled so `SceneBehaviour::update`
/// keeps one stable signature no matter which of these fields a given scene actually
/// uses. Input state isn't here - query it directly via
/// crate::input::input (a global singleton, which also owns controller
/// connect/disconnect - see that module).
pub struct FrameContext<'a> {
    pub app_state: &'a mut AppState,
    pub event_pump: &'a mut EventPump,
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
    // engine::scene_manager::physics_bridge) while the scene is built - made
    // into bodies when its physics starts (see Scene::start_physics). One
    // spawned after that gets its body right away.
    pub physics_bodies: Vec<PhysicsObjectDef>,
    // Nodes something else is moving right now (an animation) - the
    // physics' pose isn't written onto them meanwhile (see App::run). Whoever
    // puts a node in here also holds its body still
    // (PhysicsWorld::set_kinematic) and lets it go again.
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
    /// Every physics step, after every node's `Behavior::fixed_update` and
    /// right before the world advances - see that hook.
    fn fixed_update(&mut self, scene: &mut Scene, app: &mut App) { let _ = (scene, app); }
    /// After every node's behaviors have run this frame, before the
    /// hierarchy is resolved and the cameras placed - where an animation
    /// goes, so it has the last word on what it moves.
    fn late_update(&mut self, scene: &mut Scene, app: &mut App, ctx: &mut FrameContext) { let _ = (scene, app, ctx); }
    /// After this frame's cameras and renderable transforms are the ones
    /// that will be drawn - for anything placed relative to the camera.
    fn after_cameras(&mut self, scene: &mut Scene, app: &mut App) { let _ = (scene, app); }
    /// The game's debug windows for this scene - egui, drawn over everything
    /// while the debug UI is up (see `DebugUi`). Runs every frame it's up,
    /// after the scene's update.
    fn debug_ui(&mut self, ctx: &egui::Context, scene: &mut Scene, app: &mut App) { let _ = (ctx, scene, app); }
}

/// A "screen" the game can be in (main menu, a level, ...) - the generic
/// container every scene shares: whatever nodes/renderable instances it's
/// spawned (`content`), whatever cameras it's registered (`cameras`), what it
/// sets differently from the project (`settings`), plus the `SceneBehaviour`
/// actually driving it. `SceneManager::active_scene` holds whichever one is
/// currently active; `App::run`'s loop drives it.
///
/// A scene is built first - its constructor (see `SceneManager::create_scene`)
/// spawns its nodes, which only note which models and bodies they need -
/// then those models load under the loading screen, and only then does it
/// start (its `on_spawn`, then its updates).
pub struct Scene {
    pub content: SceneContent,
    pub cameras: SceneCameras,
    behaviour: Option<Box<dyn SceneBehaviour>>,
    // Set once the scene's been built (its behaviour's on_spawn has run) -
    // nodes spawned from then on are marked SpawnedLive, and put on the
    // renderer and physics right away instead of waiting in the lists below.
    started: bool,
    /// Nodes spawned while the scene was being built, with a model or a
    /// physics body still to set up - once the scene's assets have loaded
    /// (see `finish_loading`).
    pending_models: Vec<String>,
    pending_bodies: Vec<String>,
    /// Models named with `will_use` - loaded with the scene's own.
    extra_models: Vec<ModelSource>,
}

/// What a built scene needs loaded before it starts - see
/// `Scene::assets_to_load`.
pub(crate) struct SceneAssets {
    /// Models not loaded yet - only the shared copy of each (a variant is
    /// made from it when its node is set up, see `Model::variant`).
    pub models: Vec<ModelLoad>,
    /// Every model key the scene uses - loaded already or not - so the
    /// previous scene's copies of them are kept.
    pub keys: HashSet<String>,
    /// Model files mesh colliders read.
    pub trimeshes: Vec<String>,
    /// Every texture its nodes' materials use (see `Material::texture`).
    pub(crate) textures: Vec<String>,
}

impl Scene {
    // Takes &App (not width/height directly) so a fresh scene's cameras
    // start at the real current screen size (see CameraResources::width/
    // height) without every caller needing to know where that comes from.
    pub fn new(app: &App) -> Self {
        Self {
            content: SceneContent::default(),
            cameras: SceneCameras::new(app.renderer.camera_resources.width(), app.renderer.camera_resources.height()),
            behaviour: None,
            started: false,
            pending_models: Vec::new(),
            pending_bodies: Vec::new(),
            extra_models: Vec::new(),
        }
    }

    pub fn set_scene_behaviour(mut self, behaviour: impl SceneBehaviour + 'static) -> Self {
        self.behaviour = Some(Box::new(behaviour));
        self
    }

    /// Spawns `node` into this scene's own content. If it carries a `Model`
    /// property it's put on the renderer too (see
    /// `render_bridge::register_static_model`); same deal for a `Physics`
    /// property (see `physics_bridge::register_physics_body`), or a
    /// `CameraLens` (a camera under its id, see `sync_cameras`) - in every
    /// case the property itself is what triggers instantiation, not a
    /// second call the spawner has to remember to make separately.
    /// While the scene is being built, its model and body are set up once
    /// the scene's assets have loaded, before it starts; after that, right
    /// away (a model nothing loaded yet is loaded on the spot - see
    /// `will_use`).
    /// Children given with `Node::with_child` are spawned right after it,
    /// each with its `Parent`.
    /// `Node`/`SceneNodes` stay unaware of rendering/physics either way -
    /// this just composes them at the one place that actually has both
    /// `&mut Scene` and `&mut App` available.
    pub fn spawn_node(&mut self, app: &mut App, node: Node) -> Result<(), String> {
        self.spawn_tree(app, node)
    }

    fn spawn_tree(&mut self, app: &mut App, mut node: Node) -> Result<(), String> {
        let children = node.take_children();
        let id = node.id.clone();
        let changes = self.spawn_one(app, node)?;
        for mut child in children {
            let (space, inherit_rotation) = child.placement();
            let given = child.transform_3d().copied().unwrap_or_default();
            let local = match space {
                Space::Local => given,
                Space::Global => Transform3D::default(),
            };
            child.set_property(Parent { id: id.clone(), space, local, inherit_rotation });
            self.spawn_tree(app, child)?;
        }
        // What its behaviors' on_spawn asked for, now it's in.
        self.apply_changes(app, changes);
        Ok(())
    }

    /// Does what node behaviors asked for while they ran (see
    /// `SceneAccess::spawn`/`despawn`) - despawns first, so a node can be
    /// replaced by one with the same id. One that can't be spawned is
    /// reported.
    pub(crate) fn apply_changes(&mut self, app: &mut App, changes: SceneChanges) {
        for id in changes.despawns {
            self.despawn_node(app, &id);
        }
        for node in changes.spawns {
            let id = node.id.clone();
            if let Err(error) = self.spawn_tree(app, node) {
                eprintln!("couldn't spawn node '{id}': {error}");
            }
        }
    }

    /// Spawns one node (not its children) - and hands back what its
    /// behaviors' on_spawn asked to spawn or despawn, for `spawn_tree` to do
    /// once the node and its children are in.
    fn spawn_one(&mut self, app: &mut App, mut node: Node) -> Result<SceneChanges, String> {
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
        let mut changes = SceneChanges::default();
        self.content.nodes.spawn(node, &mut self.cameras, app, &mut changes)?;

        // What it's made of - read once its behaviors' on_spawn has run, so
        // a property one of them put on (a behavior setting its own node up)
        // counts the same as one it was spawned with.
        let Some(node) = self.content.nodes.get(&id) else { return Ok(changes) };
        let has_model = node.get_property::<NodeModelProperty>().is_some();
        let has_physics = node.get_property::<Physics>().is_some();
        let lens = node.get_property::<CameraLens>().copied();
        let transform = node.transform_3d().copied().unwrap_or_default();

        if has_model {
            if self.started {
                render_bridge::register_static_model(self, app, &id)?;
            } else {
                self.pending_models.push(id.clone());
            }
        }

        if has_physics {
            if self.started {
                physics_bridge::register_physics_body(self, &id, &TrimeshCache::new())?;
            } else {
                self.pending_bodies.push(id.clone());
            }
        }

        if let Some(lens) = lens {
            self.cameras.place(&id, transform.position, transform.rotation, lens.fovy, lens.znear, lens.viewport);
        }

        // With the physics already going, its body goes in now.
        if app.physics.is_running() {
            if let Some(index) = self.content.physics_bodies.iter().rposition(|def| def.id == id) {
                app.physics.add_body(self.content.physics_bodies.remove(index));
            }
        }
        Ok(changes)
    }

    /// Removes the node `id` and its children (and theirs) - their
    /// behaviors, their instances on the renderer (see
    /// `render_bridge::unregister_model`), their cameras and their bodies. Nothing
    /// happens for an id with no node. Like `SceneNodes::clear`, anything
    /// their behaviors did outside their own node isn't undone.
    pub fn despawn_node(&mut self, app: &mut App, id: &str) {
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
            self.pending_models.retain(|pending| *pending != id);
            self.pending_bodies.retain(|pending| *pending != id);
            self.cameras.remove_camera(&id);
            self.content.kinematic.remove(&id);
            self.content.physics_bodies.retain(|def| def.id != id);
            app.physics.remove_body(&id);
        }
    }

    /// Has the model `source` gives loaded with this scene, before it starts - for a model none of its nodes use yet, but one
    /// spawned while it runs will (a missile fired later). Without this,
    /// that model loads the moment its node spawns - a stutter, for a big
    /// one. Only while the scene's being built (its constructor).
    pub fn will_use(&mut self, source: ModelSource) {
        self.extra_models.push(source);
    }

    /// What this built scene needs loaded before it can start - every model
    /// its nodes (and `will_use`) name, minus what's loaded already, and the
    /// mesh colliders' files.
    pub(crate) fn assets_to_load(&self, app: &App) -> SceneAssets {
        let mut assets = SceneAssets { models: Vec::new(), keys: HashSet::new(), trimeshes: Vec::new(), textures: Vec::new() };
        // Each model's shared copy - a variant (see Model::variant) is made
        // from it when its node is set up, but its key is kept too.
        let mut loads = Vec::new();
        for id in &self.pending_models {
            let Some(model) = self.content.nodes.get(id).and_then(|node| node.get_property::<NodeModelProperty>()) else { continue };
            assets.keys.insert(model.key());
            // One that can't be loaded is reported when its node is set up.
            if let Ok(load) = model.load(&app.assets) {
                loads.push(load);
            }
        }
        for source in &self.extra_models {
            match NodeModelProperty::new(source.clone()).load(&app.assets) {
                Ok(load) => loads.push(load),
                Err(error) => eprintln!("will_use: {error}"),
            }
        }
        let mut queued = HashSet::new();
        for load in loads {
            assets.keys.insert(load.key.clone());
            if !app.assets.models.is_loaded(&load.key) && queued.insert(load.key.clone()) {
                assets.models.push(load);
            }
        }
        for material in self.content.nodes.iter().filter_map(|node| node.get_property::<Material>()) {
            for texture in material.textures() {
                if !assets.textures.contains(texture) {
                    assets.textures.push(texture.clone());
                }
            }
        }
        for id in &self.pending_bodies {
            let Some(physics) = self.content.nodes.get(id).and_then(|node| node.get_property::<Physics>()) else { continue };
            for path in physics_bridge::trimesh_paths(physics) {
                if !assets.trimeshes.iter().any(|known| known == path) {
                    assets.trimeshes.push(path.to_owned());
                }
            }
        }
        assets
    }

    /// Puts every node spawned while the scene was built on the renderer and
    /// physics - its models are loaded by now, its mesh colliders read
    /// (`trimeshes`). A node that can't be is reported and left out.
    pub(crate) fn finish_loading(&mut self, app: &mut App, trimeshes: &TrimeshCache) {
        for id in std::mem::take(&mut self.pending_models) {
            if let Err(error) = render_bridge::register_static_model(self, app, &id) {
                eprintln!("node '{id}': {error}");
            }
        }
        for id in std::mem::take(&mut self.pending_bodies) {
            if let Err(error) = physics_bridge::register_physics_body(self, &id, trimeshes) {
                eprintln!("node '{id}': {error}");
            }
        }
        self.extra_models.clear();
    }

    /// The node `id`, if the scene has one.
    pub fn get_node(&self, id: &str) -> Option<&Node> {
        self.content.nodes.get(id)
    }

    /// The node `id`, to change - `scene.get_node_mut("player")`, then its
    /// properties (`transform_2d_mut()`, `get_property_mut::<T>()`...).
    pub fn get_node_mut(&mut self, id: &str) -> Option<&mut Node> {
        self.content.nodes.get_mut(id)
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
            let Some(transform) = node.transform_3d() else { continue };
            let Some(instance) = renderizable_instances.get_mut(&node.id) else { continue };
            let render_scale = node.get_property::<NodeModelProperty>().map(|model| model.render_scale(transform.scale)).unwrap_or(transform.scale);
            instance.instance.transform.position = transform.position;
            instance.instance.transform.rotation = transform.rotation;
            instance.instance.transform.scale = render_scale;
            match node.get_property::<Material>() {
                Some(material) => {
                    let key = material.key(&node.id);
                    if instance.material != key {
                        instance.material = key;
                    }
                }
                None if instance.material != crate::rendering::shader_library::STANDARD => {
                    instance.material = crate::rendering::shader_library::STANDARD.to_owned();
                }
                None => {}
            }
        }
    }

    /// Every camera node's camera (see `CameraLens`) to where its node is
    /// and to its lens - called every frame by `App::run`, after the
    /// hierarchy's resolved.
    pub fn sync_cameras(&mut self) {
        for node in self.content.nodes.iter() {
            let Some(lens) = node.get_property::<CameraLens>() else { continue };
            let transform = node.transform_3d().copied().unwrap_or_default();
            self.cameras.place(&node.id, transform.position, transform.rotation, lens.fovy, lens.znear, lens.viewport);
        }
    }

    /// Gives a scene that starts without a camera one, so it can always be
    /// seen: a `DEFAULT_CAMERA` node at the origin, looking along +Z. A
    /// camera node spawned later takes over from it. Called when a scene
    /// starts (see App::start_scene).
    pub(crate) fn ensure_camera(&mut self, app: &mut App) {
        if self.cameras.has_active_camera() {
            return;
        }
        let camera = Node::new(DEFAULT_CAMERA).with_property(Transform3D::default()).with_property(CameraLens::default());
        if let Err(error) = self.spawn_node(app, camera) {
            eprintln!("couldn't give the scene its default camera: {error}");
        }
    }

    /// Makes the camera node `id`'s camera the one drawn from. False if
    /// there's no such camera.
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
        // What on_spawn spawned goes on the renderer and physics now - the
        // loading step already ran, so a model nothing loaded yet loads
        // here, on the spot (spawn it in the scene's constructor to have it
        // load under the loading screen instead).
        self.finish_loading(app, &TrimeshCache::new());
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

    pub(crate) fn run_debug_ui(&mut self, app: &mut App, ctx: &egui::Context) {
        if let Some(mut behaviour) = self.behaviour.take() {
            behaviour.debug_ui(ctx, self, app);
            self.behaviour = Some(behaviour);
        }
    }

    /// Starts this scene's physics: every body its nodes have so far (see
    /// `physics_bridge`), with the gravity and timestep `app.physics` has.
    /// Called once, right after the scene becomes active - a node spawned
    /// later gets its body as it spawns.
    pub(crate) fn start_physics(&mut self, app: &mut App) {
        app.physics.start(std::mem::take(&mut self.content.physics_bodies));
    }

    pub(crate) fn run_fixed_update(&mut self, app: &mut App) {
        if let Some(mut behaviour) = self.behaviour.take() {
            behaviour.fixed_update(self, app);
            self.behaviour = Some(behaviour);
        }
    }
}

/// Used by a scene's loading thread: each `step` names what's being loaded
/// now - shown on the loading screen - and times the one before it. `finish`
/// logs every step's time, slowest first, to find what makes loading slow.
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

/// What a scene's loading thread hands back: the models and images it was
/// missing (each loaded, or why it couldn't be), its mesh colliders'
/// geometry, and the environment its constructor set, built - see
/// `App::start_scene_load`.
pub struct LoadedAssets {
    pub models: Vec<(String, Result<LoadedModel, String>)>,
    pub trimeshes: TrimeshCache,
    pub images: Vec<(String, Result<crate::asset_manager::Image, String>)>,
    pub textures: Vec<(String, Result<crate::asset_manager::ModelTexture, String>)>,
    pub environment: Option<(Environment, PreparedEnvironment)>,
}

/// A scene whose assets are loading - set when `App::run` opens a scene that
/// needs anything loaded, cleared once `receiver` yields, the loading screen
/// has faded back out, and the scene has started. See `App::run`.
pub struct PendingSceneLoad {
    /// The scene, built, waiting for its assets.
    pub(crate) scene: Scene,
    pub(crate) receiver: Receiver<LoadedAssets>,
    /// What the loading thread is on now (see `LoadReporter::step`).
    pub(crate) progress: Receiver<String>,
    /// Set once `receiver` has yielded - holds the assets (plus how many
    /// seconds we've spent fading the loading screen's UI back out) until the
    /// fade finishes, at which point App::run starts the scene. None means
    /// still loading.
    pub(crate) ready: Option<(LoadedAssets, f32)>,
}

/// The name the splash scene is opened under - see `SceneManager::set_splash_scene`.
pub(crate) const SPLASH_SCENE: &str = "__splash";

/// The scene shown first, for `duration`, before the one the game opens -
/// see `SceneManager::set_splash_scene`.
struct Splash {
    duration: Duration,
    factory: Rc<dyn Fn(&mut App) -> Scene>,
}

/// The splash scene while it's up: the scene to open once it's done, and how
/// long (seconds) it has run.
struct SplashRunning {
    next: String,
    elapsed: f32,
}

/// Owns every registered scene's constructor plus whichever one is currently
/// active - lives on `App` as `app.scene_manager`. Register scenes with
/// `create_scene`, then activate one with `open_scene` from anywhere holding
/// `&mut App`.
pub struct SceneManager {
    // Rc, not Box: App::run needs to call a factory with &mut App, which means
    // cloning it out of this map first (same reason `active` below is read via
    // .clone()) - a Box can't be cheaply cloned, an Rc can.
    factories: HashMap<String, Rc<dyn Fn(&mut App) -> Scene>>,
    // Only one scene is ever meaningfully alive at a time (inactive scenes never
    // tick), so this is a single slot rather than a map of every registered scene's
    // instance - those don't exist until create_scene's factory actually runs for
    // whichever one becomes active (see App::run).
    pub active_scene: Option<Scene>,
    pub active: String,
    pub reset: bool,
    // Some while the scene being opened has its assets loading - App::run polls
    // this every frame and starts it once they're ready. The loading screen
    // occupies active_scene in the meantime so rendering keeps happening.
    pub loading: Option<PendingSceneLoad>,
    splash: Splash,
    /// Some while the splash scene is up - see `advance_splash`.
    splash_running: Option<SplashRunning>,
}

impl SceneManager {
    pub fn new() -> Self {
        // An empty scene for no time at all - no splash, until
        // set_splash_scene says otherwise.
        let splash = Splash { duration: Duration::ZERO, factory: Rc::new(|app: &mut App| Scene::new(app)) };
        Self { factories: HashMap::new(), active_scene: None, active: String::new(), reset: false, loading: None, splash, splash_running: None }
    }

    /// The scene shown when the game starts, for `duration`, before the one
    /// it opens - built by `factory` like any scene (see `create_scene`), so
    /// it holds whatever it's given: UI, nodes, a camera, its own settings.
    /// It loads like any scene too; `duration` counts from when it starts.
    /// Opening another scene from it ends it early.
    ///
    /// By default it's an empty scene shown for no time - no splash at all.
    pub fn set_splash_scene<B: SceneBehaviour + 'static>(&mut self, duration: Duration, factory: impl Fn(&mut Scene, &mut App) -> B + 'static) {
        self.splash = Splash {
            duration,
            factory: Rc::new(move |app: &mut App| {
                let mut scene = Scene::new(app);
                let behaviour = factory(&mut scene, app);
                scene.set_scene_behaviour(behaviour)
            }),
        };
    }

    /// Opens the splash scene in place of the scene the game opened - which
    /// is opened once the splash is done (see `advance_splash`). Nothing
    /// when the splash lasts no time. Called once, as `App::run` starts.
    pub(crate) fn begin_splash(&mut self) {
        if self.splash.duration.is_zero() {
            return;
        }
        self.factories.insert(SPLASH_SCENE.to_owned(), self.splash.factory.clone());
        let next = std::mem::replace(&mut self.active, SPLASH_SCENE.to_owned());
        self.reset = true;
        self.splash_running = Some(SplashRunning { next, elapsed: 0.0 });
    }

    /// Counts the splash scene's time while it runs (not while it loads),
    /// and opens the scene the game opened once it's up. `dt`: real seconds
    /// since the last frame - a paused game doesn't hold the splash up.
    pub(crate) fn advance_splash(&mut self, dt: f32) {
        let Some(running) = self.splash_running.as_mut() else { return };
        // The splash opened another scene itself - it's over.
        if self.active != SPLASH_SCENE {
            self.splash_running = None;
            self.factories.remove(SPLASH_SCENE);
            return;
        }
        if self.reset || self.loading.is_some() {
            return;
        }
        running.elapsed += dt;
        if running.elapsed < self.splash.duration.as_secs_f32() {
            return;
        }
        let next = std::mem::take(&mut running.next);
        self.splash_running = None;
        if next.is_empty() {
            eprintln!("SceneManager: the splash scene is done, but no scene was opened to follow it");
        } else {
            self.open_scene(&next);
            self.factories.remove(SPLASH_SCENE);
        }
    }

    /// Registers a scene's constructor under `name` - nothing runs yet, `factory`
    /// only gets called once this scene actually becomes active (see `open_scene`).
    /// `factory` gets a fresh, empty `Scene` to build (spawn its nodes)
    /// alongside `&mut App`, and returns the `SceneBehaviour` driving
    /// it - so a scene's own `new(scene: &mut Scene, app: &mut App) -> Self` can
    /// be passed directly: `create_scene("main_menu", MainMenu::new)`.
    ///
    /// Every model the built scene's nodes use (and the environment, if it
    /// sets a new one) is loaded before it starts - in the background, under the
    /// loading screen, when there's anything to load (see App::start_scene_load).
    /// Overwrites whatever was previously registered under the same name.
    pub fn create_scene<B: SceneBehaviour + 'static>(&mut self, name: impl Into<String>, factory: impl Fn(&mut Scene, &mut App) -> B + 'static) {
        self.factories.insert(name.into(), Rc::new(move |app: &mut App| {
            let mut scene = Scene::new(app);
            let behaviour = factory(&mut scene, app);
            scene.set_scene_behaviour(behaviour)
        }));
    }

    /// Switches the active scene and requests its `reset` for the next frame - use
    /// this instead of setting `active`/`reset` separately, since forgetting
    /// `reset = true` leaves the new scene running with the old one's leftover data.
    /// Logs an error and leaves the current scene active if `name` isn't registered,
    /// rather than switching to a scene that doesn't exist.
    pub fn open_scene(&mut self, name: &str) {
        if self.factories.contains_key(name) {
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
}
