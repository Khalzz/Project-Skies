//! Switching scenes - what `SceneManager::open_scene` asks for, carried
//! out by `App::run` at the start of the next frame: the old scene goes, the
//! new one is built, its assets load under the loading screen (when it needs
//! any), then it starts.
//!
//! These take the whole `App` rather than `&mut self` - they reach the
//! renderer, UI, assets and physics too, and the scene manager is inside
//! `App`, so it can't be borrowed apart from it.

use std::collections::HashMap;
use std::time::Instant;

use crate::app::App;
use crate::rendering::enviroment::environment::Environment;
use crate::rendering::models::model::Mesh;
use crate::rendering::ui::ui::Ui;
use crate::resources;
use crate::scene_manager::loading_screen::{LoadingScreenScene, BACKDROP_KEY, FADE_OUT_SECS, PANEL_KEY, STEP_KEY};
use crate::scene_manager::physics_bridge;
use crate::scene_manager::scene::{LoadReporter, LoadedAssets, PendingSceneLoad, Scene, SceneAssets, SceneManager};

impl SceneManager {
    /// Opens the scene `open_scene` asked for, at the start of a frame: the old one goes (its UI, its
    /// physics, its frame plan, the models the new one doesn't use), the new
    /// one is built, and either starts right away - when everything it uses
    /// is loaded already - or waits under the loading screen while its assets
    /// load (see start_load).
    pub(crate) fn apply_open(app: &mut App) {
        app.scene_manager.reset = false;
        let name = app.scene_manager.active.clone();
        // The scene doesn't exist yet at this point - create_scene only
        // registered its constructor. Cloned out first so calling it with
        // &mut App doesn't conflict with borrowing scene_manager.
        let Some(factory) = app.scene_manager.factory_for(&name) else {
            eprintln!("No scene registered for state '{name}'");
            return;
        };

        // Nothing carries over between scenes: its nodes, cameras and
        // environment go with the old Scene itself; its UI, physics thread,
        // frame plan and line/text sets are cleared here. A load still running
        // for a scene opened before is dropped.
        app.scene_manager.active_scene = None;
        app.scene_manager.loading = None;
        app.ui.renderizable_elements.clear();
        app.ui.always_on_top.clear();
        app.ui.always_on_bottom.clear();
        app.ui.overlay.clear();
        app.ui.has_changed = true;
        app.physics.stop();
        app.renderer.render_sets.clear();
        app.renderer.frame_plan = app.renderer.default_frame_plan.clone();

        // Built - its nodes only noted which models and bodies they need,
        // its UI which images.
        let scene = factory(app);
        let assets = scene.assets_to_load(app);
        // Models and images don't carry over either - except the ones this
        // scene uses too: those stay, and aren't loaded again.
        resources::unload_level_models(app, &assets.keys);
        let image_paths = app.ui.image_paths();
        app.assets.keep_only_images(&image_paths);
        let images = app.assets.images_to_load(&image_paths);

        // A skybox's textures are read from disk - that waits under the
        // loading screen too. A color or the procedural sky is instant.
        let environment = scene.settings.environment.clone();
        let slow_environment = matches!(environment, Some(Environment::Skybox(_)));
        if assets.models.is_empty() && assets.trimeshes.is_empty() && images.is_empty() && !slow_environment {
            let environment = environment.map(|environment| (environment.clone(), app.renderer.prepare_environment(environment)));
            Self::start_scene(app, scene, LoadedAssets { models: Vec::new(), trimeshes: HashMap::new(), images: Vec::new(), environment });
        } else {
            Self::start_load(app, scene, assets, images, environment);
        }
    }

