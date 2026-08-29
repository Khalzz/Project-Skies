use std::{collections::HashMap, f32::consts::PI, hash::Hash, time::{Duration, Instant}};

use glyphon::FontSystem;
use nalgebra::{vector, Point3, UnitQuaternion, Vector3};
use rapier3d::prelude::RigidBody;
use sdl2::{controller::GameController};
use crate::{app::{App, AppState}, engine::audio::subtitles::Subtitle, engine::input::input, engine::physics::physics_handler::{MetadataType, PhysicsCommand, PhysicsData, PhysicsTick, RenderMessage}, engine::physics::physics_resources::PhysicsObjectDef, engine::primitive::manual_vertex::ManualVertex, engine::rendering::{camera::camera::yaw_pitch_rotation, enviroment::environment::Environment, ui::ui::Ui}, engine::scene_manager::{node::Node, properties::{Model, Transform3D}, scene::{FrameContext, Scene, SceneBehaviour}}, transform::Transform, engine::ui::{color::UiColor, ui_node::{UiNode, UiNodeContent}, ui_transform::{Anchor, Orientation, PositionValue, SizeValue, UiTransform}}, engine::utils::lerps::{lerp, lerp_point3, lerp_quaternion, smoothstep}};
use crate::engine::game_nodes::game_object::{Camera as GameObjectCamera, Cameras, ColliderType, Lighting, Physics as PhysicsProperty, RigidBodyData};
use super::{event_handling::EventSystem, plane::{physics_logic::PlanePhysicsLogic, plane::Plane}};
use std::sync::mpsc::Sender;
use crate::game::scenes::play::plane::plane::PlaneControls;
use crate::game::selected_level::SELECTED_LEVEL;
use crate::game::ui::label;
use crate::game::scenes::play::ui as play_ui;
use crate::resources;
use crate::engine::tooling::debug_console;

// Note: "altitude"/"altitude_alert"/"stall_alert" below are stale references
// to nodes that were never added to the flight HUD (see play::ui) -
// `Ui::get_ui_node` just no-ops on a missing key.

// The normal flight follow-cam's own FOV - matches CameraHandler::new's default
// for "main". Asserted every frame by CameraState::Normal (see camera_control)
// so a CameraTrack::LookAt's FOV push-in (see level_planning.ron's camera_tracks)
// reliably lets go once its own end_time passes, instead of leaving the last
// keyframe's value stuck forever - every other CameraState already sets fovy
// unconditionally each frame for the same reason, Normal just never did.
const NORMAL_CAMERA_FOV: f32 = 45.0;

// Add a way of setting timing that can be agnostic to real time (or that will not be affected by the player pausing)
pub enum CameraState {
    Normal,
    Cockpit,
    Cinematic,
    Frontal,
    Free,
}

pub struct CameraData {
    camera_state: CameraState,
    pub look_at: Option<Vector3<f32>>,
    pub next_look_at: Option<Vector3<f32>>,
    pub mod_quaternion: UnitQuaternion<f32>,
    pub debug_offset: Vector3<f32>,
    pub debug_mode_active: bool,
    pub cockpit_current_rotation: UnitQuaternion<f32>,
    pub cockpit_target_fov: f32,
    pub cockpit_current_fov: f32,
    pub cockpit_yaw: f32,
    pub cockpit_pitch: f32,
    pub cockpit_current_head_shift: f32,
    pub free_yaw: f32,
    pub free_pitch: f32,
    pub free_current_rotation: UnitQuaternion<f32>,
    pub free_target_fov: f32,
    pub free_current_fov: f32,
    // Set the instant a CameraTrack::Shot's window ends (see GameLogic::update's
    // cinematic falling-edge block), consumed and cleared by camera_control's own
    // final-apply step - see that field's own doc comment for why this lives here
    // instead of using CameraHandler::transition_to.
    cinematic_return_blend: Option<CinematicReturnBlend>,
}

// A Shot track writes straight into "main" every frame it's active (see
// animation_tracks.rs's own doc comment on why - "assert every field every frame"),
// and the instant its window ends it simply stops running, leaving whatever
// camera_control's own CameraState computes for *this* frame to land with no
// transition at all - a hard cut. CameraHandler::transition_to doesn't fit here:
// it blends toward a *named* camera's live value, but camera_control only ever
// writes through `active_mut()` (never by name), so the moment a transition starts,
// `select_camera` would swap "main" out from under it and it'd stop being written to
// entirely until the blend finishes - a stall-then-snap, not an improvement. Blending
// locally instead, in the one place that's already the sole per-frame writer of
// "main"'s pose, sidesteps that: the "from" pose is a value snapshot (position +
// look-at point reconstructed from yaw/pitch via calc_forward_direction, since Camera
// itself has no look-at-point field), the "to" pose is whatever camera_control's
// current CameraState just computed *this* frame - always live, since it's read on
// the same frame it's produced.
struct CinematicReturnBlend {
    elapsed: f32,
    duration: f32,
    from_position: Point3<f32>,
    from_look_at: Point3<f32>,
    from_fov: f32,
}

pub struct BlinkingAlert {
    alert_state: bool,
    time_alert: f32
}

pub struct GameLogic { // here we define the data we use on our script
    pub camera_data: CameraData,
    pub blinking_alerts: HashMap<String, BlinkingAlert>,
    pub gravity: Vector3<f32>,
    pub subtitle_data: Subtitle,
    pub start_time: Instant,
    pub event_system: Option<EventSystem>,
    pub game_time: f64,
    // Last frame's CameraTrack::LookAt-active state - lets `update` detect the
    // exact rising/falling edge to pause/resume physics on, see its own comment.
    was_cinematic_active: bool,
    // Last frame's `app.is_paused` - same rising/falling-edge idiom as
    // `was_cinematic_active`, for the pause menu (see `App::is_paused`'s own
    // doc comment for why this can't just react synchronously inside the
    // pause menu's own UI button callbacks).
    was_paused: bool,
    // Seconds ESC has been held continuously during an `EventSystem::
    // input_lock_end` window - see `update`'s own "hold ESC to skip" handling.
    // Reset to 0.0 the instant ESC isn't held or the lock ends, so a skip
    // always needs a single unbroken hold, not an accumulated total across
    // several taps.
    esc_hold_time: f32,
}

/// Everything about opening "playing" that's slow enough to cause a visible
/// stall if done inline on the main thread - the skybox's cubemap textures,
/// and re-parsing ground/ground.glb's raw geometry for its trimesh collider
/// (`spawn_world` below only ever gets the already-resolved, still-unscaled
/// result - see its own comment on why "ground" builds a `ColliderType::
/// Trimesh` directly instead of a `TrimeshFromModel`, which would otherwise
/// re-read the file itself, synchronously, at spawn time). Produced by
/// `GameLogic::prepare` on a background thread, consumed by `GameLogic::finish`
/// on the main thread - see `SceneManager::create_loaded_scene`.
pub struct PreparedPlayAssets {
    environment: resources::PreparedEnvironment,
    ground_trimesh: (Vec<Vector3<f32>>, Vec<[u32; 3]>),
}

impl GameLogic {
    /// Background-thread half of construction - see `PreparedPlayAssets`'s own
    /// doc comment for why these two specifically are the slow part, not
    /// everything `finish` still does synchronously afterward (node spawning
    /// itself is cheap - the models it references are already resident in
    /// `app.game_models` by the time any scene opens, see main.rs's own
    /// `resources::register_model` calls).
    pub fn prepare(device: &wgpu::Device, queue: &wgpu::Queue, camera_bind_group_layout: &wgpu::BindGroupLayout, config: &wgpu::SurfaceConfiguration, environment: Environment) -> PreparedPlayAssets {
        let environment = resources::prepare_environment(device, queue, camera_bind_group_layout, config, environment);
        let ground_trimesh = resources::load_trimesh_geometry("ground/ground.glb").unwrap_or_else(|error| {
            eprintln!("play: ground trimesh from 'ground/ground.glb' couldn't be loaded: {error}");
            (vec![], vec![])
        });
        PreparedPlayAssets { environment, ground_trimesh }
    }

    // Main-thread half - everything else "playing" needs, now cheap since the
    // slow disk/GPU work already happened in `prepare` above. spawn_world
    // spawns test_chamber's old data.ron objects directly as nodes; physics
    // bodies get picked up automatically by Scene::spawn_node (see
    // engine::scene_manager::physics_bridge) purely because they carry a
    // Physics property, same as a Model property triggers render registration.
    pub fn finish(scene: &mut Scene, app: &mut App, prepared: PreparedPlayAssets) -> Self {
        scene.environment.skybox = prepared.environment.skybox;
        scene.environment.clear_color = prepared.environment.clear_color;
        app.scene_openned = Some("./assets/scenes/test_chamber".to_owned());

        Self::spawn_world(scene, app, prepared.ground_trimesh);

        // "main" is created here explicitly now - CameraHandler::clear_scene_cameras
        // (see App::run's reset handling) drops every camera on every scene
        // switch, no default survives implicitly any more. Seeded with the
        // same values the engine used to hardcode as its own default; only
        // matters for the first frame or so, camera_control immediately takes
        // over driving this from the plane's transform every frame after.
        //
        // The main menu switches the active camera to its own "main_menu" one
        // (see main_menu::scene::MENU_CAMERA_NAME) and nothing ever switches it
        // back - without the select_camera below, gameplay (camera_control and
        // every CameraTrack targeting "main") keeps writing into the "main"
        // CameraInstance while "main_menu" stays the one actually rendered, so
        // none of it is ever visible.
        scene.create_camera("main", Transform::new(Vector3::new(0.0, 0.0, 0.0), yaw_pitch_rotation(-90.0, -20.0), Vector3::new(1.0, 1.0, 1.0)), 45.0);
        scene.select_camera("main");

        // Defensive reset - see App::is_paused's own doc comment. Should
        // already be false by the time a scene starts (both pause-menu exit
        // paths clear it before triggering the reset that lands here), but a
        // scene that isn't "playing" (e.g. the main menu) never touches this
        // flag at all, so it'd otherwise carry over stale from wherever it
        // was last left.
        app.is_paused = false;

        // Flight HUD - built inactive (see play::ui::build_game_ui's own doc
        // comment); the `Active` ui_tracks in level_planning.ron bring it back
        // once the mission-intro card below is done.
        let screen_width = app.window_manager.size.width as f32;
        let screen_height = app.window_manager.size.height as f32;

        // add_to_ui inserts straight into renderizable_elements, unlike a
        // Layer::build-constructed node - nothing else ever resolves a node's
        // pending Percent/Pixels size or Start/Center/End position against the
        // real screen size, so without this each of these three would silently
        // stay at whatever 0-sized/0-positioned default UiTransform::new gives
        // a brand new node (see the identical comment on `backdrop` below).
        let mut game_ui = play_ui::build_game_ui(app);
        game_ui.resolve(screen_width, screen_height);
        app.ui.add_to_ui("game_ui".to_owned(), game_ui);
        let mut velocity_marker = play_ui::build_velocity_marker(app);
        velocity_marker.resolve(screen_width, screen_height);
        app.ui.add_to_ui("velocity_marker".to_owned(), velocity_marker);
        let mut subtitles = play_ui::build_subtitles(app);
        subtitles.resolve(screen_width, screen_height);
        app.ui.add_to_ui("subtitles".to_owned(), subtitles);
        let mut skip_prompt = play_ui::build_skip_prompt(app);
        skip_prompt.resolve(screen_width, screen_height);
        app.ui.add_to_ui("skip_prompt".to_owned(), skip_prompt);
        let mut debug_panel = play_ui::build_debug_panel(app);
        debug_panel.resolve(screen_width, screen_height);
        app.ui.add_to_ui("debug_panel".to_owned(), debug_panel);

        // Pause menu ("Escape") - see play::ui::build_pause_menu's own doc
        // comment. Registers its own layer (uses Layer::build, not the manual
        // resolve()+add_to_ui above, since it needs several independently-
        // addressable top-level nodes at once).
        play_ui::build_pause_menu(app);

        // Mission-intro card: build a full-screen black backdrop plus
        // the (invisible-at-rest) title/location/date stack from whatever
        // main_menu::ui::show_level last staged in SELECTED_LEVEL (falling
        // back to placeholder text if the scene was somehow opened without
        // going through Play Select first) - both fade in/out together, driven
        // by the same ui_tracks (see docs/animation_tracks.md).
        let selected = SELECTED_LEVEL.lock().unwrap().clone();
        let (mission_title, location, mission_date) = match selected {
            Some(level) => (level.mission_title, level.location, level.mission_date),
            None => ("Unknown Mission".to_owned(), String::new(), String::new()),
        };

        let mut backdrop = UiNode::container()
            .set_size(SizeValue::Percent(100.0), SizeValue::Percent(100.0))
            .set_position(PositionValue::Start(0.0), PositionValue::Start(0.0))
            .set_background_color(UiColor::Rgba(0, 0, 0, 255));
        // Ui::add_to_ui (below) inserts straight into renderizable_elements,
        // unlike a Layer::build-constructed node - nothing else ever resolves
        // this node's pending Percent size/Start position against the real
        // screen size, so without this explicit call it'd silently stay at
        // whatever 0-sized/0-positioned default UiTransform::new gives a
        // brand new node.
        backdrop.resolve(screen_width, screen_height);
        app.ui.add_to_ui("mission_intro_backdrop".to_owned(), backdrop);
        // Regular top-level nodes have no guaranteed render order
        // (renderizable_elements is a HashMap) - always_on_top guarantees
        // both render above every other UI element (the HUD, even though
        // it's inactive right now) *and*, since it's processed in this Vec's
        // own order, that each later entry lands on top of the earlier ones.
        app.ui.always_on_top.push("mission_intro_backdrop".to_owned());
        app.ui.always_on_top.push("mission_intro".to_owned());
        // "subtitles" goes on top of both (pushed last) - dialogue outranks the
        // mission-intro card (see play::ui::build_subtitles's own doc comment
        // on why it's never hidden/faded by the intro sequence at all), so
        // without this it'd stay correctly *active* through the intro but
        // still render underneath the opaque black backdrop, effectively
        // invisible anyway.
        app.ui.always_on_top.push("subtitles".to_owned());

        let mission_intro_line = |app: &mut App, text: &str, size: f32, color: UiColor| {
            label(app, text)
                .set_font_size(&mut app.ui.text.font_system, size)
                .set_text_color(color.with_alpha(0.0))
        };
        let mut mission_intro = UiNode::container()
            .set_orientation(Orientation::Vertical)
            .set_size(SizeValue::Fit, SizeValue::Fit)
            .set_position(PositionValue::Start(60.0), PositionValue::Center(0.0))
            .set_background_color(UiColor::TRANSPARENT)
            .set_child_anchor(Anchor::Start, Anchor::Start)
            .set_gap(8.0)
            .set_child("title", mission_intro_line(app, &mission_title, 34.0, UiColor::WHITE))
            .set_child("location", mission_intro_line(app, &location, 20.0, UiColor::Rgb(210, 210, 210)))
            .set_child("date", mission_intro_line(app, &mission_date, 16.0, UiColor::Rgb(170, 170, 170)));
        mission_intro.resolve(screen_width, screen_height);
        app.ui.add_to_ui("mission_intro".to_owned(), mission_intro);

        // UI ELEMENTS AND LIST
        /*
        let altitude = UiNode::new(
            UiTransform::new(((app.config.width as f32 / 2.0) - (150.0 / 2.0)) - 400.0, (app.config.height as f32 / 2.0) - (30.0 / 2.0), 30.0, 150.0, 0.0, false), 
            Visibility::new([0.0, 0.0, 0.0, 0.0], [0.0, 255.0, 0.0, 255.0]),
            UiNodeParameters::Text { text: "ALT", color: Color::rgba(0, 255, 75, 255), align: Align::Center, font_size: 20.0}, 
            app,
        );

        let speed = UiNode::new(
            UiTransform::new(((app.config.width as f32 / 2.0) - (150.0 / 2.0)) + 400.0, (app.config.height as f32 / 2.0) - (30.0 / 2.0), 30.0, 150.0, 0.0, false), 
            Visibility::new([0.0, 0.0, 0.0, 0.0], [0.0, 255.0, 0.0, 255.0]),
            UiNodeParameters::Text { text: "SPD", color: Color::rgba(0, 255, 75, 255), align: Align::Center, font_size: 20.0}, 
            app,
        );
        
        let altitude_alert = UiNode::new(
            UiTransform::new((app.config.width as f32 / 2.0) - (140.0 / 2.0), ((app.config.height as f32 / 2.0) - (50.0 / 2.0)) + 50.0, 50.0, 140.0, 0.0, false), 
            Visibility::new([0.0, 0.0, 0.0, 0.0], [255.0, 0.0, 0.0, 255.0]),
            UiNodeParameters::Text { text: "ALT", color: Color::rgba(0, 255, 75, 255), align: Align::Center, font_size: 20.0 }, 
            app,
        );

        let compass = UiNode::new(
            UiTransform::new((app.config.width as f32 / 2.0) - (100.0 / 2.0), 300.0, 50.0, 100.0, 0.0, false), 
            Visibility::new([0.0, 0.0, 0.0, 0.0], [0.0, 255.0, 0.0, 255.0]),
            UiNodeParameters::Text { text: "90°", color: Color::rgba(0, 255, 75, 255), align: Align::Center, font_size: 20.0 }, 
            app,
        );

        let timer = UiNode::new(
            UiTransform::new(10.0, 10.0, 30.0, 100.0, 0.0, false), 
            Visibility::new([0.0, 0.0, 0.0, 0.0], [0.0, 255.0, 0.0, 255.0]),
            UiNodeParameters::Text { text: "00:00:000", color: Color::rgba(0, 255, 75, 255), align: Align::Center, font_size: 20.0}, 
            app,
        );

        let framerate = UiNode::new(
            UiTransform::new(10.0, 10.0, 30.0, 100.0, 0.0, false), 
            Visibility::new([0.0, 0.0, 0.0, 0.0], [0.0, 255.0, 0.0, 255.0]),
            UiNodeParameters::Text { text: "90 fps", color: Color::rgba(0, 255, 75, 255), align: Align::Center, font_size: 20.0}, 
            app,
        );

        let g_number = UiNode::new(
            UiTransform::new(10.0, 50.0, 30.0, 100.0, 0.0, false), 
            Visibility::new([0.0, 0.0, 0.0, 0.0], [0.0, 255.0, 0.0, 255.0]),
            UiNodeParameters::Text { text: "G", color: Color::rgba(0, 255, 75, 255), align: Align::Center, font_size: 20.0}, 
            app,
        );

        let throttle_value = UiNode::new(
            UiTransform::new(10.0, 50.0, 30.0, 100.0, 0.0, false), 
            Visibility::new([0.0, 0.0, 0.0, 0.0], [0.0, 255.0, 0.0, 255.0]),
            UiNodeParameters::Text { text: "0%", color: Color::rgba(0, 255, 75, 255), align: Align::Center, font_size: 20.0}, 
            app,
        );

        let mut game_info = UiNode::new(
            UiTransform::new(10.0, 10.0, 0.0, 150.0, 0.0, false), 
            Visibility::new([0.0, 0.0, 0.0, 0.7], [0.0, 0.0, 0.0, 0.0]),
            UiNodeParameters::VerticalContainerData { margin: 10.0, separation: 10.0, children: ChildrenType::MappedChildren(HashMap::new()) }, 
            app,
        );

        */

        /* 
        game_info.add_children("framerate".to_owned(), framerate);
        game_info.add_children("g_number".to_owned(), g_number);
        game_info.add_children("timer".to_owned(), timer);
        game_info.add_children("throttle_value".to_owned(), throttle_value);

        let subtitle = UiNode::new(
            UiTransform::new((app.config.width as f32 / 2.0) - (app.config.width as f32 * 0.9) / 2.0, app.config.height as f32 * 0.7, 0.0, app.config.width as f32 * 0.9, 0.0, true), 
            Visibility::new([0.0, 0.0, 0.0, 0.7], [0.0, 0.0, 0.0, 0.0]),
            UiNodeParameters::VerticalContainerData { margin: 10.0, separation: 10.0, children: ChildrenType::IndexedChildren(vec![]) }, 
            app,
        );
        

        app.ui.renderizable_elements.clear();
        app.ui.renderizable_elements.insert("static".to_owned(), UiContainer::Tagged(HashMap::new()));
        app.ui.renderizable_elements.insert("bandits".to_owned(), UiContainer::Untagged(vec![]));

        app.ui.add_to_ui("static".to_owned(), "altitude".to_owned(), altitude);

        app.ui.add_to_ui("static".to_owned(), "speed".to_owned(), speed);
        app.ui.add_to_ui("static".to_owned(), "compass".to_owned(), compass);
        app.ui.add_to_ui("static".to_owned(), "altitude_alert".to_owned(), altitude_alert);
        app.ui.add_to_ui("static".to_owned(), "subtitles".to_owned(), subtitle);
        app.ui.add_to_ui("static".to_owned(), "game_info".to_owned(),game_info);
        */

        let subtitle_data = Subtitle::new();

        let camera_data = CameraData { 
            camera_state: CameraState::Normal, 
            look_at: None,
            next_look_at: None,
            mod_quaternion: UnitQuaternion::identity(),
            debug_offset: Vector3::new(0.0, 0.6, -3.0),
            debug_mode_active: false,
            cockpit_current_rotation: UnitQuaternion::identity(),
            cockpit_target_fov: 70.0,
            cockpit_current_fov: 70.0,
            cockpit_yaw: 0.0,
            cockpit_pitch: 0.0,
            cockpit_current_head_shift: 0.0,
            free_yaw: 0.0,
            free_pitch: 0.0,
            free_current_rotation: UnitQuaternion::identity(),
            free_target_fov: 60.0,
            free_current_fov: 60.0,
            cinematic_return_blend: None,
        };

        let mut blinking_alerts: HashMap<String, BlinkingAlert> = HashMap::new();
        blinking_alerts.insert("altitude".to_owned(), BlinkingAlert { alert_state: false, time_alert: 0.0 });
        blinking_alerts.insert("stall".to_owned(), BlinkingAlert { alert_state: false, time_alert: 0.0 });

        let gravity = vector![0.0, -9.81, 0.0];

        let event_system = match EventSystem::new(&app.scene_openned) {
            Ok(system) => Some(system),
            Err(error) => {
                eprintln!("Error: {}", error);
                None
            },
        };

        // This scene's environment (skybox vs flat color) is declared where it's
        // registered - see main.rs's create_loaded_scene("playing", ...) call -
        // and already applied above from `prepared.environment`.

        Self {
            camera_data,
            blinking_alerts,
            gravity,
            start_time: Instant::now(),
            event_system,
            subtitle_data,
            game_time: 0.0,
            was_cinematic_active: false,
            was_paused: false,
            esc_hold_time: 0.0,
        }
    }