    /// Loads what `scene` needs on a background thread - its models, its
    /// mesh colliders, its UI's `images`, its environment - while the
    /// loading screen shows. Device/Queue/BindGroupLayout are cheap,
    /// Arc-backed clones, so the thread builds the GPU resources itself.
    fn start_load(app: &mut App, scene: Scene, assets: SceneAssets, images: Vec<String>, environment: Option<Environment>) {
        let device = app.renderer.device.clone();
        let queue = app.renderer.queue.clone();
        let camera_bind_group_layout = app.renderer.camera_resources.bind_group_layout.clone();
        let image_layout = app.assets.image_layout().clone();
        let config = app.renderer.config.clone();
        let SceneAssets { models, trimeshes, .. } = assets;

        let (tx, rx) = std::sync::mpsc::channel();
        let (progress_tx, progress_rx) = std::sync::mpsc::channel();
        let scene_name = app.scene_manager.active.clone();
        std::thread::spawn(move || {
            // Names each step on the loading screen and times it (logged,
            // slowest first, at the end - see LoadReporter).
            let reporter = LoadReporter::new(&scene_name, progress_tx);
            let mesh_layout = Mesh::create_bind_group_layout(&device);
            let models = models.into_iter()
                .map(|source| {
                    let key = source.key();
                    reporter.step(format!("Model {key}"));
                    let model = source.load(&device, &queue, &mesh_layout).map_err(|error| format!("failed to load model '{key}': {error}"));
                    (key, model)
                })
                .collect();
            let trimeshes = trimeshes.into_iter()
                .map(|path| {
                    reporter.step(format!("Collider {path}"));
                    let mesh = physics_bridge::load_trimesh(&path);
                    (path, mesh)
                })
                .collect();
            let images = images.into_iter()
                .map(|path| {
                    reporter.step(format!("Image {path}"));
                    let image = crate::assets::load_image_file(&path, &device, &queue, &image_layout);
                    (path, image)
                })
                .collect();
            let environment = environment.map(|environment| {
                reporter.step("Environment");
                let prepared = resources::prepare_environment(&device, &queue, &camera_bind_group_layout, &config, environment.clone());
                (environment, prepared)
            });
            reporter.finish();
            let _ = tx.send(LoadedAssets { models, trimeshes, images, environment });
        });

        app.loading_step = None;
        let loading_screen = LoadingScreenScene::new(app);
        app.scene_manager.active_scene = Some(Scene::new(app).set_scene_behaviour(loading_screen));
        app.scene_manager.loading = Some(PendingSceneLoad { scene, receiver: rx, progress: progress_rx, ready: None });
    }

    /// What the loading thread is on now, and whether it's done - checked
    /// every frame while a scene's assets load, without waiting on it.
    pub(crate) fn poll_load(app: &mut App) {
        let Some(pending) = app.scene_manager.loading.as_mut() else { return };
        while let Ok(step) = pending.progress.try_recv() {
            app.loading_step = Some(step);
        }
        if pending.ready.is_none() {
            if let Ok(loaded) = pending.receiver.try_recv() {
                pending.ready = Some((loaded, 0.0));
            }
        }
    }

    /// Once the loading thread is done: fades the loading screen's text out,
    /// then takes the loading screen down and starts the scene. After the
    /// scene tick - the loading screen's own update pulses the same text,
    /// so the fade has to be set after it to be the value that's drawn.
    pub(crate) fn advance_load(app: &mut App) {
        let Some(mut pending) = app.scene_manager.loading.take() else { return };
        let Some((loaded, mut fade_elapsed)) = pending.ready.take() else {
            app.scene_manager.loading = Some(pending);
            return;
        };

        fade_elapsed += app.time.unscaled_delta_time;
        let t = (fade_elapsed / FADE_OUT_SECS).clamp(0.0, 1.0);
        // Only the text fades - the black backdrop stays fully opaque until
        // it's taken down below, so the scene appears from black.
        if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &format!("{PANEL_KEY}/label")) {
            node.set_alpha(1.0 - t);
        }
        if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, STEP_KEY) {
            node.set_alpha(1.0 - t);
        }
        app.ui.has_changed = true;

        if t < 1.0 {
            pending.ready = Some((loaded, fade_elapsed));
            app.scene_manager.loading = Some(pending);
            return;
        }

        // Only the loading screen's own nodes go - the scene built its UI
        // already, underneath them.
        for key in [BACKDROP_KEY, PANEL_KEY, STEP_KEY] {
            app.ui.renderizable_elements.remove(key);
        }
        app.ui.always_on_top.retain(|key| ![BACKDROP_KEY, PANEL_KEY, STEP_KEY].contains(&key.as_str()));
        app.loading_step = None;
        Self::start_scene(app, pending.scene, loaded);
    }

    /// Starts a built scene with its assets loaded: its nodes go on the
    /// renderer and physics, its `on_spawn` runs, its physics thread starts,
    /// and it becomes the active scene.
    fn start_scene(app: &mut App, mut scene: Scene, loaded: LoadedAssets) {
        let started = Instant::now();
        resources::adopt_level_models(app, loaded.models);
        app.assets.adopt_images(loaded.images);
        scene.environment = loaded.environment;
        scene.finish_loading(app, &loaded.trimeshes);
        scene.run_on_spawn(app);
        println!("[load {}] starting the scene: {:.0} ms", app.scene_manager.active, started.elapsed().as_secs_f64() * 1000.0);

        // A scene that didn't set a camera up gets a camera node at the
        // origin, so it can always be seen.
        scene.ensure_camera(app);

        // Re-baseline the frame clock: without this, the first delta_time the
        // scene's update sees would include however long it took to build and
        // set up - enough to make anything timing its first frames skip ahead.
        app.time.update();

        let thread = scene.start_physics(app);
        app.physics.start(thread, scene.settings.gravity(&app.settings));
        app.scene_manager.active_scene = Some(scene);
    }
}