    // Everything test_chamber's old data.ron authored, spawned directly as
    // nodes instead - same ids/transforms/models/physics/camera metadata,
    // just code-first. "ground" carries no Physics property (matching
    // data.ron exactly - the water plane's HalfSpace collider below is what
    // actually provides the floor, "ground" is purely decorative).
    //
    // `ground_trimesh` is `GameLogic::prepare`'s already-loaded (but not yet
    // scaled) ground/ground.glb geometry.
    fn spawn_world(scene: &mut Scene, app: &mut App, ground_trimesh: (Vec<Vector3<f32>>, Vec<[u32; 3]>)) {
        scene.spawn_node(app,
            Node::new("sun")
                .add_property(Transform3D {
                    position: Vector3::new(1000.0, 1_000_000.0, 1000.0),
                    rotation: UnitQuaternion::identity(),
                    scale: Vector3::new(1.0, 1.0, 1.0),
                })
                .add_property(Model { model_ref: "F16".to_owned() })
        ).expect("play should only spawn 'sun' once");
        if let Some(sun) = scene.content.renderizable_instances.get_mut("sun") {
            sun.instance.metadata.lighting = Some(Lighting { intensity: 1.0, color: Vector3::new(0.7, 0.7, 0.8) });
        }

        let ground_scale = Vector3::new(30_000.0, 30_000.0, 30_000.0);
        scene.spawn_node(app,
            Node::new("ground")
                .add_property(Transform3D {
                    position: Vector3::new(0.0, 0.0, 0.0),
                    rotation: UnitQuaternion::identity(),
                    scale: ground_scale,
                })
                .add_property(Model { model_ref: "Ground".to_owned() })
                // Collision shape lifted straight from the model's own
                // low-poly triangles - a good fit for irregular mountain
                // terrain no primitive shape approximates well. Already
                // loaded (GameLogic::prepare, on a background thread - see
                // that struct's own doc comment) by the time spawn_world
                // runs, just scaled here to match this node's own
                // Transform3D.scale above (see ColliderType::TrimeshFromModel's
                // own doc comment for the alternative - resolving straight from
                // a bare model_path at spawn time - which is what a node
                // wants when its geometry doesn't need to be background-loaded).
                .add_property(PhysicsProperty {
                    rigidbody: RigidBodyData {
                        is_static: true,
                        mass: 0.0,
                        center_of_mass: Vector3::new(0.0, 0.0, 0.0),
                        initial_velocity: Vector3::new(0.0, 0.0, 0.0),
                    },
                    colliders: vec![ColliderType::Trimesh {
                        vertices: ground_trimesh.0.iter().map(|v| Vector3::new(v.x * ground_scale.x, v.y * ground_scale.y, v.z * ground_scale.z)).collect(),
                        indices: ground_trimesh.1,
                    }],
                })
        ).expect("play should only spawn 'ground' once");

        scene.spawn_node(app,
            Node::new("player")
                .add_behavior(Plane::new())
                .add_property(Transform3D {
                    position: Vector3::new(0.0, 1000.0, 0.0),
                    rotation: UnitQuaternion::identity(),
                    scale: Vector3::new(14.0, 14.0, 14.0),
                })
                .add_property(Model { model_ref: "F16".to_owned() })
                .add_property(PhysicsProperty {
                    rigidbody: RigidBodyData {
                        is_static: false,
                        mass: 8900.0,
                        center_of_mass: Vector3::new(0.0, 0.0, 0.5),
                        initial_velocity: Vector3::new(0.0, 0.0, 200.4),
                    },
                    colliders: vec![
                        ColliderType::Cuboid { half_extents: (1.4, 1.4, 9.8), position: (0.0, 0.0, 2.8) },
                        ColliderType::Cuboid { half_extents: (4.2, 0.14, 2.8), position: (6.0, 0.42, 2.8) },
                        ColliderType::Cuboid { half_extents: (4.2, 0.14, 2.8), position: (-6.0, 0.42, 2.8) },
                    ],
                })
        ).expect("play should only spawn 'player' once");
        if let Some(player) = scene.content.renderizable_instances.get_mut("player") {
            let mut cameras: Cameras = HashMap::new();
            cameras.insert("cockpit".to_owned(), GameObjectCamera { position: Vector3::new(0.0, 1.8, 13.5), fov: 70.0 });
            cameras.insert("cinematic".to_owned(), GameObjectCamera { position: Vector3::new(0.0, 1000.0, 900.0), fov: 60.0 });
            cameras.insert("frontal".to_owned(), GameObjectCamera { position: Vector3::new(0.0, 6.0, 35.0), fov: 40.0 });
            player.instance.metadata.cameras = Some(cameras);
        }

        scene.spawn_node(app,
            Node::new("fellow_aviator")
                .add_property(Transform3D {
                    position: Vector3::new(50.0, 50.0, 0.0),
                    rotation: UnitQuaternion::identity(),
                    scale: Vector3::new(19.0, 19.0, 19.0),
                })
                .add_property(Model { model_ref: "F14".to_owned() })
                .add_property(PhysicsProperty {
                    rigidbody: RigidBodyData {
                        is_static: true,
                        mass: 9000.0,
                        center_of_mass: Vector3::new(0.0, 0.0, 0.0),
                        initial_velocity: Vector3::new(0.0, 0.0, 0.0),
                    },
                    colliders: vec![
                        ColliderType::Cuboid { half_extents: (1.0, 1.0, 1.0), position: (0.0, 0.0, 0.0) },
                    ],
                })
        ).expect("play should only spawn 'fellow_aviator' once");
        if let Some(fellow_aviator) = scene.content.renderizable_instances.get_mut("fellow_aviator") {
            let mut cameras: Cameras = HashMap::new();
            cameras.insert("cockpit".to_owned(), GameObjectCamera { position: Vector3::new(0.0, 2.3, 14.3), fov: 60.0 });
            cameras.insert("cinematic".to_owned(), GameObjectCamera { position: Vector3::new(-10.0, 3.0, 0.0), fov: 60.0 });
            cameras.insert("frontal".to_owned(), GameObjectCamera { position: Vector3::new(0.0, 6.0, 26.0), fov: 60.0 });
            fellow_aviator.instance.metadata.cameras = Some(cameras);
        }

        scene.spawn_node(app,
            Node::new("world")
                .add_property(Transform3D {
                    position: Vector3::new(0.0, 0.0, 0.0),
                    rotation: UnitQuaternion::identity(),
                    scale: Vector3::new(100_000.0, 1.0, 100_000.0),
                })
                .add_property(Model { model_ref: "Water".to_owned() })
                .add_property(PhysicsProperty {
                    rigidbody: RigidBodyData {
                        is_static: true,
                        mass: 0.0,
                        center_of_mass: Vector3::new(0.0, 0.0, 0.0),
                        initial_velocity: Vector3::new(0.0, 0.0, 0.0),
                    },
                    colliders: vec![
                        ColliderType::HalfSpace { normal: Vector3::new(0.0, 1.0, 0.0) },
                    ],
                })
        ).expect("play should only spawn 'world' once");
    }

    // How long ESC has to be held during an EventSystem::input_lock_end
    // window before it counts as a skip - see `update`'s own handling.
    const SKIP_HOLD_SECONDS: f32 = 1.2;

    // this is called every frame
    pub fn update(&mut self, scene: &mut Scene, app: &mut App, plane_control_tx: Option<&Sender<PlaneControls>>, physics_command_tx: Option<&Sender<PhysicsCommand>>, physics_data: &HashMap<String, RenderMessage>) {
        // F3 debug view - shows/hides in lockstep with the console toggle,
        // same "set_active every frame, no separate dirty-tracking" idiom
        // skip_prompt below already uses (its own text is updated inside
        // ui_control, right alongside compass/speed/etc. - not from a
        // function of its own).
        if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "debug_panel") {
            node.set_active(debug_console::is_console_visible());
        }

        // Whether player input is locked out right now (see EventSystem::
        // input_lock_end's own doc comment) - checked against this frame's
        // not-yet-incremented game_time, one frame stale at worst, which
        // doesn't matter for a coarse window check like this.
        let input_lock_end = self.event_system.as_ref().and_then(|es| es.input_lock_end(self.game_time));
        if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "skip_prompt") {
            node.set_active(input_lock_end.is_some());
        }

        if let Some(lock_end_ms) = input_lock_end {
            // Only a "hold ESC to skip" affordance works during a locked
            // sequence - not the pause menu itself (see EventSystem::
            // input_lock_end's own doc comment on why input locking is its
            // own concept rather than reusing is_cinematic_camera_active).
            if input::is_action_pressed("toggle_pause_menu") {
                self.esc_hold_time += app.time.delta_time;
                if self.esc_hold_time >= Self::SKIP_HOLD_SECONDS {
                    // Jump to just *before* the lock ends, not past/at it -
                    // this frame's own apply_tracks call (further down) still
                    // samples inside the window one last time, landing every
                    // scripted object/camera at its authored final position,
                    // rather than leaving whatever was on screen mid-sequence
                    // as the abrupt "final" frame. The window's own falling
                    // edge (e.g. the cinematic pause/resume handling below)
                    // then fires naturally next frame, exactly as if the
                    // sequence had simply finished on its own.
                    self.game_time = lock_end_ms.saturating_sub(1) as f64 / 1000.0;
                    self.esc_hold_time = 0.0;
                }
            } else {
                self.esc_hold_time = 0.0;
            }
        } else {
            self.esc_hold_time = 0.0;

            // Pause menu ("Escape") - see App::is_paused's own doc comment for
            // why this flag lives on App rather than here, and play::ui::
            // open_pause_menu/close_pause_menu for what actually toggles it.
            // Has to run before the early-return below, since it's what
            // actually clears app.is_paused again on resume.
            if input::is_action_just_pressed("toggle_pause_menu") {
                if app.is_paused {
                    play_ui::close_pause_menu(app);
                } else {
                    play_ui::open_pause_menu(app);
                }
            }
        }

        // Rising/falling edge, same idiom as the cinematic pause/resume below -
        // no SetTransform teleport needed here (unlike the cinematic case,
        // nothing else is scripting the plane's position while paused, so
        // physics's own resting state is exactly where it should resume from).
        // Also has to run before the early-return, since it's what actually
        // (un)pauses physics.
        let just_paused = app.is_paused && !self.was_paused;
        if let Some(physics_command_tx) = physics_command_tx {
            if just_paused {
                let _ = physics_command_tx.send(PhysicsCommand::TogglePause);
            } else if !app.is_paused && self.was_paused {
                let _ = physics_command_tx.send(PhysicsCommand::TogglePause);
            }
        }
        self.was_paused = app.is_paused;

        if app.is_paused {
            // App::run applies this frame's fresh physics data to render
            // transforms *before* calling this fn - so on the exact frame
            // pausing begins, the plane's position has already caught up to
            // the latest physics tick by the time we get here, but
            // camera_control (below) hasn't run yet for this frame. Skipping
            // it outright (like every following frame, once already paused)
            // would leave the camera's last-written position one frame
            // stale relative to that just-applied plane position - reading
            // as the plane hopping slightly out ahead of the camera right as
            // it pauses. Running it this one extra time syncs the camera to
            // the exact same fresh state before anything actually freezes.
            if just_paused {
                self.camera_control(scene, app, app.time.delta_time);
            }

            // Freeze the entire gameplay simulation - game_time (and by
            // extension every animation track sampled off it), flight/
            // afterburner animation, camera follow-cam and the "change_camera"
            // switch, HUD text - none of it should keep advancing behind the
            // pause menu. The menu's own buttons are handled independently of
            // this fn (see App::fire_ui_click_handlers), so returning early
            // here doesn't block Resume/Restart/Settings from working.
            //
            // UI vertex buffers *and* hover/click hit-testing only run when
            // the UI is marked dirty (see UiNode::on_click's own doc comment)
            // - the HUD never needed this (nothing in it is hoverable/
            // clickable), but the pause menu's buttons are, so without this
            // it'd render whatever was on screen the instant it opened
            // (usually nothing yet, since it was still inactive that frame)
            // and never register a hover or a click again. Same idiom
            // main_menu::scene::GameLogic::update already uses unconditionally
            // every frame, scoped here to just while paused instead.
            app.ui.has_changed = true;
            return;
        }

        self.game_time += app.time.delta_time as f64;

        if input::is_action_just_pressed("test") {
            self.subtitle_data.add_text("SKIBIDI DAM DAM DAM YES YES", 3000, app);
        }

        // While a CameraTrack::LookAt cinematic is driving the camera (see
        // level_planning.ron's camera_tracks), the player doesn't get flight
        // controls - skipping Plane::update leaves `controls` at whatever they
        // already are (neutral, since nothing else ever touches them). The plane
        // itself is expected to be driven by a matching Object3DTrack targeting
        // "player" for the same window (see level_planning.ron) rather than
        // physics - see the pause/resume handling right below.
        let cinematic_active = match &self.event_system {
            Some(event_system) => event_system.is_cinematic_camera_active(self.game_time),
            None => false,
        };

        let cinematic_just_ended = !cinematic_active && self.was_cinematic_active;

        // Rising/falling edge of the cinematic window - pause physics for its
        // duration (so the rigidbody's own simulated position doesn't drift away
        // from wherever the scripted Object3DTrack is putting the plane, which
        // would otherwise cause a visible snap the instant physics starts
        // driving the render transform again), then on the way out, teleport the
        // rigidbody to match exactly where the script left the plane before
        // un-pausing - see PhysicsCommand::SetTransform's own doc comment.
        if let Some(physics_command_tx) = physics_command_tx {
            if cinematic_active && !self.was_cinematic_active {
                let _ = physics_command_tx.send(PhysicsCommand::TogglePause);
            } else if cinematic_just_ended {
                if let Some(player) = scene.content.renderizable_instances.get("player") {
                    let transform = player.instance.transform;
                    let _ = physics_command_tx.send(PhysicsCommand::SetTransform {
                        name: "player".to_owned(),
                        translation: transform.position,
                        rotation: transform.rotation.into_inner(),
                        // Matches data.ron's own player initial_velocity - a
                        // reasonable cruise speed to resume normal flight at
                        // regardless of exactly what the cinematic's own path was.
                        linvel: Vector3::new(0.0, 0.0, 200.4),
                    });
                }
                let _ = physics_command_tx.send(PhysicsCommand::TogglePause);
            }
        }

        // Same falling edge, for the camera - see CinematicReturnBlend's own doc
        // comment for why this snapshot (rather than CameraHandler::transition_to)
        // is what actually fixes the hard cut. Has to happen here, before
        // camera_control runs later this frame - "main" still holds whatever the
        // Shot track last wrote to it *last* frame at this point.
        if cinematic_just_ended {
            let cam = scene.cameras.active();
            self.camera_data.cinematic_return_blend = Some(CinematicReturnBlend {
                elapsed: 0.0,
                duration: 0.6,
                from_position: cam.camera.position(),
                from_look_at: cam.camera.position() + cam.camera.calc_forward_direction() * 100.0,
                from_fov: cam.projection.fovy,
            });
        }

        self.was_cinematic_active = cinematic_active;

        // "player"'s own control-surface/afterburner mesh animation runs
        // automatically every frame via the generic Behavior tick (see
        // Plane::update) - what's left here is exactly what a Behavior
        // structurally can't reach on its own: whether input should be
        // locked out this frame (Plane has no visibility into
        // cinematic_active/input_lock_end), sending this frame's controls to
        // the physics thread, and feeding this frame's physics results back
        // in (speedometer/G-meter/wheel meshes - see
        // Plane::apply_physics_feedback's own doc comment for why that has
        // to happen from here rather than from Plane's own Behavior::update).
        if let Some(node) = scene.content.nodes.get_mut("player") {
            let scale = node.get_property::<Transform3D>().map(|transform| transform.scale).unwrap_or(Vector3::new(1.0, 1.0, 1.0));
            let model_ref = node.get_property::<Model>().map(|model| model.model_ref.clone());

            if let Some(plane) = node.get_behavior_mut::<Plane>() {
                plane.input_locked = cinematic_active || input_lock_end.is_some();

                if let Some(plane_control_tx) = plane_control_tx {
                    let _ = plane_control_tx.send(plane.controls.clone());
                }

                if let (Some(model_ref), Some(physics_message)) = (&model_ref, physics_data.get("player")) {
                    if let Some(model_instance) = app.game_models.get_mut(model_ref) {
                        plane.apply_physics_feedback(&mut model_instance.model, physics_message, self.gravity, scale, &app.renderer.queue, app.time.delta_time);
                    }
                }
            }
        }

        if let Some(event_system) = &mut self.event_system {
            event_system.handle_events(self.game_time, app, &mut self.subtitle_data);
        }
        self.subtitle_data.update(app);
        self.camera_control(scene, app, app.time.delta_time);
        self.ui_control(scene, app);
        Self::render_physics_debug(scene, app, physics_data);

        // Applied last so a running animation track is the final word for this
        // frame - e.g. a Position/Fov track targeting the "main" camera visibly
        // overrides camera_control's flight follow-cam for as long as it runs
        // (see CameraTrack's own doc comment for the tradeoffs of that).
        if let Some(event_system) = &self.event_system {
            event_system.apply_tracks(self.game_time, scene, app);
        }

    }

    // Debug-only wireframes for physics colliders - the player's own
    // (dynamic, moving) ones read back from that frame's physics results,
    // static bodies (ground, world, fellow_aviator, ...) straight from
    // scene.content.physics_bodies (see the block below its own comment for
    // why those two need different sources). Doesn't touch `self` at all -
    // an associated fn rather than a method, since nothing here is GameLogic
    // state, just a render step gated on app.render_physics.visible.
    fn render_physics_debug(scene: &mut Scene, app: &mut App, physics_data: &HashMap<String, RenderMessage>) {
        if app.render_physics.visible {
            if let Some(physics_data_renderizable) = physics_data.get("player") {
                if let Some(MetadataType::Colliders(colliders)) = physics_data_renderizable.metadata.get("colliders") {
                    let plane = scene.content.renderizable_instances.get("player").unwrap();
                    let pos = &plane.instance.transform.position;
                    let rot = &plane.instance.transform.rotation;
                    let color = [0.0, 1.0, 1.0];
                    for col in colliders {
                        let he = &col.half_extents;
                        let off = &col.local_offset;
                        let corners_local = [
                            Vector3::new(-he.x, -he.y, -he.z),
                            Vector3::new( he.x, -he.y, -he.z),
                            Vector3::new( he.x,  he.y, -he.z),
                            Vector3::new(-he.x,  he.y, -he.z),
                            Vector3::new(-he.x, -he.y,  he.z),
                            Vector3::new( he.x, -he.y,  he.z),
                            Vector3::new( he.x,  he.y,  he.z),
                            Vector3::new(-he.x,  he.y,  he.z),
                        ];
                        let cw: Vec<Vector3<f32>> = corners_local.iter()
                            .map(|c| pos + rot * (off + c))
                            .collect();
                        let edges = [
                            (0,1),(1,2),(2,3),(3,0),
                            (4,5),(5,6),(6,7),(7,4),
                            (0,4),(1,5),(2,6),(3,7),
                        ];
                        for (a, b) in edges {
                            app.render_physics.renderizable_lines.push([
                                ManualVertex { position: [cw[a].x, cw[a].y, cw[a].z], color },
                                ManualVertex { position: [cw[b].x, cw[b].y, cw[b].z], color },
                            ]);
                        }
                    }
                }

                // Render wing debug lines (axes + lift force) using visual transform
                if let Some(MetadataType::Wings(wings)) = physics_data_renderizable.metadata.get("wings") {
                    let plane = scene.content.renderizable_instances.get("player").unwrap();
                    let pos = &plane.instance.transform.position;
                    let rot = &plane.instance.transform.rotation;
                    let axis_len = 0.2;

                    for w in wings {
                        let wpc = pos + rot * w.pressure_center;
                        // X axis (red)
                        let x_end = wpc + rot * Vector3::new(axis_len, 0.0, 0.0);
                        app.render_physics.renderizable_lines.push([
                            ManualVertex { position: [wpc.x, wpc.y, wpc.z], color: [1.0, 0.0, 0.0] },
                            ManualVertex { position: [x_end.x, x_end.y, x_end.z], color: [1.0, 0.0, 0.0] },
                        ]);
                        // Y axis (green)
                        let y_end = wpc + rot * Vector3::new(0.0, axis_len, 0.0);
                        app.render_physics.renderizable_lines.push([
                            ManualVertex { position: [wpc.x, wpc.y, wpc.z], color: [0.0, 1.0, 0.0] },
                            ManualVertex { position: [y_end.x, y_end.y, y_end.z], color: [0.0, 1.0, 0.0] },
                        ]);
                        // Z axis (blue)
                        let z_end = wpc + rot * Vector3::new(0.0, 0.0, axis_len);
                        app.render_physics.renderizable_lines.push([
                            ManualVertex { position: [wpc.x, wpc.y, wpc.z], color: [0.0, 0.0, 1.0] },
                            ManualVertex { position: [z_end.x, z_end.y, z_end.z], color: [0.0, 0.0, 1.0] },
                        ]);
                        // Lift force (yellow/orange)
                        let lift_end = wpc + w.last_lift_force * 0.01;
                        app.render_physics.renderizable_lines.push([
                            ManualVertex { position: [wpc.x, wpc.y, wpc.z], color: [1.0, 0.8, 0.0] },
                            ManualVertex { position: [lift_end.x, lift_end.y, lift_end.z], color: [1.0, 0.5, 0.0] },
                        ]);
                    }
                }

                // Render suspension debug lines using visual transform
                if let Some(MetadataType::Suspensions(suspensions)) = physics_data_renderizable.metadata.get("suspensions") {
                    let plane = scene.content.renderizable_instances.get("player").unwrap();
                    let pos = &plane.instance.transform.position;
                    let rot = &plane.instance.transform.rotation;

                    for s in suspensions {
                        let vis_origin = pos + rot * s.local_origin;
                        let vis_wheel = pos + rot * s.local_wheel;
                        app.render_physics.renderizable_lines.push([
                            ManualVertex { position: [vis_origin.x, vis_origin.y, vis_origin.z], color: [0.0, 1.0, 0.0] },
                            ManualVertex { position: [vis_wheel.x, vis_wheel.y, vis_wheel.z], color: [0.0, 1.0, 0.0] },
                        ]);
                    }
                }
            }

            // Static bodies (ground, world, fellow_aviator, ...) never move,
            // so their debug wireframe can be built straight from
            // scene.content.physics_bodies (already-resolved collider data,
            // see engine::scene_manager::physics_bridge) instead of round-
            // tripping through the physics thread the way the player's own
            // dynamic colliders above need to - there's no live rigidbody
            // transform to catch up with when it's never going to move.
            let color = [1.0, 0.6, 0.0];
            for body in &scene.content.physics_bodies {
                if !body.physics.rigidbody.is_static {
                    continue;
                }
                for collider in &body.physics.colliders {
                    match collider {
                        ColliderType::Cuboid { half_extents, position } => {
                            let center = body.position + Vector3::new(position.0, position.1, position.2);
                            let he = Vector3::new(half_extents.0, half_extents.1, half_extents.2);
                            let corners: Vec<Vector3<f32>> = [
                                Vector3::new(-he.x, -he.y, -he.z), Vector3::new( he.x, -he.y, -he.z),
                                Vector3::new( he.x,  he.y, -he.z), Vector3::new(-he.x,  he.y, -he.z),
                                Vector3::new(-he.x, -he.y,  he.z), Vector3::new( he.x, -he.y,  he.z),
                                Vector3::new( he.x,  he.y,  he.z), Vector3::new(-he.x,  he.y,  he.z),
                            ].iter().map(|c| center + c).collect();
                            let edges = [(0,1),(1,2),(2,3),(3,0),(4,5),(5,6),(6,7),(7,4),(0,4),(1,5),(2,6),(3,7)];
                            for (a, b) in edges {
                                app.render_physics.renderizable_lines.push([
                                    ManualVertex { position: [corners[a].x, corners[a].y, corners[a].z], color },
                                    ManualVertex { position: [corners[b].x, corners[b].y, corners[b].z], color },
                                ]);
                            }
                        },
                        ColliderType::Trimesh { vertices, indices } => {
                            for triangle in indices {
                                let p: Vec<Vector3<f32>> = triangle.iter().map(|&i| body.position + vertices[i as usize]).collect();
                                for (a, b) in [(0, 1), (1, 2), (2, 0)] {
                                    app.render_physics.renderizable_lines.push([
                                        ManualVertex { position: [p[a].x, p[a].y, p[a].z], color },
                                        ManualVertex { position: [p[b].x, p[b].y, p[b].z], color },
                                    ]);
                                }
                            }
                        },
                        _ => {},
                    }
                }
            }
        }
    }

    fn camera_control(&mut self, scene: &mut Scene, _app: &mut App, delta_time: f32) {
        if let Some(player) = scene.content.renderizable_instances.get_mut("player") {
            // Calculate target camera position and look-at point
            let (target_position, target_look_at, target_up) = match self.camera_data.camera_state {
                CameraState::Normal => {
                    let active = scene.cameras.active_mut();
                    active.projection.znear = 0.1;
                    active.projection.fovy = NORMAL_CAMERA_FOV;
                    let target_pos = player.instance.transform.position + (player.instance.transform.rotation * Vector3::new(0.0, 8.0, -50.0));
                    let look_at = player.instance.transform.position + (player.instance.transform.rotation * Vector3::new(0.0, 0.0, 100.0));
                    (target_pos, look_at, player.instance.transform.rotation * *Vector3::y_axis())
                },
                CameraState::Cockpit => {
                    let (base_pos, default_fov) = if let Some(cameras) = &player.instance.metadata.cameras {
                        if let Some(cam) = cameras.get("cockpit") {
                            (player.instance.transform.rotation * cam.position, cam.fov)
                        } else {
                            (player.instance.transform.rotation * Vector3::new(0.0, 0.2, 1.3), 70.0)
                        }
                    } else {
                        (player.instance.transform.rotation * Vector3::new(0.0, 0.2, 1.3), 70.0)
                    };
                    scene.cameras.active_mut().projection.znear = 0.01;

                    // Mouse wheel adjusts target FOV
                    let scroll = input::mouse_scroll_y();
                    if scroll != 0.0 {
                        self.camera_data.cockpit_target_fov = (self.camera_data.cockpit_target_fov - scroll * 5.0).clamp(20.0, 120.0);
                    }
                    // Lerp current FOV toward target
                    self.camera_data.cockpit_current_fov = lerp(self.camera_data.cockpit_current_fov, self.camera_data.cockpit_target_fov, delta_time * 8.0);
                    scene.cameras.active_mut().projection.fovy = self.camera_data.cockpit_current_fov;

                    let max_yaw: f32 = 170.0;
                    let max_pitch: f32 = 70.0;
                    let sens = input::mouse_sensitivity();

                    // Update yaw/pitch from relative mouse, clamp immediately so no over-accumulation
                    self.camera_data.cockpit_yaw = (self.camera_data.cockpit_yaw - input::mouse_rel_x() as f32 * sens.0).clamp(-max_yaw, max_yaw);
                    self.camera_data.cockpit_pitch = (self.camera_data.cockpit_pitch + input::mouse_rel_y() as f32 * sens.1).clamp(-max_pitch, max_pitch);

                    let yaw = self.camera_data.cockpit_yaw;
                    let pitch = self.camera_data.cockpit_pitch;

                    // Target head shift (cubic easing near the limit)
                    let t = (yaw / max_yaw).clamp(-1.0, 1.0);
                    let target_head_shift = 0.03 * t * t * t.signum();
                    self.camera_data.cockpit_current_head_shift = lerp(self.camera_data.cockpit_current_head_shift, target_head_shift, delta_time * 8.0);
                    let head_offset = player.instance.transform.rotation * Vector3::new(self.camera_data.cockpit_current_head_shift, 0.0, 0.0);

                    let target_pos = player.instance.transform.position + base_pos + head_offset;

                    // Target rotation from mouse
                    let rotation_y = UnitQuaternion::from_axis_angle(&Vector3::y_axis(), yaw.to_radians());
                    let rotation_x = UnitQuaternion::from_axis_angle(&Vector3::x_axis(), pitch.to_radians());
                    let target_rotation = rotation_y * rotation_x;

                    // Lerp the rotation for smooth feel
                    self.camera_data.cockpit_current_rotation = UnitQuaternion::new_normalize(lerp_quaternion(
                        self.camera_data.cockpit_current_rotation.into_inner(),
                        target_rotation.into_inner(),
                        delta_time * 12.0,
                    ));

                    let look_dir = player.instance.transform.rotation * self.camera_data.cockpit_current_rotation * Vector3::new(0.0, 0.0, 100.0);
                    let look_at = target_pos + look_dir;
                    (target_pos, look_at, player.instance.transform.rotation * *Vector3::y_axis())
                },
                CameraState::Cinematic => {
                    let (target_pos, fov) = if let Some(cameras) = &player.instance.metadata.cameras {
                        if let Some(cam) = cameras.get("cinematic") {
                            (player.instance.transform.position + (player.instance.transform.rotation * cam.position), cam.fov)
                        } else {
                            (player.instance.transform.position + (player.instance.transform.rotation * Vector3::new(-1.0, 3.0, -1.0)), 60.0)
                        }
                    } else {
                        (player.instance.transform.position + (player.instance.transform.rotation * Vector3::new(-1.0, 3.0, -1.0)), 60.0)
                    };
                    scene.cameras.active_mut().projection.fovy = fov;
                    let look_at = player.instance.transform.position + (player.instance.transform.rotation * Vector3::new(30.0, 0.0, 100.0));
                    (target_pos, look_at, player.instance.transform.rotation * *Vector3::y_axis())
                },
                CameraState::Frontal => {
                    let (target_pos, fov) = if let Some(cameras) = &player.instance.metadata.cameras {
                        if let Some(cam) = cameras.get("frontal") {
                            (player.instance.transform.position + (player.instance.transform.rotation * cam.position), cam.fov)
                        } else {
                            (player.instance.transform.position + (player.instance.transform.rotation * Vector3::new(0.0, 2.0, 3.0)), 60.0)
                        }
                    } else {
                        (player.instance.transform.position + (player.instance.transform.rotation * Vector3::new(0.0, 2.0, 3.0)), 60.0)
                    };
                    scene.cameras.active_mut().projection.fovy = fov;
                    let look_at = player.instance.transform.position;
                    (target_pos, look_at, player.instance.transform.rotation * *Vector3::y_axis())
                },
                CameraState::Free => {
                    scene.cameras.active_mut().projection.znear = 0.1;

                    // Mouse wheel adjusts target FOV
                    let scroll = input::mouse_scroll_y();
                    if scroll != 0.0 {
                        self.camera_data.free_target_fov = (self.camera_data.free_target_fov - scroll * 5.0).clamp(20.0, 120.0);
                    }
                    self.camera_data.free_current_fov = lerp(self.camera_data.free_current_fov, self.camera_data.free_target_fov, delta_time * 8.0);
                    scene.cameras.active_mut().projection.fovy = self.camera_data.free_current_fov;

                    let sens = input::mouse_sensitivity();
                    self.camera_data.free_yaw = self.camera_data.free_yaw - input::mouse_rel_x() as f32 * sens.0;
                    self.camera_data.free_pitch = (self.camera_data.free_pitch + input::mouse_rel_y() as f32 * sens.1).clamp(-89.0, 89.0);

                    let rotation_y = UnitQuaternion::from_axis_angle(&Vector3::y_axis(), self.camera_data.free_yaw.to_radians());
                    let rotation_x = UnitQuaternion::from_axis_angle(&Vector3::x_axis(), self.camera_data.free_pitch.to_radians());
                    let target_rotation = rotation_y * rotation_x;

                    self.camera_data.free_current_rotation = UnitQuaternion::new_normalize(lerp_quaternion(
                        self.camera_data.free_current_rotation.into_inner(),
                        target_rotation.into_inner(),
                        delta_time * 12.0,
                    ));

                    let target_pos = player.instance.transform.position + (self.camera_data.free_current_rotation * Vector3::new(0.0, 0.0, -45.0));
                    let look_at = player.instance.transform.position;
                    (target_pos, look_at, *Vector3::y_axis())
                },
            };

            // Debug mode overlay: move camera offset with arrow keys / W / S
            let final_position = if self.camera_data.debug_mode_active {
                let speed = 2.0 * delta_time;
                let mut changed = false;

                if input::is_action_pressed("throttle_up") {
                    self.camera_data.debug_offset.z += speed;
                    changed = true;
                }
                if input::is_action_pressed("throttle_down") {
                    self.camera_data.debug_offset.z -= speed;
                    changed = true;
                }
                if input::is_action_pressed("up_wheel") {
                    self.camera_data.debug_offset.x += speed;
                    changed = true;
                }
                if input::is_action_pressed("down_wheel") {
                    self.camera_data.debug_offset.x -= speed;
                    changed = true;
                }
                if input::is_action_pressed("pitch_up") {
                    self.camera_data.debug_offset.y += speed;
                    changed = true;
                }
                if input::is_action_pressed("pitch_down") {
                    self.camera_data.debug_offset.y -= speed;
                    changed = true;
                }

                if changed {
                    println!("Camera offset: Vector3::new({:.3}, {:.3}, {:.3})",
                        self.camera_data.debug_offset.x,
                        self.camera_data.debug_offset.y,
                        self.camera_data.debug_offset.z);
                }

                // Apply debug offset relative to the plane
                player.instance.transform.position + (player.instance.transform.rotation * self.camera_data.debug_offset)
            } else {
                target_position
            };

            // Apply camera position directly (no interpolation to match object
            // movement) - except right after a CameraTrack::Shot ends, where
            // cinematic_return_blend (see its own doc comment) eases from the
            // Shot's last pose into whatever's computed above instead of cutting.
            let target_position: Point3<f32> = final_position.into();
            let target_look_at: Point3<f32> = target_look_at.into();
            let target_fov = scene.cameras.active().projection.fovy;

            let (blended_position, blended_look_at, blended_fov) = match &mut self.camera_data.cinematic_return_blend {
                Some(blend) => {
                    blend.elapsed += delta_time;
                    let t = smoothstep((blend.elapsed / blend.duration).clamp(0.0, 1.0));
                    let blended = (
                        lerp_point3(blend.from_position, target_position, t),
                        lerp_point3(blend.from_look_at, target_look_at, t),
                        lerp(blend.from_fov, target_fov, t),
                    );
                    if blend.elapsed >= blend.duration {
                        self.camera_data.cinematic_return_blend = None;
                    }
                    blended
                }
                None => (target_position, target_look_at, target_fov),
            };

            let active = scene.cameras.active_mut();
            active.camera.set_position(blended_position);
            active.camera.look_at(blended_look_at);
            active.camera.up = target_up;
            active.projection.fovy = blended_fov;
        }
        // self.calculate_lockable(app);
        if input::is_action_just_pressed("change_camera") {
            self.next_camera();
        }
        // F5 - offsets whichever named camera is already active (see
        // final_position above).
        if input::is_action_just_pressed("toggle_camera_offset_debug") {
            self.camera_data.debug_mode_active = !self.camera_data.debug_mode_active;
            println!("Camera debug mode: {}", if self.camera_data.debug_mode_active { "ON" } else { "OFF" });
        }
    }

    // i do this so my "ui_controller" is smaller
    fn update_text_label(&mut self, ui_tagged_elements: &mut HashMap<String, UiNode>, tag: &str, text: &str, font_system: &mut FontSystem) {
        if let Some(node) = ui_tagged_elements.get_mut(tag) {
            match &mut node.content {
                UiNodeContent::Text(label) => {
                    label.set_text(font_system, &text, true);
                },
                _ => {}
            }
        }
    }

    fn format_duration(seconds: f64) -> String {
        let duration = Duration::from_secs_f64(seconds);

        let total_millis = duration.as_millis() as u64;
        let hours = total_millis / 3_600_000; // 1 hour = 3,600,000 milliseconds
        let minutes = (total_millis % 3_600_000) / 60_000; // 1 minute = 60,000 milliseconds
        let seconds = (total_millis % 60_000) / 1_000; // 1 second = 1,000 milliseconds
        let milliseconds = total_millis % 1_000; // Remaining milliseconds
        // Format as hh:mm:ss:milmilmil
        format!("{:02}:{:02}:{:02}:{:03}", hours, minutes, seconds, milliseconds)
    }

    fn ui_control(&mut self, scene: &mut Scene, app: &mut App) {
        // How much real time actually passed since the last throttled tick - NOT
        // app.time.delta_time (the current frame's own delta). At framerates above
        // the throttle's own rate (ui_update_interval, ~120Hz), most frames fail the
        // gate below and this fn simply doesn't run for them - passing only the
        // triggering frame's own (tiny) delta_time would silently drop every skipped
        // frame's worth of real time from anything here that accumulates elapsed
        // seconds (blinking_alert), making it run far slower than real time the
        // faster the game renders.
        // Capped: last_ui_update is a single cross-scene timestamp that's never
        // reset on a scene switch, so the very first tick after this scene starts
        // (or after any other long gap - a paused/backgrounded window, a debugger
        // breakpoint) would otherwise report however long it's been since the
        // *previous* scene last ran ui_control, potentially several seconds. 100ms
        // is comfortably above ui_update_interval (so normal per-tick values are
        // never affected), but caps a stale gap down to something that just reads
        // as a slightly-longer-than-usual frame.
        //
        // The mission-intro sequence (backdrop/title/HUD fades) used to be driven
        // from here too, but now runs off `self.game_time` via the `ui_tracks` in
        // `level_planning.ron` (see `EventSystem::apply_tracks`, called at the end
        // of `GameLogic::update`) - `game_time` accumulates every real frame's own
        // `delta_time` (already re-baselined after this scene's blocking asset
        // load, see `App::run`'s own comment on that), not this throttled/capped
        // `ui_elapsed`, so it doesn't need the same clamping.
        let ui_elapsed = app.throttling.last_ui_update.elapsed().min(Duration::from_millis(100));
        if ui_elapsed >= app.throttling.ui_update_interval {
            let ui_delta_time = ui_elapsed.as_secs_f32();

            // A Behavior isn't reachable from here as `&mut Scene` - only
            // `SceneBehaviour` methods get that - so this reads the "player"
            // node's Plane once up front (plain Copy values, so nothing
            // borrowed from `scene` needs to stay alive afterward) instead of
            // fetching it again at every label below.
            let (throttle, g_meter, altimeter, speedometer, previous_velocity, stall) = scene.content.nodes.get("player")
                .and_then(|node| node.get_behavior::<Plane>())
                .map(|plane| (plane.controls.throttle, plane.flight_data.g_meter, plane.flight_data.altimeter, plane.flight_data.speedometer, plane.previous_velocity, plane.stall))
                .unwrap_or((0.0, 0.0, 0.0, 0.0, None, false));

            if let Some(label) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "game_ui/data_box/framerate").and_then(|n| n.as_label_mut()) {
                label.set_text(&mut app.ui.text.font_system, &format!("FPS: {}", app.time.get_fps()), true);
            }

            if let Some(label) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "game_ui/data_box/g").and_then(|n| n.as_label_mut()) {
                label.set_text(&mut app.ui.text.font_system, &format!("G: {:.0}", g_meter), true);
            }

            if let Some(label) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "game_ui/data_box/timer").and_then(|n| n.as_label_mut()) {
                label.set_text(&mut app.ui.text.font_system, &Self::format_duration(self.game_time), true);
            }

            if let Some(label) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "game_ui/data_box/power").and_then(|n| n.as_label_mut()) {
                label.set_text(&mut app.ui.text.font_system, &format!("Power: {}%", (throttle * 100.0).round()), true);
            }

            if let Some(label) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "altitude").and_then(|n| n.as_label_mut()) {
                label.set_text(&mut app.ui.text.font_system, &format!("ALT: {}", altimeter), true);
            }

            // F3 debug view - two separate single-line labels (see
            // play::ui::build_debug_panel's own doc comment on why not one
            // multi-line label), each updated the same way as every other
            // label in this block.
            if let Some(label) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "debug_panel/stats_box/fps").and_then(|n| n.as_label_mut()) {
                label.set_text(&mut app.ui.text.font_system, &format!("{:.0} FPS", app.time.get_fps()), true);
            }
            if let Some(pos) = scene.content.renderizable_instances.get("player").map(|i| i.instance.transform.position) {
                if let Some(label) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "debug_panel/stats_box/position").and_then(|n| n.as_label_mut()) {
                    label.set_text(&mut app.ui.text.font_system, &format!("Player position: ({:.1}, {:.1}, {:.1})", pos.x, pos.y, pos.z), true);
                }
            }

            if let Some(label) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "game_ui/speed").and_then(|n| n.as_label_mut()) {
                label.set_text(&mut app.ui.text.font_system, &format!("SPD: {:.0}", speedometer), true);
            }

            let rotation = Self::map_to_range(scene.cameras.active().camera.yaw().into(), -PI as f64, PI  as f64, 0.0, 360.0).round();
            
            let text_compass = if rotation >= 355.0 || rotation <= 5.0 {
                "N".to_owned()
            } else if rotation >= 175.0 && rotation <= 185.0{
                "S".to_owned()
            } else if rotation >= 85.0 && rotation <= 95.0 {
                "E".to_owned()
            } else if rotation >= 265.0 && rotation <= 275.0 {
                "O".to_owned()
            } else {
                rotation.round().to_string() + "°"
            };

            if let Some(label) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "game_ui/compass").and_then(|n| n.as_label_mut()) {
                label.set_text(&mut app.ui.text.font_system, &text_compass, true);
            }

            // Update velocity vector marker position
            if let Some(velocity) = &previous_velocity {
                if velocity.magnitude() > 0.1 {
                    if let Some(player) = scene.content.renderizable_instances.get("player") {
                        let pos = player.instance.transform.position;
                        let vel_point = Point3::from(pos + velocity.normalize() * 100.0);
                        if let Some(screen_pos) = app.camera_resources.world_to_screen(scene.cameras.active(), vel_point, app.window_manager.size.width, app.window_manager.size.height) {
                            if let Some(marker) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "velocity_marker") {
                                marker.transform.x = screen_pos.x as f32 - marker.transform.width / 2.0;
                                marker.transform.y = screen_pos.y as f32 - marker.transform.height / 2.0;
                                marker.transform.rect.left = marker.transform.x;
                                marker.transform.rect.top = marker.transform.y;
                                marker.transform.rect.right = marker.transform.x + marker.transform.width;
                                marker.transform.rect.bottom = marker.transform.y + marker.transform.height;
                                marker.update_style(|s| s.set_text_color(UiColor::Rgb(0, 255, 75)));
                            }
                        } else {
                            // Off screen — hide marker
                            if let Some(marker) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "velocity_marker") {
                                marker.update_style(|s| s.set_text_color(UiColor::Rgba(0, 255, 75, 0)));
                            }
                        }
                    }
                }
            }

            if let Some(altitude_alert) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "altitude_alert") {
                self.blinking_alert("altitude".to_owned(), altitude_alert, altimeter < 1000.0, ui_delta_time);
            }

            if let Some(stall_alert) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "stall_alert") {
                self.blinking_alert("stall".to_owned(), stall_alert, stall, ui_delta_time);
            }

            app.ui.has_changed = true; // Mark UI as changed so it gets processed
            app.throttling.last_ui_update = Instant::now();
        }
    }

    fn blinking_alert(&mut self, blinking_alert: String ,blinkable: &mut UiNode, condition: bool, delta_time: f32) {
        let blinking_alert = self.blinking_alerts.get_mut(&blinking_alert).unwrap();

        blinking_alert.time_alert += delta_time;
        if condition {
            if blinking_alert.alert_state == false {
                if blinking_alert.time_alert > 0.5 {
                    blinking_alert.time_alert = 0.0;
                    blinking_alert.alert_state = true;
                }
            } else {
                if blinking_alert.time_alert > 0.5 {
                    blinking_alert.time_alert = 0.0;
                    blinking_alert.alert_state = false;
                }
            }
        } else {
            blinking_alert.time_alert = 0.0;
            blinking_alert.alert_state = false
        }

        if matches!(&blinkable.content, UiNodeContent::Text(_)) {
            if blinking_alert.alert_state {
                blinkable.update_style(|s| s.set_border_color(UiColor::Rgb(255, 0, 0)).set_text_color(UiColor::Rgb(255, 0, 0)));
            } else {
                blinkable.update_style(|s| s.set_border_color(UiColor::TRANSPARENT).set_text_color(UiColor::TRANSPARENT));
            }
        }

    }

    fn map_to_range(x: f64, in_min: f64, in_max: f64, out_min: f64, out_max: f64) -> f64 {
        (x - in_min) * (out_max - out_min) / (in_max - in_min) + out_min
    }

    fn next_camera(&mut self) {
        match self.camera_data.camera_state {
            CameraState::Normal => {
                self.camera_data.camera_state = CameraState::Free;
            },
            CameraState::Cockpit => self.camera_data.camera_state = CameraState::Cinematic,
            CameraState::Cinematic => self.camera_data.camera_state = CameraState::Frontal,
            CameraState::Frontal => self.camera_data.camera_state = CameraState::Normal,
            CameraState::Free => self.camera_data.camera_state = CameraState::Cockpit,
        }
    }
}

impl SceneBehaviour for GameLogic {
    fn update(&mut self, scene: &mut Scene, app: &mut App, ctx: &mut FrameContext) {
        self.update(scene, app, ctx.plane_control_tx, ctx.physics_command_tx, ctx.physics_data);
    }

    fn fixed_update(&self, scene: &Scene, app: &App) -> Option<(Vec<PhysicsObjectDef>, Box<dyn PhysicsTick + Send>)> {
        let _ = app;
        // Nodes spawned in `new` (see `spawn_world`) already collected one
        // PhysicsObjectDef per Physics-carrying node onto scene.content -
        // just hand that straight to the physics thread.
        Some((scene.content.physics_bodies.clone(), Box::new(PlanePhysicsLogic::new())))
    }
}
