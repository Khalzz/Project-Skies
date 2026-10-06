use std::{collections::HashMap, f32::consts::PI, time::{Duration, Instant}};

use glyphon::FontSystem;
use nalgebra::{vector, Point3, UnitQuaternion, Vector3};
use crate::{app::App, engine::{audio::subtitles::Subtitle, input::input, physics::physics_handler::{PhysicsCommand, RenderMessage}, primitive::manual_vertex::ManualVertex, rendering::{enviroment::environment::Environment, ui::ui::Ui}, scene_manager::{node::Node, properties::{CameraLens, Model, Transform3D}, scene::{FrameContext, LoadReporter, Scene, SceneBehaviour}}, ui::{color::UiColor, ui_node::{UiNode, UiNodeContent}, ui_transform::{Anchor, Orientation, PositionValue, SizeValue}}}};
use crate::engine::game_nodes::game_object::{Camera as GameObjectCamera, Cameras, ColliderType, Lighting, Physics as PhysicsProperty, RigidBodyData};
use crate::game::scenes::main_menu::ui as main_menu_ui;
use super::flight_editor::{EditorRequest, NodeKind, Playback, SceneNodeInfo, Telemetry, TimelineRequest};
use super::map_nodes::MapNodes;
use super::animations::{self, Animations};
use super::map::AnimAction;
use crate::engine::scene_manager::properties::degrees_from_rotation;
use crate::engine::scene_manager::properties::{Parent, Space, SpawnedLive};
use super::flight_manager::{cruise_speed, flight_manager, LiveAircraft};
use super::plane::autopilot::Speed;
use super::map::MapSpec;
use super::tactical_map::{self, contour_lines, TacticalMap, View as TacticalView};
use super::flight_editor::next_path_point;
use super::tactical_ui::{AircraftStatus, FlightPlan, PlanTarget, RosterCoalition, RosterEntry};
use super::plane::aircraft_spec::AircraftSpec;
use super::ai_defense::AiDefense;
use super::map::Difficulty;
use super::weapon::countermeasures::{Countermeasures, Release};
use super::weapon::heat;
use super::weapon::weapon_spec::{Seeker, WeaponCategory, WeaponSpec};
use super::weapon::fire_control::{self, Contact, FireControl, OwnShip};
use super::weapon::missile::TargetTrack;
use super::weapon::missiles::Missiles;
use super::crash_effects::{Crash, CrashEffects};
use super::world_collision::WorldCollision;
use crate::engine::rendering::render_pipeline::blur_renderer::ScreenEffects;
use crate::engine::ui::components::label::text_width;
use crate::game::ui::HUD_FONT;
use crate::engine::rendering::thick_lines::{convex_mask, LineInstance};
use super::camera::head_motion::HeadInput;
use super::{camera::camera::{Camera, CameraState}, event_handling::EventSystem, plane::{messages::AircraftState, plane::Plane, quick_list}};
use std::sync::mpsc::Sender;
use crate::game::selected_level::SELECTED_LEVEL;
use crate::engine::rendering::render_pipeline::water_renderer::WakeSource;
use crate::engine::particles::ParticleEmitters;
use crate::game::ui::label;
use crate::game::scenes::play::ui as play_ui;
use crate::resources;
use crate::engine::tooling::debug_console;

pub struct BlinkingAlert {
    alert_state: bool,
    time_alert: f32
}

pub struct GameLogic {
    pub blinking_alerts: HashMap<String, BlinkingAlert>,
    pub gravity: Vector3<f32>,
    pub subtitle_data: Subtitle,
    pub start_time: Instant,
    pub event_system: Option<EventSystem>,
    pub game_time: f64,
    was_paused: bool,
    esc_hold_time: f32,
    is_dead: bool,
    // Whether the HUD is hidden because the pilot can't see - see
    // update_g_vision.
    hud_hidden_by_g: bool,
    // Seconds since the plane was wrecked - see check_crash.
    crash_time: f32,
    // The pilot was killed with the camera in the cockpit - the crash's
    // fade shows through to the wreck view (see update_g_vision).
    crash_seen_from_cockpit: bool,
    // Where each wreck in the sea went in, on the surface, and which way it
    // was heading - what the camera stays on (not the sinking wreck).
    went_in_at: HashMap<String, (Vector3<f32>, UnitQuaternion<f32>)>,
    // F8's heat view is on - see update_heat_view.
    heat_view: bool,
    // The kill feed's lines, oldest first - see update_kill_feed.
    kill_feed: Vec<KillFeedEntry>,
    // Every plane in the scene - see flight_manager.
    flight_manager: flight_manager,
    // Whether the flight editor (F10's editor view) was open last frame - see
    // update_flight_editor.
    flight_editor_was_open: bool,
    // The editor's map didn't validate last frame - its error is showing.
    flight_editor_map_invalid: bool,
    // The instance the camera follows (node id) - see update_camera_focus.
    // Empty: the player's.
    camera_focus: String,
    // The aircraft selected in the flight editor last frame - picking a new
    // one points the camera at it.
    editor_selection: Option<String>,
    // F10's tactical map - see update_tactical_map.
    tactical_map: TacticalMap,
    // The player's master mode, radar and seeker - see update_weapons.
    fire_control: FireControl,
    // Every missile in flight.
    missiles: Missiles,
    // Aircraft blowing up on the ground and splashing into the sea.
    crash_effects: CrashEffects,
    // Every weapon's data read so far, by id.
    weapon_specs: HashMap<String, WeaponSpec>,
    // Which side the player last fired from (+1 left, -1 right) - the next
    // goes from the other.
    last_fired_side: f32,
    // The weapons line on the HUD - see update_weapons.
    weapons_status: String,
    // The countermeasures line on the data box. The column is what's left
    // and the release setting - the HUD itself only flashes the setting
    // when it changes (see cms_cue).
    countermeasures_status: String,
    countermeasures_column: Option<(u32, u32, Release)>,
    // The setting just chosen, and the game time it was chosen - the HUD
    // blinks it, holds it, then fades it.
    cms_cue: Option<(String, f64)>,
    // Fuel has already crossed bingo, so the four-second BINGO cue doesn't
    // start again every frame. Cleared once the tanks are above bingo or empty.
    bingo_latched: bool,
    // Game time until which the WARN text reads BINGO.
    bingo_warn_until: Option<f64>,
    // Every flare and chaff in the air.
    countermeasures: Countermeasures,
    // The bots' reactions to missiles fired at them - see
    // update_ai_countermeasures.
    ai_defense: AiDefense,
    // With a lock: the range scale's numbers - see update_hud.
    range_cue: Option<RangeCue>,
    // The weapon picked from the list (a weapon id) - None: OFF, nothing
    // armed. See update_weapons.
    selected_weapon: Option<String>,
    // The HUD's weapon list as it should read: each entry's text, and
    // whether it's the selected one - see play_ui::build_weapon_list.
    weapon_list: Vec<(String, bool)>,
    // The land and runway, for missiles to fly into - see spawn_world.
    world_collision: WorldCollision,
    // The map's own nodes and every node's tracks, kept in the scene - see
    // play::map_nodes.
    map_nodes: MapNodes,
    // Every node's animations running (map.ron's animations) and the paths
    // followed meanwhile - see late_update.
    animations: Animations,
    // A running animation has the HUD hidden (as of last frame) - see
    // update_g_vision.
    animation_hides_hud: bool,
    // Seconds into the HUD power-on after a cinematic returns to the
    // cockpit. None once it's on for good - see tick_hud_boot.
    hud_boot: Option<f32>,
    // F10's editor camera keeping up with what F1-F4 picked: its node id
    // and where it was last frame - see update_editor_follow.
    editor_follow: Option<(String, Vector3<f32>)>,
    // The telemetry's last look at what the camera's on: its id and
    // velocity - for a missile's G.
    telemetry_last: Option<(String, Vector3<f32>)>,
    // The camera the view was last put on - see update_view.
    view_camera: String,
}

/// The player's view - a camera node driven by play::camera::camera::Camera.
pub const PLAYER_CAMERA: &str = "camera";
/// The camera animations' camera keys move (see play::animations).
pub const ANIMATION_CAMERA: &str = "animation_camera";
/// How far F10's editor camera stands off what it focuses on (m) - an
/// aircraft, anything else.
const EDITOR_FOCUS_AIRCRAFT: f32 = 60.0;
const EDITOR_FOCUS_NODE: f32 = 25.0;
const EDITOR_FOCUS_MISSILE: f32 = 15.0;

/// The level's folder - its level_planning.ron (see EventSystem) and
/// map.ron (see flight_manager::spawn_map).
const LEVEL_FOLDER: &str = "./assets/scenes/test_chamber";

pub struct PreparedPlayAssets {
    environment: resources::PreparedEnvironment,
    ground_trimesh: (Vec<Vector3<f32>>, Vec<[u32; 3]>),
    runway_trimesh: (Vec<Vector3<f32>>, Vec<[u32; 3]>),
}

impl GameLogic {
    /// Every model the level spawns (see spawn_world) - loaded under the
    /// loading screen instead of when each node spawns (see
    /// SceneManager::preload_models). The ocean is procedural, always
    /// resident, so it isn't listed.
    pub fn models(app: &App) -> Vec<resources::ModelSource> {
        ["Ground", "Runway", "F14"].iter()
            .filter_map(|name| app.model_sources.get(*name).cloned())
            .chain(super::flight_manager::models(LEVEL_FOLDER))
            .collect()
    }

    pub fn prepare(device: &wgpu::Device, queue: &wgpu::Queue, camera_bind_group_layout: &wgpu::BindGroupLayout, config: &wgpu::SurfaceConfiguration, reporter: &LoadReporter, environment: Environment) -> PreparedPlayAssets {
        reporter.step("Sky");
        let environment = resources::prepare_environment(device, queue, camera_bind_group_layout, config, environment);
        reporter.step("Ground collision");
        let ground_trimesh = resources::load_trimesh_geometry("ground/ground.glb").unwrap_or_else(|error| {
            eprintln!("play: ground trimesh from 'ground/ground.glb' couldn't be loaded: {error}");
            (vec![], vec![])
        });
        reporter.step("Runway collision");
        let runway_trimesh = resources::load_trimesh_geometry("Runway/Runway.glb").unwrap_or_else(|error| {
            eprintln!("play: runway trimesh from 'Runway/Runway.glb' couldn't be loaded: {error}");
            (vec![], vec![])
        });
        PreparedPlayAssets { environment, ground_trimesh, runway_trimesh }
    }

    pub fn finish(scene: &mut Scene, app: &mut App, prepared: PreparedPlayAssets) -> Self {
        scene.environment.skybox = prepared.environment.skybox;
        scene.environment.clear_color = prepared.environment.clear_color;
        app.scene_openned = Some(LEVEL_FOLDER.to_owned());

        let mut flight_manager = flight_manager::new();
        let mut world_collision = WorldCollision::default();
        Self::spawn_world(scene, app, &mut flight_manager, &mut world_collision, prepared.ground_trimesh, prepared.runway_trimesh);
        // The map's own nodes (cameras...) and its timeline's tracks.
        let mut map_nodes = MapNodes::default();
        map_nodes.sync(scene, app, flight_manager.map());
        // What animations' camera keys move - a camera of its own.
        scene.spawn_node(app,
            Node::new(ANIMATION_CAMERA)
                .add_property(Transform3D::default())
                .add_property(CameraLens::new(60.0))
        ).expect("play should only spawn the animation camera once");
        let mut animations = Animations::default();
        animations.start_level(flight_manager.map());

        app.window_manager.context.mouse().set_relative_mouse_mode(true);
        app.debug_mouse_free = false;

        // The player's view is the "camera" node (see spawn_world); the
        // level's animations cut to their own camera nodes (see update_view).
        scene.select_camera(PLAYER_CAMERA);

        app.is_paused = false;

        let screen_width = app.window_manager.size.width as f32;
        let screen_height = app.window_manager.size.height as f32;

        let mut game_ui = play_ui::build_game_ui(app);
        game_ui.resolve(screen_width, screen_height);
        app.ui.add_to_ui("game_ui".to_owned(), game_ui);
        for (key, text, font_size, width) in play_ui::HUD_READOUTS {
            let mut readout = play_ui::build_hud_readout(app, text, font_size, width);
            if key == "hud_corner_fuel" || key == "hud_radar_altitude" {
                readout = readout.set_align(glyphon::cosmic_text::Align::Left);
            }
            if key == "hud_altitude_warning" {
                readout = readout.set_font(&mut app.ui.text.font_system, play_ui::HUD_WARN_FACE);
            }
            readout.resolve(screen_width, screen_height);
            app.ui.add_to_ui(key.to_owned(), readout);
        }
        let pitch_keys = (0..play_ui::HUD_PITCH_LABELS).map(play_ui::hud_pitch_label_key);
        let heading_keys = (0..play_ui::HUD_HEADING_LABELS).map(play_ui::hud_heading_label_key);
        for key in pitch_keys.chain(heading_keys) {
            let mut small_label = play_ui::build_hud_small_label(app);
            small_label.resolve(screen_width, screen_height);
            app.ui.add_to_ui(key, small_label);
        }
        for index in 0..play_ui::HEAT_LABELS {
            let mut heat_label = play_ui::build_heat_label(app);
            heat_label.resolve(screen_width, screen_height);
            let key = play_ui::heat_label_key(index);
            app.ui.add_to_ui(key.clone(), heat_label);
            // Over the grey heat-view grade, so the labels stay white.
            app.ui.always_on_top.push(key);
        }
        for row in 0..play_ui::KILL_FEED_ROWS {
            let mut kill_feed_box = play_ui::build_kill_feed_box(app);
            kill_feed_box.resolve(screen_width, screen_height);
            app.ui.add_to_ui(play_ui::kill_feed_key(row), kill_feed_box);
            for piece in 0..play_ui::KILL_FEED_PIECES {
                let mut kill_feed_piece = play_ui::build_kill_feed_piece(app);
                kill_feed_piece.resolve(screen_width, screen_height);
                app.ui.add_to_ui(play_ui::kill_feed_piece_key(row, piece), kill_feed_piece);
            }
        }
        let mut velocity_marker = play_ui::build_velocity_marker(app);
        velocity_marker.resolve(screen_width, screen_height);
        app.ui.add_to_ui("velocity_marker".to_owned(), velocity_marker);
        let mut subtitles = play_ui::build_subtitles(app);
        subtitles.resolve(screen_width, screen_height);
        app.ui.add_to_ui("subtitles".to_owned(), subtitles);
        let mut skip_prompt = play_ui::build_skip_prompt(app);
        skip_prompt.resolve(screen_width, screen_height);
        app.ui.add_to_ui("skip_prompt".to_owned(), skip_prompt);
        let mut quick_menu = quick_list::build_quick_list_ui(app);
        quick_menu.resolve(screen_width, screen_height);
        app.ui.add_to_ui(quick_list::QUICK_LIST_UI.to_owned(), quick_menu);
        let mut debug_panel = play_ui::build_debug_panel(app);
        debug_panel.resolve(screen_width, screen_height);
        app.ui.add_to_ui("debug_panel".to_owned(), debug_panel);

        play_ui::build_pause_menu(app);
        play_ui::build_death_screen(app);

        let selected = SELECTED_LEVEL.lock().unwrap().clone();
        let (mission_title, location, mission_date) = match selected {
            Some(level) => (level.mission_title, level.location, level.mission_date),
            None => ("Unknown Mission".to_owned(), String::new(), String::new()),
        };

        let mut backdrop = UiNode::container()
            .set_size(SizeValue::Percent(100.0), SizeValue::Percent(100.0))
            .set_position(PositionValue::Start(0.0), PositionValue::Start(0.0))
            .set_background_color(UiColor::Rgba(0, 0, 0, 255));

        backdrop.resolve(screen_width, screen_height);
        
        app.ui.add_to_ui("mission_intro_backdrop".to_owned(), backdrop);
        app.ui.always_on_top.push("mission_intro_backdrop".to_owned());
        app.ui.always_on_top.push("mission_intro".to_owned());
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

        let subtitle_data = Subtitle::new();

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

        Self {
            blinking_alerts,
            gravity,
            start_time: Instant::now(),
            event_system,
            subtitle_data,
            game_time: 0.0,
            was_paused: false,
            esc_hold_time: 0.0,
            is_dead: false,
            hud_hidden_by_g: false,
            crash_time: 0.0,
            crash_seen_from_cockpit: false,
            went_in_at: HashMap::new(),
            heat_view: false,
            kill_feed: Vec::new(),
            flight_manager,
            flight_editor_was_open: false,
            flight_editor_map_invalid: false,
            camera_focus: String::new(),
            editor_selection: None,
            tactical_map: TacticalMap::default(),
            fire_control: FireControl::new(),
            missiles: Missiles::default(),
            crash_effects: CrashEffects::default(),
            weapon_specs: HashMap::new(),
            last_fired_side: -1.0,
            weapons_status: String::new(),
            countermeasures_status: String::new(),
            countermeasures_column: None,
            cms_cue: None,
            bingo_latched: false,
            bingo_warn_until: None,
            countermeasures: Countermeasures::default(),
            ai_defense: AiDefense::default(),
            range_cue: None,
            selected_weapon: None,
            weapon_list: Vec::new(),
            world_collision,
            map_nodes,
            animations,
            animation_hides_hud: false,
            hud_boot: None,
            editor_follow: None,
            telemetry_last: None,
            view_camera: PLAYER_CAMERA.to_owned(),
        }
    }

    fn spawn_world(scene: &mut Scene, app: &mut App, flight_manager: &mut flight_manager, world_collision: &mut WorldCollision, ground_trimesh: (Vec<Vector3<f32>>, Vec<[u32; 3]>), runway_trimesh: (Vec<Vector3<f32>>, Vec<[u32; 3]>)) {
        scene.spawn_node(app,
            Node::new("sun")
                .add_property(Transform3D {
                    // Only its direction from the world origin matters - the
                    // sun is treated as infinitely far away (see App::run's
                    // lighting update). 40 degrees above the horizon.
                    position: Vector3::new(445_200.0, 642_800.0, 623_300.0),
                    rotation: UnitQuaternion::identity(),
                    scale: Vector3::new(1.0, 1.0, 1.0),
                })
                .add_property(Model::new(crate::MARKER_MODEL))
        ).expect("play should only spawn 'sun' once");
        if let Some(sun) = scene.content.renderizable_instances.get_mut("sun") {
            sun.instance.metadata.lighting = Some(Lighting { intensity: 1.0, color: Vector3::new(0.7, 0.7, 0.8) });
        }

        // Pushed off the origin (was 0,0,0) to make room for the runway
        // island, which now sits at 0,0,0 - see the "runway" node below. Both
        // the mesh and its trimesh collider move with this Transform3D's
        // position, so nothing else needs adjusting.
        let ground_position = Vector3::new(0.0, 0.0, 50_000.0);
        let ground_scale = Vector3::new(30_000.0, 30_000.0, 30_000.0);
        let runway_position = Vector3::new(0.0, 100.0, 0.0);
        let runway_scale = Vector3::new(60.0, 60.0, 60.0);
        let sea_level = app.water.ocean.sea_level();
        // F10's tactical map: the same land as height contours, uploaded once.
        let contours = [
            contour_lines(&ground_trimesh.0, &ground_trimesh.1, ground_position, ground_scale, sea_level),
            contour_lines(&runway_trimesh.0, &runway_trimesh.1, runway_position, runway_scale, sea_level),
        ].concat();
        app.thick_lines.set_static(&app.renderer.device, &contours);
        // The same land, solid for missiles - the aircraft's is on the
        // physics thread (the colliders below).
        world_collision.add_mesh(&ground_trimesh.0, &ground_trimesh.1, ground_position, ground_scale);
        world_collision.add_mesh(&runway_trimesh.0, &runway_trimesh.1, runway_position, runway_scale);
        app.thick_lines.dynamic.clear();
        app.tactical_map_active = false;
        app.editor_active = false;
        scene.spawn_node(app,
            Node::new("ground")
                .add_property(Transform3D {
                    position: ground_position,
                    rotation: UnitQuaternion::identity(),
                    scale: ground_scale,
                })
                .add_property(Model::new("Ground"))
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

        // Runway island at the origin (the old "ground" island moved out to
        // ground_position above to make room). Trimesh collider built from the
        // model's own geometry - same pattern as "ground" - with the vertices
        // pre-scaled by the node's scale, since the collider is authored in
        // world units.
        scene.spawn_node(app,
            Node::new("runway")
                .add_property(Transform3D {
                    position: runway_position,
                    rotation: UnitQuaternion::identity(),
                    scale: runway_scale,
                })
                .add_property(Model::new("Runway"))
                .add_property(PhysicsProperty {
                    rigidbody: RigidBodyData {
                        is_static: true,
                        mass: 0.0,
                        center_of_mass: Vector3::new(0.0, 0.0, 0.0),
                        initial_velocity: Vector3::new(0.0, 0.0, 0.0),
                    },
                    colliders: vec![ColliderType::Trimesh {
                        vertices: runway_trimesh.0.iter().map(|v| Vector3::new(v.x * runway_scale.x, v.y * runway_scale.y, v.z * runway_scale.z)).collect(),
                        indices: runway_trimesh.1,
                    }],
                })
        ).expect("play should only spawn 'runway' once");

        // Every aircraft in the level's map.ron, the player's included -
        // see flight_manager::spawn_map.
        flight_manager.spawn_map(scene, app, LEVEL_FOLDER);

        // The spray the plane kicks up off the water when flying low and
        // fast - its own node, kept on the sea under the plane (see
        // update_water_wake), not on the plane itself: the spray comes off
        // the water, wherever the plane is and however it's banked.
        scene.spawn_node(app,
            Node::new("water_spray")
                .add_property(Transform3D::default())
                .add_property(super::plane::effects::spray_emitters())
        ).expect("play should only spawn 'water_spray' once");

        // The player's camera - a camera node (CameraLens) whose Camera
        // behavior places it every frame. See play::camera::camera::Camera's
        // own doc comment for how it gets fed the followed aircraft's
        // position each frame despite a Behavior having no way to reach
        // another node's data on its own.
        scene.spawn_node(app,
            Node::new(PLAYER_CAMERA)
                .add_property(Transform3D::default())
                .add_property(CameraLens::new(45.0))
                .add_behavior(Camera::new())
        ).expect("play should only spawn 'camera' once");

        scene.spawn_node(app,
            Node::new("fellow_aviator")
                .add_property(Transform3D {
                    position: Vector3::new(50.0, 50.0, 0.0),
                    rotation: UnitQuaternion::identity(),
                    scale: Vector3::new(19.0, 19.0, 19.0),
                })
                .add_property(Model::new("F14"))
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
            cameras.insert("front".to_owned(), GameObjectCamera { position: Vector3::new(0.0, 6.0, 26.0), fov: 60.0 });
            fellow_aviator.instance.metadata.cameras = Some(cameras);
        }

        // The whole ocean surface, out past the horizon - a level-of-detail
        // ring mesh (see main.rs's OCEAN_SURFACE_MODEL), kept centered under
        // the camera by update_water_plane. Scale 1: the mesh is in world units.
        scene.spawn_node(app,
            Node::new("world")
                .add_property(Transform3D {
                    position: Vector3::new(0.0, 0.0, 0.0),
                    rotation: UnitQuaternion::identity(),
                    scale: Vector3::new(1.0, 1.0, 1.0),
                })
                .add_property(Model::new(crate::OCEAN_SURFACE_MODEL))
        ).expect("play should only spawn 'world' once");
    }

    // Recenters "world"/"world_far" (see spawn_world) under the camera's XZ
    // every frame - continuous, not snapped to a grid: the wave field is an
    // analytic function of position, not a sampled heightmap/texture, and
    // "world"'s ~2-unit cells still comfortably oversample even the shortest
    // geometry wave (OceanSettings::geometry_min_wavelength, 10 units), so
    // there's no texel-shimmer/vertex-swimming risk to guard against. Y is left untouched on both - only XZ
    // needs to track the camera.
    fn update_water_plane(scene: &mut Scene) {
        let camera_position = scene.cameras.active().camera.position().coords;
        if let Some(world) = scene.content.nodes.get_mut("world").and_then(|node| node.get_property_mut::<Transform3D>()) {
            world.position.x = camera_position.x;
            world.position.z = camera_position.z;
        }
    }

    // Tells the water where the player's plane is and how fast it's going,
    // so it can stir up a wake when flying fast and low (see
    // WaterRenderData::set_wake_source - how strongly is decided there) -
    // and moves the "water_spray" node onto the sea right under it, turned
    // to its heading, spraying as hard as the wake is strong.
    fn update_water_wake(scene: &mut Scene, app: &mut App, player_id: &str) {
        let source = scene.content.renderizable_instances.get(player_id).map(|player| WakeSource {
            position: player.instance.transform.position,
            velocity: scene.content.nodes.get(player_id)
                .and_then(|node| node.physics_state::<AircraftState>())
                .map(|state| state.flight_data.velocity)
                .unwrap_or_else(Vector3::zeros),
        });
        app.water.set_wake_source(source);

        let Some(source) = source else { return };
        let strength = app.water.wake_strength(&source);
        let sea_level = app.water.ocean.sea_level();
        let Some(spray) = scene.content.nodes.get_mut("water_spray") else { return };
        if let Some(transform) = spray.get_property_mut::<Transform3D>() {
            transform.position = Vector3::new(source.position.x, sea_level, source.position.z);
            let heading = Vector3::new(source.velocity.x, 0.0, source.velocity.z);
            if heading.magnitude() > 1.0 {
                transform.rotation = UnitQuaternion::from_axis_angle(&Vector3::y_axis(), heading.x.atan2(heading.z));
            }
        }
        if let Some(emitters) = spray.get_property_mut::<ParticleEmitters>() {
            let plane_speed = Vector3::new(source.velocity.x, 0.0, source.velocity.z).magnitude();
            super::plane::effects::update_spray(emitters, strength, plane_speed);
        }
    }

    /// Once the plane has crashed into the water (see Plane::check_wreck -
    /// the physics keep running: it gets dragged to a stop, floats, sinks),
    /// the camera backs out into a free orbit around the wreck. No death
    /// screen for now (play_ui::open_death_screen) - restart from the pause
    /// menu.
    ///
    /// When the crash kills the pilot the view cuts to black first (see
    /// Pilot's DEATH_*) and the camera switches while it's black, so the
    /// cut isn't seen. A crash the pilot survives switches after
    /// WRECK_VIEW_DELAY_SECONDS - long enough to tell whether it was fatal
    /// (then it waits for the black instead).
    fn check_crash(&mut self, scene: &mut Scene, app: &App) {
        if self.is_dead {
            return;
        }
        let Some(plane) = scene.content.nodes.get(self.flight_manager.player_id()).and_then(|node| node.get_behavior::<Plane>()) else { return };
        if !plane.wrecked {
            return;
        }
        self.crash_time += app.time.delta_time;
        let ready = if plane.pilot.is_dead() {
            plane.pilot.death_view_hidden()
        } else {
            self.crash_time >= Self::WRECK_VIEW_DELAY_SECONDS
        };
        if !ready {
            return;
        }
        self.is_dead = true;
        let sea_level = app.water.ocean.sea_level();
        if let Some(camera) = scene.content.nodes.get_mut("camera").and_then(|node| node.get_behavior_mut::<Camera>()) {
            camera.enter_wreck_view(sea_level);
        }
    }

    /// Drains the colors, closes in the tunnel or reds out the view as the
    /// pilot does (see plane::pilot::Pilot::screen_effects) - every frame,
    /// since screen effects only last one (see BlurRender::write_effects).
    /// The flight HUD is drawn under that same pass, so it greys, dims and
    /// tunnels with the world. It's still switched off once crashed, and
    /// while the map, the editor or an animation asks for it gone.
    fn update_g_vision(&mut self, scene: &Scene, app: &mut App) {
        let player_id = self.flight_manager.player_id();
        let Some(plane) = scene.content.nodes.get(player_id).and_then(|node| node.get_behavior::<Plane>()) else { return };
        // Only seen through the pilot's eyes - out of the cockpit the pilot
        // still greys/blacks/reds out (and loses the plane), and a crash
        // still kills them, it just isn't shown. A crash seen from the
        // cockpit fades out and back in on the wreck view, the camera having
        // left the cockpit by then.
        let in_cockpit = self.camera_focus == player_id
            && scene.content.nodes.get("camera").and_then(|node| node.get_behavior::<Camera>()).is_some_and(Camera::is_cockpit);
        if !plane.pilot.is_dead() {
            self.crash_seen_from_cockpit = false;
        } else if in_cockpit {
            self.crash_seen_from_cockpit = true;
        }
        let shown = in_cockpit || self.crash_seen_from_cockpit;
        let mut effects = if shown { plane.pilot.screen_effects() } else { ScreenEffects::default() };
        // F8's heat view: the world dim and grey under the heat sources
        // (see update_heat_view).
        if self.heat_view && !(app.tactical_map_active || app.editor_active) {
            effects = ScreenEffects::default();
            effects.desaturation = 1.0;
            effects.darkening = 0.75;
        }
        app.renderer.blur.effects = effects;

        let hide = plane.wrecked || plane.pilot.is_dead() || (app.tactical_map_active || app.editor_active) || self.animation_hides_hud;
        if !hide && self.hud_hidden_by_g {
            // Vision's back - the flight HUD returns (the quick menu shows
            // itself again once the pilot has control, see Plane).
            for hud in Self::FLIGHT_HUD {
                if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, hud) {
                    node.set_active(true);
                }
            }
            app.ui.has_changed = true;
        }
        self.hud_hidden_by_g = hide;
    }

    // The flight HUD: timer, FPS, G, power, altitude, speed, compass (all
    // under "game_ui") and the flight path marker.
    const FLIGHT_HUD: [&'static str; 2] = ["game_ui", "velocity_marker"];

    /// The player's own cockpit, the view the HUD belongs to.
    fn in_player_cockpit(&self, scene: &Scene) -> bool {
        self.view_camera == PLAYER_CAMERA
            && scene.content.nodes.get(PLAYER_CAMERA).and_then(|node| node.get_behavior::<Camera>()).is_some_and(Camera::is_cockpit)
    }

    /// Starts the HUD power-on the frame a cinematic stops hiding it and
    /// the view is the cockpit. A shot that never hid the HUD leaves it
    /// alone. Opening the editor or the map, or another shot hiding it
    /// again, drops a boot that's still running.
    fn sync_hud_boot(&mut self, scene: &Scene, app: &mut App, was_hiding_hud: bool) {
        if app.editor_active || app.tactical_map_active || self.animation_hides_hud {
            if self.hud_boot.is_some() {
                self.hud_boot = None;
                app.renderer.blur.hud_opacity = 1.0;
            }
            return;
        }
        if was_hiding_hud && self.in_player_cockpit(scene) {
            self.hud_boot = Some(0.0);
            app.renderer.blur.hud_opacity = 0.0;
        }
    }

    /// Advances the power-on and writes the HUD's opacity. `advance` is
    /// false while paused, so the fade holds instead of running behind the
    /// menu. Once the fade is done the HUD stays fully on.
    fn tick_hud_boot(&mut self, app: &mut App, advance: bool) {
        if self.hud_hidden_by_g {
            self.hud_boot = None;
            app.renderer.blur.hud_opacity = 1.0;
            return;
        }
        let Some(t) = self.hud_boot else {
            app.renderer.blur.hud_opacity = 1.0;
            return;
        };
        let t = if advance { t + app.time.delta_time } else { t };
        if t >= hud_boot_length() {
            self.hud_boot = None;
            app.renderer.blur.hud_opacity = 1.0;
        } else {
            self.hud_boot = Some(t);
            app.renderer.blur.hud_opacity = hud_boot_alpha(t);
        }
    }

    /// Keeps the flight HUD hidden while update_g_vision says so - every
    /// frame, after the level's UI tracks (EventSystem::apply_tracks) have
    /// run, since those set "game_ui" active every frame too. The quick
    /// menu hides itself (see Plane::update_quick_menu).
    fn hide_hud(&self, app: &mut App) {
        if !self.hud_hidden_by_g {
            return;
        }
        for hud in Self::FLIGHT_HUD {
            if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, hud) {
                if node.is_active {
                    node.set_active(false);
                    app.ui.has_changed = true;
                }
            }
        }
    }

    // How long ESC has to be held during an EventSystem::input_lock_end
    // window before it counts as a skip - see `update`'s own handling.
    const SKIP_HOLD_SECONDS: f32 = 1.2;

    // A survivable crash waits this long before the wreck camera - see
    // check_crash.
    const WRECK_VIEW_DELAY_SECONDS: f32 = 0.6;

    // this is called every frame
    // physics_data is unused now - Plane::fixed_update reacts to physics
    // results generically (see that fn's own doc comment on what used to
    // read this here instead) - kept as a parameter rather than dropped
    // since ctx.physics_data still needs somewhere to go through FrameContext
    // and another SceneBehaviour-level consumer may want it later.
    pub fn update(&mut self, scene: &mut Scene, app: &mut App, physics_command_tx: Option<&Sender<PhysicsCommand>>, _physics_data: &HashMap<String, RenderMessage>) {
        // Planes' data.ron edits, applied live - see flight_manager::update.
        self.flight_manager.update(scene, app.time.delta_time);
        // The live map editor (F10's editor view); paused or not.
        self.update_flight_editor(scene, app, physics_command_tx);
        // The map's own nodes and the tracks on every node, as the map (the
        // editor's, while it's up) has them.
        self.map_nodes.sync(scene, app, self.flight_manager.map());
        // F10 - the tactical map; live, paused or not.
        self.update_tactical_map(scene, app, physics_command_tx);
        // The aircraft the player flies - its name in map.ron (the editor
        // can switch it).
        let player_id = self.flight_manager.player_id().to_owned();
        app.player_node = player_id.clone();
        let player_id = player_id.as_str();

        // See play::camera::camera::Camera::set_target's own doc comment -
        // has to run unconditionally, before ANY early return below
        // (app.is_paused, self.is_dead), so the "camera" node's own generic
        // Behavior tick (which always runs every frame regardless of what
        // this fn does - see App::run's own unconditional node-tick call)
        // never reads a target one frame stale relative to whatever physics
        // just wrote into "player"'s own renderable transform this frame.
        //
        // The missiles move first - see fly_missiles.
        if !app.is_paused {
            self.fly_missiles(scene, app);
            self.update_ai_countermeasures(scene, app);
            self.show_crashes(scene, app);
        }

        // Which instance the camera follows - see update_camera_focus.
        let (focus, switch) = self.update_camera_focus(scene, app, player_id);
        if let Some(camera) = scene.content.nodes.get_mut("camera").and_then(|node| node.get_behavior_mut::<Camera>()) {
            match switch {
                Some(CameraSwitch::To(state)) => camera.set_state(state),
                Some(CameraSwitch::Next) => camera.next_camera(),
                None => {}
            }
        }
        self.update_editor_follow(scene, app, &focus);
        if let Some(followed) = scene.content.renderizable_instances.get(&focus) {
            // A missile: along its flight, not turned like its model.
            let (position, rotation) = self.missiles.camera_pose(&focus)
                .unwrap_or((followed.instance.transform.position, followed.instance.transform.rotation));
            // A wreck in the sea: where it went in, on the surface - not
            // following it down as it sinks.
            let in_water = scene.content.nodes.get(&focus).and_then(|node| node.get_behavior::<Plane>()).is_some_and(Plane::in_water);
            let (position, rotation) = if in_water {
                let sea_level = app.water.ocean.sea_level();
                *self.went_in_at.entry(focus.clone()).or_insert_with(|| {
                    let forward = rotation * Vector3::z();
                    let heading = Vector3::new(forward.x, 0.0, forward.z).try_normalize(1e-4).unwrap_or_else(Vector3::z);
                    (Vector3::new(position.x, sea_level, position.z), UnitQuaternion::face_towards(&heading, &Vector3::y()))
                })
            } else {
                self.went_in_at.remove(&focus);
                (position, rotation)
            };
            let named_cameras = followed.instance.metadata.cameras.clone();
            let head_input = scene.content.nodes.get(&focus)
                .and_then(|node| node.physics_state::<AircraftState>())
                .map(|state| HeadInput {
                    lateral: state.flight_data.lateral_g,
                    forward: state.flight_data.longitudinal_g,
                    vertical_g: state.flight_data.body_g.y,
                });
            if let Some(node) = scene.content.nodes.get_mut("camera") {
                if let Some(camera) = node.get_behavior_mut::<Camera>() {
                    camera.set_target(position, rotation, named_cameras);
                    if let Some(head_input) = head_input {
                        camera.set_head_input(head_input);
                    }
                }
            }
        }

        // Every frame, paused or not - the effects only last one frame, and
        // the view shouldn't pop back to normal behind the pause menu.
        self.update_g_vision(scene, app);
        // The HUD's power-on, if a cinematic just handed the cockpit back.
        // Paused, it holds whatever brightness it had.
        self.tick_hud_boot(app, !app.is_paused);

        // F3 debug view - shows/hides in lockstep with the console toggle,
        // same "set_active every frame, no separate dirty-tracking" idiom
        // skip_prompt below already uses (its own text is updated inside
        // ui_control, right alongside compass/speed/etc. - not from a
        // function of its own).
        if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "debug_panel") {
            node.set_active(debug_console::is_console_visible());
        }

        // F7 - the debug windows, with the mouse freed to click them (see App::
        // debug_mouse_free). The pause menu owns the cursor while it's up -
        // close_pause_menu puts back whichever mode this left.
        if input::is_action_just_pressed("toggle_debug_mouse") {
            app.debug_mouse_free = !app.debug_mouse_free;
            if !app.is_paused {
                app.window_manager.context.mouse().set_relative_mouse_mode(!app.mouse_free());
            }
        }

        // Whether player input is locked out right now (see EventSystem::
        // input_lock_end's own doc comment) - checked against this frame's
        // not-yet-incremented game_time, one frame stale at worst, which
        // doesn't matter for a coarse window check like this.
        let input_lock_end = self.event_system.as_ref().and_then(|es| es.input_lock_end(self.game_time));
        if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "skip_prompt") {
            node.set_active(input_lock_end.is_some());
        }
        // Locked (the intro, an animation playing - from its first frame):
        // no flying and no looking round either - the camera eases back to
        // its center. And the editor camera flying (right button held) keeps
        // the plane's controls off too.
        let controls_locked = input_lock_end.is_some() || self.sequence_locks_input();
        let mut editor_flying = false;
        if let Some(camera) = scene.content.nodes.get_mut(PLAYER_CAMERA).and_then(|node| node.get_behavior_mut::<Camera>()) {
            camera.look_locked = controls_locked;
            editor_flying = camera.editor_looking();
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
            // Unlike before this became a Behavior, no special "run the
            // camera one extra time right now" case is needed on the exact
            // frame pausing begins - the "camera" node's own generic
            // Behavior tick (see App::run's own unconditional node-tick
            // call) runs every frame regardless of this early return, and
            // set_target above (also unconditional, before this check) has
            // already fed it this frame's fresh player position - so it
            // never reads stale data even on this exact frame.
            //
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
            // The pause menu's Settings panel is main_menu's own - its
            // sliders need following while dragged here too.
            main_menu_ui::update_settings_sliders(app);
            main_menu_ui::refresh_controller_states(app);
            return;
        }

        // A paused or finished animation still has the plane. Flying it lets
        // go at once, from where it is, on its own flight plan.
        self.release_player_to_live(scene, app, player_id);

        self.game_time += app.time.delta_time as f64;

        // Everything else about the plane (input, physics feedback, model
        // animation) runs in its own Plane behavior - input_locked is the one
        // thing that depends on the level's animations / intro state only this
        // SceneBehaviour sees.
        if let Some(plane) = scene.content.nodes.get_mut(player_id).and_then(|node| node.get_behavior_mut::<Plane>()) {
            // Typing in the flight editor isn't flying.
            plane.input_locked = input_lock_end.is_some() || app.flight_editor.typing || self.sequence_locks_input() || editor_flying;
        }

        Self::update_water_plane(scene);
        Self::update_water_wake(scene, app, player_id);
        self.check_crash(scene, app);
        let has_control = input_lock_end.is_none() && !app.flight_editor.typing && !self.sequence_locks_input();
        self.update_weapons(scene, app, has_control);

        if let Some(event_system) = &mut self.event_system {
            event_system.handle_events(self.game_time, app, &mut self.subtitle_data);
        }
        self.subtitle_data.update(app);
        self.ui_control(scene, app);

        if let Some(event_system) = &self.event_system {
            event_system.apply_tracks(self.game_time, app);
        }
        self.update_hud(scene, app);
        self.update_lock_arrow(scene, app);
        self.update_heat_view(scene, app);
        self.update_kill_feed(app);
        self.hide_hud(app);

    }

    /// Which instance the camera follows, and the view to switch it to -
    /// F1: the player's aircraft, in the cockpit - pressed again there, its
    /// next view (see play::camera::camera::CameraState); F2: the next of
    /// the other aircraft, round and round, in the free view; F3/F4: the
    /// next ground unit / launched missile (there are no ground units yet);
    /// or an aircraft picked in the flight editor (F10's editor view)'s hierarchy. Back to
    /// the player when the one it followed is gone. Returns its node id.
    fn update_camera_focus(&mut self, scene: &Scene, app: &App, player_id: &str) -> (String, Option<CameraSwitch>) {
        let mut switch = None;
        let selection = app.flight_editor.selected_aircraft().map(str::to_owned);
        if selection != self.editor_selection {
            if let Some(name) = &selection {
                self.camera_focus = name.clone();
            }
            self.editor_selection = selection;
        }

        if input::is_action_just_pressed("camera_player") {
            if self.camera_focus == player_id {
                switch = Some(CameraSwitch::Next);
            } else {
                self.camera_focus = player_id.to_owned();
                switch = Some(CameraSwitch::To(CameraState::Cockpit));
            }
        }
        if input::is_action_just_pressed("camera_next_aircraft") {
            // Every aircraft but the player's, in the map's order - the one
            // after whichever it's on now.
            let others: Vec<&str> = self.flight_manager.map().aircraft()
                .map(|(_, aircraft)| aircraft.name.as_str())
                .filter(|name| *name != player_id && scene.content.renderizable_instances.contains_key(*name))
                .collect();
            let current = others.iter().position(|name| *name == self.camera_focus);
            match others.get(current.map(|index| index + 1).unwrap_or(0) % others.len().max(1)) {
                Some(name) => {
                    self.camera_focus = (*name).to_owned();
                    switch = Some(CameraSwitch::To(CameraState::Free));
                }
                None => println!("Camera: no other aircraft to follow"),
            }
        }
        if input::is_action_just_pressed("camera_next_ground_unit") {
            println!("Camera: no ground units to follow yet");
        }
        if input::is_action_just_pressed("camera_next_missile") {
            let missiles: Vec<&str> = self.missiles.node_ids().collect();
            let current = missiles.iter().position(|id| *id == self.camera_focus);
            match missiles.get(current.map(|index| index + 1).unwrap_or(0) % missiles.len().max(1)) {
                Some(id) => self.camera_focus = (*id).to_owned(),
                None => println!("Camera: no missiles in the air"),
            }
        }

        if !scene.content.renderizable_instances.contains_key(&self.camera_focus) {
            self.camera_focus = player_id.to_owned();
        }
        (self.camera_focus.clone(), switch)
    }

    /// F10's editor and M's map (see play::tactical_map): opens/closes them
    /// (one at a time) - for the map, the
    /// camera into its orbit view and the renderer onto its lines - keeps
    /// every aircraft's trail, and while it's open builds this frame's
    /// lines.
    fn update_tactical_map(&mut self, scene: &mut Scene, app: &mut App, physics_command_tx: Option<&Sender<PhysicsCommand>>) {
        // F10 - the editor; not with the map up (one closes the other).
        if input::is_action_just_pressed("toggle_editor") {
            let open = !app.editor_active;
            if open && self.tactical_map.open {
                self.tactical_map.open = false;
                app.tactical_map_active = false;
                if let Some(camera) = scene.content.nodes.get_mut(PLAYER_CAMERA).and_then(|node| node.get_behavior_mut::<Camera>()) {
                    camera.set_tactical_view(false);
                }
            }
            app.editor_active = open;
            app.ui.has_changed = true;
            // Starts where the view is now.
            let active = scene.cameras.active();
            let from = (active.camera.position().coords, active.camera.calc_forward_direction());
            if let Some(camera) = scene.content.nodes.get_mut(PLAYER_CAMERA).and_then(|node| node.get_behavior_mut::<Camera>()) {
                camera.set_editor_view(open, from);
            }
            if !app.is_paused {
                app.window_manager.context.mouse().set_relative_mouse_mode(!app.mouse_free());
            }
        }
        if input::is_action_just_pressed("toggle_tactical_map") && !app.flight_editor.typing {
            if app.editor_active {
                app.editor_active = false;
                if let Some(camera) = scene.content.nodes.get_mut(PLAYER_CAMERA).and_then(|node| node.get_behavior_mut::<Camera>()) {
                    camera.set_editor_view(false, (Vector3::zeros(), Vector3::z()));
                }
            }
            self.tactical_map.open = !self.tactical_map.open;
            app.tactical_map_active = self.tactical_map.open;
            app.ui.has_changed = true;
            if let Some(camera) = scene.content.nodes.get_mut("camera").and_then(|node| node.get_behavior_mut::<Camera>()) {
                camera.set_tactical_view(self.tactical_map.open);
            }
            // A pointer while it's up, to click with - see App::mouse_free.
            if !app.is_paused {
                app.window_manager.context.mouse().set_relative_mouse_mode(!app.mouse_free());
            }
            // Opens with something selected - what the camera follows - so its
            // flight plan's up straight away.
            if self.tactical_map.open && app.tactical_ui.selected.is_none() {
                let followed = if self.camera_focus.is_empty() { self.flight_manager.player_id().to_owned() } else { self.camera_focus.clone() };
                app.tactical_ui.selected = Some(followed);
            }
        }

        let live: HashMap<String, LiveAircraft> = self.flight_manager.map().aircraft()
            .filter_map(|(_, aircraft)| Some((aircraft.name.clone(), self.flight_manager.live(scene, &aircraft.name)?)))
            .collect();
        if !app.is_paused {
            self.tactical_map.record(&live, app.time.delta_time);
        }
        if !self.tactical_map.open {
            app.thick_lines.dynamic.clear();
            return;
        }

        // A flight plan changed in its window: into the map, applied now (a
        // bot flies the new path) - and into the editor's copy too, so it
        // doesn't put the old one back.
        // A flight plan point dragged on the map: across under the pointer,
        // at its own height - or, with Shift, up and down. Into the path like
        // an edit in its window (below).
        if let Some(drag) = app.tactical_ui.point_drag.take() {
            // The flight plan shown - an aircraft's or a map path's.
            let shown = app.tactical_ui.plan.as_ref().map(|plan| (plan.target.clone(), plan.points.clone()));
            if let Some((target, mut path)) = shown {
                if let Some(point) = path.get_mut(drag.index) {
                    let active = scene.cameras.active();
                    let [left, top, width, height] = tactical_viewport(app);
                    if drag.vertical {
                        // A pixel's worth of height at the point's distance.
                        let distance = (Vector3::new(point.0, point.1, point.2) - active.camera.position().coords).magnitude();
                        let meters_per_pixel = 2.0 * distance * (active.projection.fovy.to_radians() * 0.5).tan() / height.max(1.0);
                        point.1 = (point.1 - drag.delta.y * meters_per_pixel).clamp(0.0, 20_000.0);
                    } else if let Some((origin, direction)) = app.camera_resources.screen_ray(active, drag.pointer.x - left, drag.pointer.y - top, width as u32, height as u32) {
                        // Where the pointer's ray meets the flat plane at the
                        // point's height - not past the far horizon.
                        if direction.y.abs() > 1e-4 {
                            let distance = (point.1 - origin.y) / direction.y;
                            if distance > 0.0 && distance < 500_000.0 {
                                let hit = origin.coords + direction * distance;
                                point.0 = hit.x;
                                point.2 = hit.z;
                            }
                        }
                    }
                    app.tactical_ui.path_edit = Some((target, path));
                }
            }
        }

        // Into the map path it is (or the aircraft follows - every aircraft
        // on it flies the new one), or the aircraft's own.
        if let Some((target, path)) = app.tactical_ui.path_edit.take() {
            let apply = |map: &mut MapSpec| match &target {
                PlanTarget::Aircraft(name) => map.set_flight_plan(name, path.clone()),
                PlanTarget::Path(name) => {
                    if let Some(spec) = map.named_path_mut(name) {
                        spec.points = path.clone();
                    }
                }
            };
            let mut map = self.flight_manager.map().clone();
            apply(&mut map);
            if let Err(error) = self.flight_manager.sync(scene, app, physics_command_tx, &map) {
                eprintln!("F10 flight plan: {error}");
            }
            apply(&mut app.flight_editor.map);
        }

        // Its center button: the camera kept on the selected aircraft -
        // otherwise free, dragged across the map.
        let ui = &mut app.tactical_ui;
        if ui.selected.as_ref().is_some_and(|name| self.flight_manager.map().find(name).is_none()) {
            ui.selected = None;
        }
        if ui.following {
            match &ui.selected {
                Some(name) => self.camera_focus = name.clone(),
                None => ui.following = false,
            }
        }
        let following = ui.following;
        let pan = std::mem::take(&mut ui.pan_delta);
        let [_, _, _, map_height] = tactical_viewport(app);
        if let Some(camera) = scene.content.nodes.get_mut("camera").and_then(|node| node.get_behavior_mut::<Camera>()) {
            camera.set_tactical_follow(following);
            if pan != (0.0, 0.0) {
                camera.pan_tactical(pan, map_height);
            }
        }

        let active = scene.cameras.active();
        let focus = scene.content.renderizable_instances.get(&self.camera_focus)
            .map(|instance| instance.instance.transform.position)
            .unwrap_or_else(|| active.camera.position().coords);
        let to_pixels = app.window_manager.pixel_size.height as f32 / (app.window_manager.size.height as f32).max(1.0);
        let view = TacticalView {
            camera_position: active.camera.position().coords,
            fovy_deg: active.projection.fovy,
            screen_height: map_height * to_pixels,
            focus,
        };
        let sea_level = app.water.ocean.sea_level();
        let selected = app.tactical_ui.selected.clone();
        let hovered = app.tactical_ui.hovered.clone();
        // A map path picked in the editor is shown instead of the aircraft's
        // flight plan.
        let picked_path = app.tactical_ui.selected_path.as_deref().and_then(|name| self.flight_manager.map().named_path(name)).cloned();
        let plan_of = if picked_path.is_some() { None } else { selected.as_deref() };
        app.thick_lines.dynamic = self.tactical_map.lines(self.flight_manager.map(), &live, self.flight_manager.player_id(), plan_of, hovered.as_deref(), &view, sea_level);
        if let Some(path) = &picked_path {
            tactical_map::flight_plan(&mut app.thick_lines.dynamic, &path.points, path.looped, None, sea_level, &view);
        }
        // The missiles in the air, in their shooter's colors.
        for missile in self.missiles.in_flight() {
            let rgb = match self.flight_manager.map().find(&missile.shooter) {
                Some((_, placement)) if placement.player => [1.0, 0.82, 0.30],
                Some((coalition, _)) => [coalition.color.0, coalition.color.1, coalition.color.2].map(|channel| channel as f32 / 255.0),
                None => [1.0, 1.0, 1.0],
            };
            tactical_map::missile_marker(&mut app.thick_lines.dynamic, missile.position, missile.velocity, rgb, &view);
        }
        self.fill_tactical_ui(scene, app, &live, selected.as_deref());
    }

    /// What the tactical map's egui half needs this frame (see play::
    /// tactical_ui): where every aircraft is on screen (to click), and the
    /// selected one's flight plan and its points' numbers.
    fn fill_tactical_ui(&self, scene: &Scene, app: &mut App, live: &HashMap<String, LiveAircraft>, selected: Option<&str>) {
        // On the map's part of the screen (see tactical_viewport).
        let [left, top, width, height] = tactical_viewport(app);
        let active = scene.cameras.active();
        let on_screen = |position: Vector3<f32>| {
            app.camera_resources.world_to_screen(active, Point3::from(position), width as u32, height as u32)
                .map(|point| egui::pos2(left + point.x as f32, top + point.y as f32))
        };

        let pickables = live.iter()
            .filter(|(_, aircraft)| !aircraft.wrecked)
            .filter_map(|(name, aircraft)| Some((name.clone(), on_screen(aircraft.position)?)))
            .collect();

        let mut labels = Vec::new();
        let mut point_pickables = Vec::new();
        let picked_path = app.tactical_ui.selected_path.as_deref().and_then(|name| self.flight_manager.map().named_path(name));
        let path_plan = picked_path.map(|path| {
            for (index, &(x, altitude, z)) in path.points.iter().enumerate() {
                if let Some(at) = on_screen(Vector3::new(x, altitude, z)) {
                    labels.push((at, format!("{}  {:.0} m", index + 1, altitude), egui::Color32::WHITE));
                    point_pickables.push((index, at));
                }
            }
            // On from the last point the way the last leg goes (or north).
            let next_suggestion = match path.points.as_slice() {
                [.., a, b] => (b.0 + (b.0 - a.0).signum() * 1000.0, b.1, b.2 + (b.2 - a.2).signum() * 1000.0),
                [only] => (only.0, only.1, only.2 + 1000.0),
                [] => {
                    let at = scene.cameras.active().camera.position();
                    (at.x, 1000.0, at.z)
                }
            };
            FlightPlan {
                target: PlanTarget::Path(path.name.clone()),
                path_name: Some(path.name.clone()),
                name: path.name.clone(),
                coalition: String::new(),
                color: egui::Color32::from_rgb(140, 200, 255),
                plane: format!("{} points · {}{}", path.points.len(), if path.looped { "looped" } else { "once" }, if path.smooth { " · smooth" } else { "" }),
                is_player: false,
                live: None,
                points: path.points.clone(),
                next: None,
                next_suggestion,
            }
        });
        let plan = if path_plan.is_some() { path_plan } else { selected.and_then(|name| self.flight_manager.map().find(name)).map(|(coalition, placement)| {
            let now = live.get(&placement.name).copied();
            let next = now.and_then(|now| now.next_path_point);
            let (points, _) = self.flight_manager.map().flight_plan(placement);
            for (index, &(x, altitude, z)) in points.iter().enumerate() {
                if let Some(at) = on_screen(Vector3::new(x, altitude, z)) {
                    let color = if next == Some(index) { egui::Color32::from_rgb(255, 205, 40) } else { egui::Color32::WHITE };
                    labels.push((at, format!("{}  {:.0} m", index + 1, altitude), color));
                    point_pickables.push((index, at));
                }
            }
            let is_player = placement.name == self.flight_manager.player_id();
            let color = if is_player {
                egui::Color32::from_rgb(255, 210, 90)
            } else {
                egui::Color32::from_rgb(coalition.color.0, coalition.color.1, coalition.color.2)
            };
            FlightPlan {
                target: PlanTarget::Aircraft(placement.name.clone()),
                path_name: placement.follow.clone().filter(|name| self.flight_manager.map().named_path(name).is_some()),
                name: placement.name.clone(),
                coalition: coalition.name.clone(),
                color,
                plane: placement.plane.clone(),
                is_player,
                live: now,
                next_suggestion: next_path_point(placement, &points, now),
                points,
                next,
            }
        }) };

        let roster = self.flight_manager.map().coalitions.iter().map(|coalition| RosterCoalition {
            name: coalition.name.clone(),
            color: egui::Color32::from_rgb(coalition.color.0, coalition.color.1, coalition.color.2),
            aircraft: coalition.aircraft.iter().map(|aircraft| RosterEntry {
                name: aircraft.name.clone(),
                plane: aircraft.plane.clone(),
                is_player: aircraft.name == self.flight_manager.player_id(),
                status: match live.get(&aircraft.name) {
                    None => AircraftStatus::Missing,
                    Some(now) if now.wrecked => AircraftStatus::Destroyed,
                    Some(_) => AircraftStatus::Flying,
                },
            }).collect(),
        }).collect();

        let ui = &mut app.tactical_ui;
        ui.pickables = pickables;
        ui.labels = labels;
        ui.point_pickables = point_pickables;
        ui.roster = roster;
        ui.plan = plan;
    }

    /// The flight editor (see play::flight_editor), docked in F10's editor
    /// view: starts it from the world as it is when that opens, carries out
    /// what it was asked to (save, reload, respawn...), and applies its map
    /// to the world - every frame it's up (see flight_manager::sync).
    fn update_flight_editor(&mut self, scene: &mut Scene, app: &mut App, physics_command_tx: Option<&Sender<PhysicsCommand>>) {
        // Docked in F10's editor view - up while that is.
        let open = app.editor_active;
        if open != self.flight_editor_was_open {
            self.flight_editor_was_open = open;
            if open {
                // Starts from the world as it is.
                app.flight_editor.map = self.flight_manager.map().clone();
                app.flight_editor.planes = AircraftSpec::available();
                // What each plane's loadout can fill, and with what.
                app.flight_editor.pylons = app.flight_editor.planes.iter()
                    .filter_map(|plane| Some((plane.clone(), AircraftSpec::load(plane).ok()?.pylons)))
                    .collect();
                // And how many flares and chaff it carries.
                app.flight_editor.countermeasures = app.flight_editor.planes.iter()
                    .filter_map(|plane| {
                        let countermeasures = AircraftSpec::load(plane).ok()?.countermeasures?;
                        Some((plane.clone(), (countermeasures.flares, countermeasures.chaff)))
                    })
                    .collect();
                app.flight_editor.fuel_capacity = app.flight_editor.planes.iter()
                    .filter_map(|plane| {
                        let capacity = AircraftSpec::load(plane).ok()?.fuel.capacity_kg;
                        (capacity > 0.0).then_some((plane.clone(), capacity))
                    })
                    .collect();
                app.flight_editor.weapons = WeaponSpec::available().into_iter()
                    .filter_map(|id| Some((id.clone(), WeaponSpec::read(&id).ok()?.category)))
                    .collect();
                app.flight_editor.status = None;
                app.flight_editor.requests.clear();
            } else {
                app.flight_editor.typing = false;
            }
            // The pause menu owns the cursor while it's up - see
            // play_ui::close_pause_menu.
            if !app.is_paused {
                app.window_manager.context.mouse().set_relative_mouse_mode(!app.mouse_free());
            }
        }
        if !open {
            return;
        }

        let mut editor = std::mem::take(&mut app.flight_editor);
        let mut respawn_all = false;
        let mut respawn = Vec::new();
        for request in std::mem::take(&mut editor.requests) {
            match request {
                EditorRequest::Save => {
                    editor.status = Some(match editor.map.save(LEVEL_FOLDER) {
                        Ok(()) => (format!("Saved to {}", MapSpec::path(LEVEL_FOLDER).display()), false),
                        Err(error) => (error, true),
                    });
                }
                EditorRequest::ReloadFromFile => {
                    editor.status = Some(match MapSpec::load(LEVEL_FOLDER) {
                        Ok(map) => {
                            editor.map = map;
                            ("Reloaded map.ron".to_owned(), false)
                        }
                        Err(error) => (error, true),
                    });
                }
                EditorRequest::Timeline(request) => match request {
                    TimelineRequest::Play(node, name) => {
                        let length = editor.map.animation(&node, &name).map(|animation| animation.length).unwrap_or(0.0);
                        self.animations.resume(&node, &name, length);
                    }
                    TimelineRequest::Start(node, name) => self.animations.start(&node, &name),
                    TimelineRequest::Pause(node, name) => self.animations.pause(&node, &name),
                    TimelineRequest::Stop(node, name) => self.animations.stop(&node, &name),
                    TimelineRequest::Reset(node, name) => {
                        self.animations.stop(&node, &name);
                        scene.content.kinematic.remove(&node);
                        if editor.map.find(&node).is_some() {
                            respawn.push(node);
                        } else if let Some(spec) = editor.map.node(&node).cloned() {
                            scene.content.kinematic.remove(&node);
                            if let Some(placed) = scene.content.nodes.get_mut(&node) {
                                placed.set_property(Transform3D { position: spec.position(), rotation: spec.rotation(), scale: Vector3::new(1.0, 1.0, 1.0) });
                            }
                        }
                    }
                    TimelineRequest::Seek(node, name, time) => {
                        let length = editor.map.animation(&node, &name).map(|animation| animation.length).unwrap_or(time);
                        self.animations.seek(&node, &name, time.min(length));
                    }
                },
                EditorRequest::Focus(id) => {
                    self.editor_follow = None;
                    if let Some(world) = scene.world_transform(&id) {
                        let distance = if self.flight_manager.map().find(&id).is_some() { EDITOR_FOCUS_AIRCRAFT } else { EDITOR_FOCUS_NODE };
                        if let Some(camera) = scene.content.nodes.get_mut(PLAYER_CAMERA).and_then(|node| node.get_behavior_mut::<Camera>()) {
                            camera.editor_focus(world.position, distance);
                        }
                    }
                }
                EditorRequest::RespawnAll => respawn_all = true,
                EditorRequest::Respawn(name) => respawn.push(name),
                EditorRequest::ResetLoadout(name) => self.flight_manager.reset_loadout(scene, app, &name),
                EditorRequest::CaptureCurrent(name) => {
                    // Into both copies of the map, so nothing teleports.
                    if let Some((position, rotation)) = self.flight_manager.capture_current(scene, &name) {
                        if let Some(placement) = editor.map.find_mut(&name) {
                            placement.position = position;
                            placement.rotation = rotation;
                        }
                    }
                }
            }
        }

        match self.flight_manager.sync(scene, app, physics_command_tx, &editor.map) {
            Ok(()) => {
                if self.flight_editor_map_invalid {
                    editor.status = None;
                }
                self.flight_editor_map_invalid = false;
            }
            Err(error) => {
                editor.status = Some((error, true));
                self.flight_editor_map_invalid = true;
            }
        }
        let respawned = if respawn_all {
            self.flight_manager.respawn_all(scene, app, physics_command_tx)
        } else {
            respawn.iter().map(|name| self.flight_manager.respawn(scene, app, physics_command_tx, name)).collect()
        };
        if let Err(error) = respawned {
            editor.status = Some((error, true));
        }

        editor.live = self.flight_manager.map().aircraft()
            .filter_map(|(_, aircraft)| Some((aircraft.name.clone(), self.flight_manager.live(scene, &aircraft.name)?)))
            .collect();
        self.fill_editor_scene(scene, &mut editor);
        app.flight_editor = editor;

        // The player flying again (respawned, or switched to a plane that
        // isn't wrecked) - out of the crash view.
        if self.is_dead {
            let player_wrecked = scene.content.nodes.get(self.flight_manager.player_id())
                .and_then(|node| node.get_behavior::<Plane>())
                .is_none_or(|plane| plane.wrecked);
            if !player_wrecked {
                self.is_dead = false;
                self.crash_time = 0.0;
                if let Some(camera) = scene.content.nodes.get_mut("camera").and_then(|node| node.get_behavior_mut::<Camera>()) {
                    camera.leave_wreck_view();
                }
            }
        }
    }

    /// What the editor shows of the scene this frame: every node (for the
    /// hierarchy), the shown animation's clock, and where the view is (for a
    /// new camera key).
    fn fill_editor_scene(&self, scene: &Scene, editor: &mut super::flight_editor::FlightEditor) {
        let map = self.flight_manager.map();
        editor.nodes = scene.content.nodes.iter().map(|node| {
            let kind = if map.find(&node.id).is_some() {
                NodeKind::Aircraft
            } else if map.node(&node.id).is_some() {
                NodeKind::MapNode
            } else if node.get_property::<SpawnedLive>().is_some() || self.flight_manager.is_store(&node.id) || node.id == ANIMATION_CAMERA {
                NodeKind::Runtime
            } else {
                NodeKind::Level
            };
            let parent = node.get_property::<Parent>();
            let world = node.get_property::<Transform3D>().copied().unwrap_or_default();
            SceneNodeInfo {
                id: node.id.clone(),
                kind,
                parent: parent.map(|parent| parent.id.clone()),
                space: parent.map(|parent| parent.space).unwrap_or(Space::Local),
                camera: node.get_property::<CameraLens>().is_some(),
                animated: map.animations.get(&node.id).is_some_and(|animations| !animations.is_empty()),
                world_position: world.position,
                world_rotation: degrees_from_rotation(&world.rotation),
            }
        }).collect();
        editor.playback = editor.current_animation.as_ref()
            .and_then(|(node, name)| self.animations.get(node, name))
            .map(|running| Playback { time: running.time, playing: running.playing, engaged: true })
            .unwrap_or_default();
        // The editor camera's pose - what a new camera key takes.
        let active = scene.cameras.active();
        editor.view_position = active.camera.position().coords;
        editor.view_rotation = Some(facing_with_up(active.camera.calc_forward_direction(), active.camera.up));
        editor.view_fov = active.projection.fovy;
        if editor.preview_camera.as_ref().is_some_and(|camera| scene.cameras.get(camera).is_none()) {
            editor.preview_camera = None;
        }
    }

    /// F10's editor: F1-F4 fly the editor camera to what they picked (it
    /// keeps looking the way it was), and from then on it keeps up with it
    /// as it moves - flying it about with the mouse moves round it. Until
    /// something else is picked (or focused from the hierarchy), or what it
    /// follows is gone.
    fn update_editor_follow(&mut self, scene: &mut Scene, app: &App, focus: &str) {
        if !app.editor_active {
            self.editor_follow = None;
            return;
        }
        let picked = ["camera_player", "camera_next_aircraft", "camera_next_ground_unit", "camera_next_missile"]
            .iter()
            .any(|action| input::is_action_just_pressed(action)) && !app.flight_editor.typing;
        let at = scene.world_transform(focus).map(|transform| transform.position);
        let Some(camera) = scene.content.nodes.get_mut(PLAYER_CAMERA).and_then(|node| node.get_behavior_mut::<Camera>()) else { return };
        if picked {
            if let Some(at) = at {
                let distance = if self.flight_manager.map().find(focus).is_some() { EDITOR_FOCUS_AIRCRAFT } else { EDITOR_FOCUS_MISSILE };
                camera.editor_focus(at, distance);
                self.editor_follow = Some((focus.to_owned(), at));
            }
            return;
        }
        match (&self.editor_follow, at) {
            (Some((id, last)), Some(at)) if id == focus => {
                camera.editor_shift(at - last);
                self.editor_follow = Some((id.clone(), at));
            }
            // Gone (a missile that blew up) or something else picked - it
            // stays where it is.
            (Some(_), _) => self.editor_follow = None,
            (None, _) => {}
        }
    }

    /// What the camera's on, for the editor's Telemetry window - an
    /// aircraft or a missile (not the player's own jet): how fast, how high,
    /// the G, what it's doing.
    fn telemetry(&mut self, scene: &Scene, delta_time: f32) -> Option<Telemetry> {
        let focus = self.camera_focus.clone();
        if focus.is_empty() || focus == self.flight_manager.player_id() {
            self.telemetry_last = None;
            return None;
        }
        let row = |label: &str, value: String| (label.to_owned(), value, false);
        let warn = |label: &str, value: String| (label.to_owned(), value, true);
        let heading = |velocity: Vector3<f32>| (velocity.x.atan2(velocity.z).to_degrees() + 360.0) % 360.0;

        if let Some(missile) = self.missiles.telemetry(&focus) {
            // Its G: how its velocity changed since last frame, gravity out.
            let g = match &self.telemetry_last {
                Some((id, last)) if *id == focus && delta_time > 1e-4 => {
                    let felt = (missile.velocity - last) / delta_time - Vector3::new(0.0, -9.81, 0.0);
                    Some(felt.magnitude() / 9.81)
                }
                _ => None,
            };
            self.telemetry_last = Some((focus.clone(), missile.velocity));
            let speed = missile.velocity.magnitude();
            let target_at = missile.target.as_deref().and_then(|target| scene.world_transform(target)).map(|transform| transform.position);
            let mut rows = vec![
                row("Speed", format!("{:.0} kt · M{:.2}", speed * 1.943_84, speed / 340.29)),
                row("Altitude", format!("{:.0} m", missile.position.y)),
                g.map(|g| if g > 30.0 { warn("G", format!("{g:.0} G")) } else { row("G", format!("{g:.1} G")) }).unwrap_or_else(|| row("G", "-".to_owned())),
                row("Heading", format!("{:.0}°", heading(missile.velocity))),
                row("Flying for", format!("{:.1} s", missile.time)),
                row("Motor", if missile.burning { "burning".to_owned() } else { "burnt out".to_owned() }),
            ];
            match (&missile.target, target_at) {
                (Some(target), Some(at)) => rows.push(row("Target", format!("{target} · {:.1} km", (at - missile.position).magnitude() / 1000.0))),
                (Some(target), None) => rows.push(row("Target", target.clone())),
                (None, _) => rows.push(warn("Target", "none - lost it".to_owned())),
            }
            return Some(Telemetry { title: focus.clone(), subtitle: format!("{} · fired by {}", missile.weapon, missile.shooter), rows });
        }

        self.telemetry_last = None;
        let (coalition, placement) = self.flight_manager.map().find(&focus)?;
        let live = self.flight_manager.live(scene, &focus)?;
        let node = scene.content.nodes.get(&focus);
        let state = node.and_then(|node| node.physics_state::<AircraftState>());
        let plane = node.and_then(|node| node.get_behavior::<Plane>());
        let flight_data = state.map(|state| &state.flight_data);
        let mut rows = vec![
            row("Speed", format!("{:.0} kt · M{:.2}", live.speed_kt, flight_data.map(|data| data.mach).unwrap_or(0.0))),
            row("Altitude", format!("{:.0} m", live.altitude)),
        ];
        let g_text = format!("{:+.1} G (commanded {:+.1})", live.g, live.g_command);
        rows.push(if live.g > 9.0 || live.g < -3.0 { warn("G", g_text) } else { row("G", g_text) });
        rows.push(row("Angle of attack", format!("{:.1}°", flight_data.map(|data| data.aoa_y).unwrap_or(0.0))));
        if let Some(plane) = plane {
            let afterburner = state.map(|state| state.afterburner).unwrap_or(0.0);
            rows.push(row("Throttle", format!("{:.0}%{}", plane.controls.throttle * 100.0, if afterburner > 0.01 { " · afterburner" } else { "" })));
        }
        rows.push(row("Heading", format!("{:.0}°", heading(flight_data.map(|data| data.velocity).unwrap_or_else(Vector3::z)))));
        rows.push(row("Flown by", live.flown_by.to_owned()));
        if let Some(waypoint) = live.waypoint {
            rows.push(row("Next point", format!("{:.1} km", (waypoint - live.position).magnitude() / 1000.0)));
        }
        if live.wrecked {
            rows.push(warn("State", "wrecked".to_owned()));
        }
        let who = if placement.player { "player".to_owned() } else { format!("{} pilot", placement.difficulty.label()) };
        Some(Telemetry { title: focus.clone(), subtitle: format!("{} · {} · {who}", placement.plane, coalition.name), rows })
    }

    /// Stick or keys asking the plane to fly. The editor's fly button (right
    /// mouse) is the camera's, so it doesn't count.
    fn player_flight_input() -> bool {
        if input::mouse_right_button_down() {
            return false;
        }
        ["pitch_up", "pitch_down", "roll_left", "roll_right", "rudder_left", "rudder_right", "throttle_up", "throttle_down"]
            .iter()
            .any(|action| input::is_action_pressed(action))
    }

    /// The player is flying while an animation has the plane and isn't
    /// playing: let go of that hold and fly the map's own plan from here.
    fn release_player_to_live(&mut self, scene: &mut Scene, app: &App, player_id: &str) {
        if app.flight_editor.typing || !Self::player_flight_input() || self.animations.playing_on(player_id) {
            return;
        }
        if scene.content.nodes.get(PLAYER_CAMERA).and_then(|node| node.get_behavior::<Camera>()).is_some_and(|camera| camera.editor_looking()) {
            return;
        }
        let placement = self.flight_manager.map().find(player_id).map(|(_, placement)| placement.clone());
        let stopped = self.animations.stop_paused(player_id);
        let Some(plane) = scene.content.nodes.get_mut(player_id).and_then(|node| node.get_behavior_mut::<Plane>()) else { return };
        if !stopped && !plane.held && !plane.scripted {
            return;
        }
        plane.held = false;
        plane.scripted = false;
        if let Some(placement) = placement {
            let (points, looped) = self.flight_manager.map().flight_plan(&placement);
            plane.set_path(&points, Speed::Hold(cruise_speed(&placement)), placement.difficulty, looped);
        }
    }

    /// An animation is playing with `lock_input` - the player has no
    /// controls meanwhile.
    fn sequence_locks_input(&self) -> bool {
        self.animations.locks_input(self.flight_manager.map())
    }

    /// Puts the right camera on screen:
    /// - in M's map, the map's view;
    /// - in F10's editor, the editor camera - or a camera it's looking
    ///   through ("View through", or the animation's camera with "Camera in
    ///   view");
    /// - otherwise the animation camera while a running animation's camera
    ///   keys drive it (`animating`);
    /// - the player's view the rest of the time, cut to as soon as a
    ///   cinematic camera lets go of the screen.
    fn update_view(&mut self, scene: &mut Scene, app: &App, animating: bool) {
        let preview = app.editor_active.then(|| {
            app.flight_editor.preview_camera.clone()
                .or_else(|| (app.flight_editor.timeline_preview && animating).then(|| ANIMATION_CAMERA.to_owned()))
        }).flatten();
        let (wanted, blend) = if app.tactical_map_active {
            (PLAYER_CAMERA.to_owned(), 0.0)
        } else if app.editor_active {
            (preview.clone().unwrap_or_else(|| PLAYER_CAMERA.to_owned()), 0.0)
        } else if animating {
            (ANIMATION_CAMERA.to_owned(), 0.0)
        } else {
            (PLAYER_CAMERA.to_owned(), 0.0)
        };
        let wanted = if scene.cameras.get(&wanted).is_some() { wanted } else { PLAYER_CAMERA.to_owned() };

        // Every camera drawn the whole screen's size (the player's sizes
        // itself).
        for node in scene.content.nodes.iter_mut() {
            if node.id == PLAYER_CAMERA {
                continue;
            }
            if let Some(lens) = node.get_property_mut::<CameraLens>() {
                lens.viewport = None;
            }
        }

        // Already on it - unless the camera on screen went away (a camera
        // node despawned with its parent and spawned again).
        if wanted == self.view_camera && scene.cameras.has_active_camera() {
            return;
        }
        if blend > 0.0 {
            scene.cameras.transition_to(&wanted, blend);
        } else {
            scene.select_camera(&wanted);
        }
        self.view_camera = wanted;
    }

    /// The level's animations' turn each frame, after every behavior (see
    /// SceneBehaviour::late_update): their clocks on, every animated node
    /// set to its keys, the paths followed, their events carried out, and
    /// the right camera on screen.
    fn late_update(&mut self, scene: &mut Scene, app: &mut App, physics_command_tx: Option<&Sender<PhysicsCommand>>) {
        if app.is_paused {
            return;
        }
        let frame = self.animations.update(app.time.delta_time, scene, self.flight_manager.map(), physics_command_tx);
        // Aircraft an animation's placing: nothing the physics says about
        // them counts meanwhile (see Plane::held) - and one an animation's
        // playing on feels no G (Plane::calm).
        for node in scene.content.nodes.iter_mut() {
            let held = scene.content.kinematic.contains(&node.id);
            let calm = frame.calm.contains(&node.id);
            if let Some(plane) = node.get_behavior_mut::<Plane>() {
                plane.held = held;
                plane.calm = calm;
            }
        }
        let was_hiding_hud = self.animation_hides_hud;
        self.animation_hides_hud = frame.hides_hud;
        for event in frame.events {
            match event {
                AnimAction::PlayAudio(folder) => app.audio.play_audio(folder),
                AnimAction::Subtitle { text, seconds } => self.subtitle_data.add_text(&text, (seconds * 1000.0) as u64, app),
                // Done by the animations themselves.
                AnimAction::StartAnimation { .. } | AnimAction::StopAnimation { .. } => {}
            }
        }
        // The animation camera where its keys put it.
        if let Some(pose) = frame.camera {
            if let Some(node) = scene.content.nodes.get_mut(ANIMATION_CAMERA) {
                node.set_property(Transform3D { position: pose.position, rotation: pose.rotation, scale: Vector3::new(1.0, 1.0, 1.0) });
                node.set_property(CameraLens::new(pose.fovy));
            }
        }
        self.update_view(scene, app, frame.camera.is_some());
        self.sync_hud_boot(scene, app, was_hiding_hud);
        // What the camera's on, while spectating - for the editor, or the
        // debug view (F5/F9) in game.
        let debug_view = app.render_physics.visible;
        let telemetry = if app.editor_active || debug_view { self.telemetry(scene, app.time.delta_time) } else { None };
        app.spectate_telemetry = if app.editor_active { None } else { telemetry.clone() };
        app.flight_editor.telemetry = telemetry;
        self.update_editor_gizmos(scene, app);
    }

    /// F10's editor's lines over the world - see play::editor_gizmos.
    /// F10's editor's lines over the world (see play::editor_gizmos) - and,
    /// in the editor or the debug view (F5/F9) while spectating, the path
    /// of what's followed.
    fn update_editor_gizmos(&self, scene: &Scene, app: &mut App) {
        let spectating = app.flight_editor.telemetry.is_some();
        if !app.editor_active && !spectating {
            app.editor_lines.dynamic.clear();
            return;
        }
        let active = scene.cameras.active();
        let to_pixels = app.window_manager.pixel_size.height as f32 / (app.window_manager.size.height as f32).max(1.0);
        let view = TacticalView {
            camera_position: active.camera.position().coords,
            fovy_deg: active.projection.fovy,
            screen_height: app.window_manager.size.height as f32 * to_pixels,
            focus: active.camera.position().coords,
        };
        let aspect = app.window_manager.size.width as f32 / (app.window_manager.size.height as f32).max(1.0);
        let editor = &app.flight_editor;
        let selected = editor.selected_node();
        let mut highlighted: Vec<&str> = selected.into_iter().collect();
        if let Some(preview) = editor.preview_camera.as_deref() {
            highlighted.push(preview);
        }
        let mut lines = if app.editor_active {
            super::editor_gizmos::lines(scene, self.flight_manager.map(), scene.cameras.active_name(), selected, &highlighted, &view, app.water.ocean.sea_level(), aspect)
        } else {
            Vec::new()
        };
        // What the camera's on (see telemetry): an aircraft's path - the one
        // its autopilot's flying - with a line to the point it's making for;
        // a missile's trail and a line to its target.
        if spectating && !self.camera_focus.is_empty() {
            let sea_level = app.water.ocean.sea_level();
            if let Some(missile) = self.missiles.telemetry(&self.camera_focus) {
                for pair in missile.trail.windows(2).chain(std::iter::once([*missile.trail.last().unwrap_or(&missile.position), missile.position].as_slice())) {
                    lines.push(LineInstance { start: pair[0].into(), end: pair[1].into(), color: [1.0, 0.6, 0.2, 0.9], width: 2.0 });
                }
                if let Some(target) = missile.target.as_deref().and_then(|target| scene.world_transform(target)) {
                    lines.push(LineInstance { start: missile.position.into(), end: target.position.into(), color: [1.0, 0.3, 0.3, 0.6], width: 1.5 });
                }
            } else if let Some(mut live) = self.flight_manager.live(scene, &self.camera_focus) {
                let route = scene.content.nodes.get(&self.camera_focus).and_then(|node| node.get_behavior::<Plane>()).and_then(Plane::route);
                let (points, looped) = match &route {
                    Some((points, next, looped)) => {
                        live.next_path_point = *next;
                        (points.iter().map(|point| (point.x, point.y, point.z)).collect::<Vec<_>>(), *looped)
                    }
                    None => self.flight_manager.map().find(&self.camera_focus).map(|(_, placement)| self.flight_manager.map().flight_plan(placement)).unwrap_or_default(),
                };
                tactical_map::flight_plan(&mut lines, &points, looped, Some(&live), sea_level, &view);
            }
        }
        // The shown animation's camera keys, each where it puts the camera.
        if let Some(animation) = editor.current_animation.as_ref().filter(|_| app.editor_active).and_then(|current| Some(current)).as_ref().and_then(|(node, name)| self.flight_manager.map().animation(node, name)) {
            let picked = editor.picked_camera_key();
            for (index, key) in animation.camera.iter().enumerate() {
                if let Some(pose) = animations::key_pose(key, scene) {
                    super::editor_gizmos::camera_key(&mut lines, pose.position, pose.rotation, pose.fovy, picked == Some(index), &view, aspect);
                }
            }
        }
        app.editor_lines.dynamic = lines;
    }

    /// Every aircraft still flying - where it is, how it's moving, how hot
    /// it is (its engine and afterburner, or its fire - see
    /// play::weapon::heat) - and those shot down and falling, still hot
    /// (what heat seekers see), if `with_falling`.
    fn aircraft_tracks(&self, scene: &Scene, with_falling: bool) -> Vec<TargetTrack> {
        self.flight_manager.map().aircraft()
            .filter_map(|(_, placement)| {
                let live = self.flight_manager.live(scene, &placement.name)?;
                if live.wrecked && !(with_falling && self.falling(scene, &placement.name)) {
                    return None;
                }
                let node = scene.content.nodes.get(&placement.name);
                let state = node.and_then(|node| node.physics_state::<AircraftState>());
                let velocity = state.map(|state| state.flight_data.velocity).unwrap_or_else(Vector3::zeros);
                let afterburner = state.map(|state| state.afterburner).unwrap_or(0.0);
                let plane = node.and_then(|node| node.get_behavior::<Plane>());
                let throttle = plane.map(|plane| plane.controls.throttle).unwrap_or(0.0);
                let burning = plane.is_some_and(|plane| plane.wrecked && !plane.in_water());
                let heat = heat::aircraft_heat(throttle, afterburner, burning);
                Some(TargetTrack { name: placement.name.clone(), position: live.position, velocity, heat, decoy: false })
            })
            .collect()
    }

    /// With a lock on something the camera can't see (off screen, or
    /// behind it): an arrow near the screen's edge pointing the way to turn
    /// to it - in every view, not over F10's map. Drawn into the HUD's
    /// screen lines, after update_hud has filled them.
    fn update_lock_arrow(&self, scene: &Scene, app: &mut App) {
        const COLOR: [f32; 4] = [0.0, 1.0, 0.3, 0.95];
        const WIDTH: f32 = 2.0;
        // How far in from the screen's edge it sits, and its size (px).
        const EDGE_INSET: f32 = 70.0;
        const LENGTH: f32 = 22.0;
        const HALF_WIDTH: f32 = 11.0;
        if (app.tactical_map_active || app.editor_active) || self.hud_hidden_by_g {
            return;
        }
        let Some(target) = self.fire_control.seeker_target.as_ref()
            .and_then(|name| scene.content.renderizable_instances.get(name))
            .map(|instance| instance.instance.transform.position)
        else { return };
        let active = scene.cameras.active();
        let (width, height) = (app.window_manager.size.width, app.window_manager.size.height);
        if app.camera_resources.world_to_screen(active, Point3::from(target), width, height).is_some() {
            // In view - its square shows where it is.
            return;
        }

        // Which way it is across the screen: the camera's own right and up.
        let forward = (active.camera.rotation_modifier * active.camera.calc_forward_direction()).normalize();
        let up = active.camera.rotation_modifier * active.camera.up;
        let Some(right) = forward.cross(&up).try_normalize(1e-6) else { return };
        let up = right.cross(&forward);
        let to_target = target - active.camera.position().coords;
        // Screen's y runs down; straight behind, point down (turn round).
        let direction = nalgebra::Vector2::new(to_target.dot(&right), -to_target.dot(&up))
            .try_normalize(1e-6)
            .unwrap_or_else(|| nalgebra::Vector2::new(0.0, 1.0));

        // From the middle out to just inside the edge, that way.
        let (half_width, half_height) = (width as f32 * 0.5, height as f32 * 0.5);
        let reach_x = if direction.x.abs() > 1e-6 { (half_width - EDGE_INSET) / direction.x.abs() } else { f32::MAX };
        let reach_y = if direction.y.abs() > 1e-6 { (half_height - EDGE_INSET) / direction.y.abs() } else { f32::MAX };
        let center = nalgebra::Vector2::new(half_width, half_height) + direction * reach_x.min(reach_y).max(0.0);
        let across = nalgebra::Vector2::new(-direction.y, direction.x);
        let tip = center + direction * LENGTH * 0.5;
        let base = center - direction * LENGTH * 0.5;
        let corners = [tip, base + across * HALF_WIDTH, base - across * HALF_WIDTH];

        let to_pixels = app.window_manager.pixel_size.height as f32 / (height as f32).max(1.0);
        for index in 0..3 {
            let (start, end) = (corners[index] * to_pixels, corners[(index + 1) % 3] * to_pixels);
            app.hud_screen_lines.dynamic.push(LineInstance { start: [start.x, start.y, 0.0], end: [end.x, end.y, 0.0], color: COLOR, width: WIDTH });
        }
    }

    /// F8's heat view - what an IR seeker sees: the world dimmed to grey
    /// (see update_g_vision) and every heat source on it - each aircraft
    /// (its engine, afterburner or fire) and the sun - as a glowing blob,
    /// bigger and whiter the brighter it looks from the camera, labelled
    /// with its heat and how bright it looks (the number a seeker compares,
    /// see play::weapon::heat). The blobs are drawn after the grey grade,
    /// so they keep their color.
    fn update_heat_view(&mut self, scene: &Scene, app: &mut App) {
        // Past this many meters from the camera - not the jet it's sitting in.
        const MIN_DISTANCE: f32 = 30.0;
        app.heat_screen_lines.dynamic.clear();
        if input::is_action_just_pressed("toggle_heat_view") {
            self.heat_view = !self.heat_view;
        }
        for index in 0..play_ui::HEAT_LABELS {
            if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &play_ui::heat_label_key(index)) {
                node.set_active(false);
            }
        }
        if !self.heat_view || (app.tactical_map_active || app.editor_active) {
            return;
        }

        let active = scene.cameras.active();
        let eye = active.camera.position().coords;
        let (width, height) = (app.window_manager.size.width, app.window_manager.size.height);
        let on_screen = |point: Vector3<f32>| app.camera_resources.world_to_screen(active, Point3::from(point), width, height)
            .map(|point| (point.x as f32, point.y as f32));
        // (where on screen, how bright it looks, its label)
        let mut spots: Vec<((f32, f32), f32, String)> = Vec::new();
        for track in self.aircraft_tracks(scene, true).into_iter().chain(self.countermeasures.heat_tracks()) {
            let distance = (track.position - eye).magnitude();
            if distance < MIN_DISTANCE {
                continue;
            }
            if let Some(at) = on_screen(track.position) {
                let apparent = heat::apparent(track.heat, distance);
                spots.push((at, apparent, format!("{:.1}  ({apparent:.2})", track.heat)));
            }
        }
        if let Some(at) = Self::sun_direction(scene).and_then(|sun| on_screen(eye + sun * 10_000.0)) {
            spots.push((at, heat::SUN_APPARENT, format!("SUN  ({:.2})", heat::SUN_APPARENT)));
        }
        // Dimmest first, so the brightest draw on top.
        spots.sort_by(|a, b| a.1.total_cmp(&b.1));

        let to_pixels = app.window_manager.pixel_size.height as f32 / (height as f32).max(1.0);
        let (label_width, label_height) = play_ui::HEAT_LABEL_SIZE;
        for (index, ((x, y), apparent, text)) in spots.into_iter().enumerate() {
            // 0 (dim) .. 1 (searing) - halfway at an apparent 1, the sun.
            let glow = apparent / (apparent + 1.0);
            let radius = 4.0 + 30.0 * glow;
            let color = heat_color(glow);
            heat_blob(&mut app.heat_screen_lines.dynamic, (x, y), radius * 2.0, [color[0], color[1], color[2], 0.25 * glow + 0.05], to_pixels);
            heat_blob(&mut app.heat_screen_lines.dynamic, (x, y), radius, [color[0], color[1], color[2], 0.95], to_pixels);

            let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &play_ui::heat_label_key(index)).filter(|_| index < play_ui::HEAT_LABELS) else { continue };
            node.set_active(true);
            node.transform.x = x - label_width / 2.0;
            node.transform.y = y + radius + 2.0;
            node.transform.rect.left = node.transform.x;
            node.transform.rect.top = node.transform.y;
            node.transform.rect.right = node.transform.x + label_width;
            node.transform.rect.bottom = node.transform.y + label_height;
            if let Some(label) = node.as_label_mut() {
                label.set_text(&mut app.ui.text.font_system, &text, true);
            }
        }
        app.ui.has_changed = true;
    }

    /// Which way the sun in the sky is - the "sun" node's position, read as
    /// a direction the way the renderer does (see App's lighting update).
    fn sun_direction(scene: &Scene) -> Option<Vector3<f32>> {
        scene.content.renderizable_instances.get("sun")
            .and_then(|sun| sun.instance.transform.position.try_normalize(1e-6))
    }

    /// The bots defending themselves: each one with flares bursts them once
    /// it's noticed a heat seeker homing on it - how quickly, and how often
    /// again, its pilot's difficulty says (see play::ai_defense) - and its
    /// bursts in progress go on. Once a frame the game runs.
    fn update_ai_countermeasures(&mut self, scene: &mut Scene, app: &mut App) {
        let delta_time = app.time.delta_time;
        let homing = self.missiles.heat_seekers_homing();
        let bots: Vec<(String, Difficulty)> = self.flight_manager.map().aircraft()
            .filter(|(_, placement)| !placement.player)
            .map(|(_, placement)| (placement.name.clone(), placement.difficulty))
            .collect();
        for (name, difficulty) in bots {
            let flying = scene.content.nodes.get(&name).and_then(|node| node.get_behavior::<Plane>()).is_some_and(|plane| !plane.wrecked);
            if !flying {
                self.ai_defense.forget(&name);
                continue;
            }
            let incoming: Vec<String> = homing.iter().filter(|(_, target)| *target == name).map(|(id, _)| id.clone()).collect();
            let burst = self.ai_defense.update(&name, difficulty, &incoming, delta_time);
            let Some(set) = self.flight_manager.countermeasures_mut(&name) else { continue };
            let mut released = Vec::new();
            if burst {
                set.release = Release::Flares;
                released.extend(set.start_program());
            }
            released.extend(set.update(delta_time));
            if released.is_empty() {
                continue;
            }
            let Some(transform) = scene.content.renderizable_instances.get(&name).map(|instance| instance.instance.transform) else { continue };
            let velocity = scene.content.nodes.get(&name)
                .and_then(|node| node.physics_state::<AircraftState>())
                .map(|state| state.flight_data.velocity)
                .unwrap_or_else(Vector3::zeros);
            for release in released {
                self.countermeasures.release(scene, app, release, transform.position, transform.rotation, velocity);
            }
        }
    }

    /// Every aircraft that's just ended: an explosion where it hit the
    /// ground, a splash where it went into the sea (on the water's
    /// surface) - and the ones already playing kept going.
    fn show_crashes(&mut self, scene: &mut Scene, app: &mut App) {
        let names: Vec<String> = self.flight_manager.map().aircraft().map(|(_, placement)| placement.name.clone()).collect();
        for name in names {
            let Some(crash) = scene.content.nodes.get_mut(&name).and_then(|node| node.get_behavior_mut::<Plane>()).and_then(|plane| plane.crash.take()) else { continue };
            let Some(mut at) = scene.content.renderizable_instances.get(&name).map(|instance| instance.instance.transform.position) else { continue };
            if crash == Crash::Water {
                at.y = app.water.ocean.sea_level();
            }
            self.crash_effects.spawn(scene, app, crash, at);
        }
        self.crash_effects.update(scene, app, app.time.delta_time);
    }

    /// Whether the aircraft `name` is shot down and still in the air.
    fn falling(&self, scene: &Scene, name: &str) -> bool {
        scene.content.nodes.get(name).and_then(|node| node.get_behavior::<Plane>()).is_some_and(Plane::going_down)
    }

    /// Flies every missile one frame and wrecks what they hit - early in
    /// the frame, before the camera reads where they are (see `update`), so
    /// a camera riding one moves with it, not a frame behind.
    fn fly_missiles(&mut self, scene: &mut Scene, app: &mut App) {
        // Falling wrecks too: a missile chasing one keeps on its heat. And
        // the flares burning - heat sources it can be pulled onto.
        self.countermeasures.update(scene, app, app.time.delta_time);
        let mut aircraft = self.aircraft_tracks(scene, true);
        aircraft.extend(self.countermeasures.heat_tracks());
        let sun = Self::sun_direction(scene);
        let events = self.missiles.update(scene, app, &aircraft, &self.world_collision, sun, app.time.delta_time);
        for kill in events.destroyed {
            // Only aircraft (not flares), and only once - not a wreck again.
            let Some(plane) = scene.content.nodes.get_mut(&kill.victim).and_then(|node| node.get_behavior_mut::<Plane>()) else { continue };
            if plane.wrecked {
                continue;
            }
            println!("Splash - {} destroyed by {}'s {}", kill.victim, kill.shooter, kill.weapon);
            plane.shot_down = true;
            // Killer and victim in their coalitions' colors, the rest plain -
            // each piece measured now, for its row's box to fit them.
            const PLAIN: UiColor = UiColor::Rgb(230, 230, 230);
            let texts = [
                (kill.shooter.clone(), self.team_color(&kill.shooter)),
                ("killed".to_owned(), PLAIN),
                (kill.victim.clone(), self.team_color(&kill.victim)),
                (format!("with {}", weapon_label(&kill.weapon)), PLAIN),
            ];
            let pieces = texts.map(|(text, color)| {
                let width = text_width(&mut app.ui.text.font_system, &text, play_ui::KILL_FEED_FONT_SIZE, HUD_FONT);
                (text, color, width)
            });
            self.kill_feed.push(KillFeedEntry { pieces, seconds_left: KILL_FEED_SECONDS });
        }
    }

    /// The aircraft `name`'s coalition's color - grey when it isn't in the
    /// map.
    fn team_color(&self, name: &str) -> UiColor {
        match self.flight_manager.map().find(name) {
            Some((coalition, _)) => UiColor::Rgb(coalition.color.0, coalition.color.1, coalition.color.2),
            None => UiColor::Rgb(180, 180, 180),
        }
    }

    /// The kill feed, top right: the latest kills (newest on top), each on
    /// its own dark box for KILL_FEED_SECONDS - not over F10's map. Every
    /// frame.
    fn update_kill_feed(&mut self, app: &mut App) {
        const ROW_GAP: f32 = 4.0;
        const MARGIN: f32 = 16.0;
        if !app.is_paused {
            for entry in &mut self.kill_feed {
                entry.seconds_left -= app.time.delta_time;
            }
            self.kill_feed.retain(|entry| entry.seconds_left > 0.0);
        }
        // Oldest dropped once there are more than it has room for.
        let overflow = self.kill_feed.len().saturating_sub(play_ui::KILL_FEED_ROWS);
        self.kill_feed.drain(..overflow);

        let shown = !(app.tactical_map_active || app.editor_active);
        let height = play_ui::KILL_FEED_ROW_HEIGHT;
        let right = app.window_manager.size.width as f32 - MARGIN;
        // Puts a node at (x, y), `width` wide - shown or hidden.
        let place = |node: &mut UiNode, x: f32, y: f32, width: f32, visible: bool| {
            node.set_active(visible);
            node.transform.x = x;
            node.transform.y = y;
            node.transform.rect.left = x;
            node.transform.rect.top = y;
            node.transform.rect.right = x + width;
            node.transform.rect.bottom = y + height;
        };
        for row in 0..play_ui::KILL_FEED_ROWS {
            // Newest on top.
            let entry = self.kill_feed.iter().rev().nth(row).filter(|_| shown);
            let top = MARGIN + row as f32 * (height + ROW_GAP);
            // The box just fits its text, against the right edge.
            let text_width = entry.map(|entry| entry.pieces.iter().map(|(_, _, width)| width).sum::<f32>() + play_ui::KILL_FEED_GAP * (play_ui::KILL_FEED_PIECES - 1) as f32).unwrap_or(0.0);
            let box_width = text_width + 2.0 * play_ui::KILL_FEED_PADDING;
            let left = right - box_width;
            if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &play_ui::kill_feed_key(row)) {
                place(node, left, top, box_width, entry.is_some());
            }
            let mut x = left + play_ui::KILL_FEED_PADDING;
            for piece in 0..play_ui::KILL_FEED_PIECES {
                let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &play_ui::kill_feed_piece_key(row, piece)) else { continue };
                let Some((text, color, width)) = entry.map(|entry| &entry.pieces[piece]) else {
                    node.set_active(false);
                    continue;
                };
                // A pixel spare, so nothing's clipped by rounding.
                place(node, x, top, width + 2.0, true);
                node.update_style(|style| style.set_text_color(*color));
                if let Some(label) = node.as_label_mut() {
                    label.set_text(&mut app.ui.text.font_system, text, true);
                }
                x += width + play_ui::KILL_FEED_GAP;
            }
        }
        app.ui.has_changed = true;
    }

    /// The weapons, every frame the game runs: the master mode keys (1 NAV,
    /// 2 air-to-air - again for DOGFIGHT/BVR - 3 air-to-ground), the seeker
    /// and its lock key (see play::weapon::fire_control), the trigger, the
    /// missiles' plumes (they're flown by fly_missiles), and the HUD's symbols and
    /// weapons line. `has_control`: the player can fly (not in a scripted
    /// sequence, not typing in the editor).
    fn update_weapons(&mut self, scene: &mut Scene, app: &mut App, has_control: bool) {
        let delta_time = app.time.delta_time;
        let player_id = self.flight_manager.player_id().to_owned();
        let aircraft = self.aircraft_tracks(scene, true);

        // The data of every weapon hung anywhere - read once.
        for (_, placement) in self.flight_manager.map().aircraft() {
            for weapon in placement.loadout.values() {
                if !self.weapon_specs.contains_key(weapon) {
                    if let Ok(spec) = WeaponSpec::read(weapon) {
                        self.weapon_specs.insert(weapon.clone(), spec);
                    }
                }
            }
        }

        // Plumes: off on the pylons, lit on the missiles burning.
        let mut model_files: Vec<String> = self.weapon_specs.values()
            .map(|spec| Model::from_file(&spec.model.model).model_ref)
            .collect();
        model_files.sort_unstable();
        model_files.dedup();
        self.missiles.update_plumes(app, &model_files, delta_time);

        // The player's jet.
        let player_alive = scene.content.nodes.get(&player_id)
            .and_then(|node| node.get_behavior::<Plane>())
            .is_some_and(|plane| !plane.wrecked && !plane.pilot.is_unconscious());
        let own = aircraft.iter().find(|track| track.name == player_id).and_then(|track| {
            let rotation = scene.content.renderizable_instances.get(&player_id)?.instance.transform.rotation;
            Some(OwnShip { position: track.position, rotation, velocity: track.velocity })
        });
        let player_coalition = self.flight_manager.coalition_of(&player_id).map(str::to_owned);
        let contacts: Vec<Contact> = aircraft.iter()
            .filter(|track| track.name != player_id && self.flight_manager.coalition_of(&track.name).map(str::to_owned) != player_coalition)
            .map(|track| Contact { name: track.name.clone(), position: track.position, velocity: track.velocity, falling: self.falling(scene, &track.name) })
            .collect();

        // The weapon list: OFF, then each kind of weapon still on the jet,
        // in pylon order - with how many are left.
        let hung = self.flight_manager.hung_weapons(&player_id);
        let mut kinds: Vec<(String, usize)> = Vec::new();
        for (_, weapon, _) in &hung {
            match kinds.iter_mut().find(|(kind, _)| kind == weapon) {
                Some((_, count)) => *count += 1,
                None => kinds.push((weapon.clone(), 1)),
            }
        }
        // The last one fired: back to OFF.
        if self.selected_weapon.as_ref().is_some_and(|selected| !kinds.iter().any(|(kind, _)| kind == selected)) {
            self.selected_weapon = None;
        }
        // The next one down the list, round to OFF again.
        if has_control && player_alive && input::is_action_just_pressed("next_weapon") {
            let current = self.selected_weapon.as_ref().and_then(|selected| kinds.iter().position(|(kind, _)| kind == selected));
            let next = match current { None => 0, Some(index) => index + 1 };
            self.selected_weapon = kinds.get(next).map(|(kind, _)| kind.clone());
            self.fire_control.drop_locks();
        }
        self.weapon_list = kinds.iter()
            .map(|(kind, count)| (format!("{} x{count}", weapon_label(kind)), self.selected_weapon.as_ref() == Some(kind)))
            .collect();

        // The selected weapon's data, and every pylon carrying one.
        let selected = self.selected_weapon.as_ref()
            .and_then(|weapon| Some((weapon.clone(), self.weapon_specs.get(weapon)?.clone())));
        let ready: Vec<(String, Vector3<f32>)> = selected.as_ref().map(|(weapon, _)| hung.iter()
            .filter(|(_, hung_weapon, _)| hung_weapon == weapon)
            .map(|(pylon, _, offset)| (pylon.clone(), *offset))
            .collect()).unwrap_or_default();
        // What its seeker looks with - a guided missile's.
        let seeker = selected.as_ref()
            .filter(|(_, spec)| spec.category == WeaponCategory::Missile && spec.guidance.seeker != Seeker::None)
            .map(|(_, spec)| spec.guidance.clone());

        self.range_cue = None;
        let Some(own) = own.filter(|_| player_alive) else {
            self.fire_control.drop_locks();
            self.weapons_status = String::new();
            self.countermeasures_status = String::new();
            self.countermeasures_column = None;
            self.bingo_latched = false;
            self.bingo_warn_until = None;
            app.hud_lines.dynamic.clear();
            return;
        };

        // Countermeasures: the key choosing flares, chaff or both, one
        // release, the burst program - and the program's releases due now.
        let mut released = Vec::new();
        let (status, column) = match self.flight_manager.countermeasures_mut(&player_id) {
            Some(set) => {
                if has_control && input::is_action_just_pressed("countermeasure_mode") {
                    set.release = set.release.next();
                    self.cms_cue = Some((set.release.label().to_owned(), self.game_time));
                }
                if has_control && input::is_action_just_pressed("countermeasure_single") {
                    released.extend(set.release_one());
                }
                if has_control && input::is_action_just_pressed("countermeasure_program") {
                    released.extend(set.start_program());
                }
                released.extend(set.update(delta_time));
                (format!("FLR {}  CHF {}  {}", set.flares, set.chaff, set.release.label()), Some((set.flares, set.chaff, set.release)))
            }
            None => (String::new(), None),
        };
        self.countermeasures_status = status;
        self.countermeasures_column = column;
        let (fuel_kg, bingo_kg) = scene.content.nodes.get(&player_id)
            .and_then(|node| node.get_behavior::<Plane>())
            .map(|plane| (plane.fuel().0, plane.bingo_kg))
            .unwrap_or((0.0, None));
        let in_bingo = bingo_kg.is_some_and(|bingo| fuel_kg <= bingo && fuel_kg > 0.0);
        if in_bingo && !self.bingo_latched {
            self.bingo_warn_until = Some(self.game_time + 4.0);
        }
        if !in_bingo || fuel_kg <= 0.0 {
            self.bingo_warn_until = None;
        }
        self.bingo_latched = in_bingo;
        for release in released {
            self.countermeasures.release(scene, app, release, own.position, own.rotation, own.velocity);
        }
        // The lock key: lets the lock go, or locks the seeker on what's on
        // the nose (in the diamond).
        if has_control && input::is_action_just_pressed("toggle_lock") {
            if self.fire_control.seeker_target.is_some() {
                self.fire_control.drop_locks();
            } else if let Some(seeker) = &seeker {
                self.fire_control.lock(&own, &contacts, seeker);
            }
        }
        self.fire_control.update(&own, &contacts, seeker.as_ref(), delta_time);
        let locked = self.fire_control.locked(&contacts, &own).map(str::to_owned);
        let locked_contact = locked.as_ref().and_then(|name| contacts.iter().find(|contact| contact.name == *name));
        let shoot = match (locked_contact, &selected) {
            (Some(contact), Some((_, spec))) => fire_control::in_range(&own, contact, spec.guidance.lock_range),
            _ => false,
        };
        self.range_cue = match (locked_contact, &selected) {
            (Some(contact), Some((_, spec))) => Some(RangeCue {
                range: (contact.position - own.position).magnitude(),
                min_range: fire_control::MIN_RANGE,
                max_range: fire_control::max_range(&own, contact, spec.guidance.lock_range),
                scale_top: spec.guidance.lock_range,
                in_range: shoot,
            }),
            _ => None,
        };

        // The trigger: the next one off - from the other side than the last,
        // outboard first - at whatever the seeker's locked on (or nothing:
        // it flies on straight).
        if has_control && input::is_action_just_pressed("fire_weapon") {
            let outboard = |a: &&(String, Vector3<f32>), b: &&(String, Vector3<f32>)| a.1.x.abs().total_cmp(&b.1.x.abs());
            let other_side = ready.iter()
                .filter(|(_, offset)| offset.x.signum() != self.last_fired_side)
                .max_by(outboard);
            let next = other_side.or_else(|| ready.iter().max_by(outboard)).cloned();
            match (next, &selected) {
                (Some((pylon, offset)), Some((_, spec))) if spec.category == WeaponCategory::Missile => {
                    if let Some((position, rotation)) = self.flight_manager.release(scene, app, &player_id, &pylon) {
                        self.last_fired_side = offset.x.signum();
                        // Its lock - or, without one, whatever its seeker
                        // sees in the diamond coming off the rail.
                        let target = locked.clone().or_else(|| {
                            seeker.as_ref().and_then(|seeker| fire_control::on_the_nose(&own, &contacts, seeker))
                        });
                        self.missiles.launch(scene, app, &player_id, spec.clone(), position, rotation, own.velocity, target);
                    }
                }
                (_, Some((_, spec))) => println!("{} can't be fired yet - only missiles fly so far", weapon_label(&spec.id)),
                (_, None) => println!("No weapon selected"),
            }
        }

        // The HUD's weapons line.
        self.weapons_status = match &selected {
            Some((weapon, _)) => {
                let state = if shoot { "SHOOT" } else if locked.is_some() { "LOCK" } else { "" };
                format!("{}  {state}", weapon_label(weapon))
            }
            None => "WPN OFF".to_owned(),
        };

        // The HUD's symbol: locked, a square on the target (flashing while
        // it's in range - fire!) and nothing else; unlocked, the seeker's
        // diamond out ahead where it looks - the nose.
        app.hud_lines.dynamic.clear();
        if seeker.is_none() || self.hud_hidden_by_g {
            return;
        }
        let up = own.rotation * Vector3::y();
        let target_position = self.fire_control.seeker_target.as_ref()
            .and_then(|name| contacts.iter().find(|contact| contact.name == *name))
            .map(|contact| contact.position);
        match target_position {
            Some(target_position) => {
                let flash_off = shoot && (self.game_time * 4.0).fract() > 0.5;
                if !flash_off {
                    Self::hud_symbol(scene, app, target_position, up, 18.0, false);
                }
            }
            None => {
                let seeker_point = own.position + self.fire_control.seeker_direction * 3000.0;
                Self::hud_symbol(scene, app, seeker_point, up, 11.0, true);
            }
        }
    }

    /// The flight HUD, laid out in a box in the middle of the screen - the
    /// same in the cockpit or out of it:
    /// - a vertical line either side, its ends turned in toward the middle;
    ///   the speed in a box outside the left one, the altitude in a box
    ///   outside the right one - each box against the HUD's edge;
    /// - between the lines, the horizon line and a shorter line every
    ///   LADDER_STEP_DEG above and below it, laid out toward where the jet
    ///   points (they tilt and slide as it banks and pitches, and are only
    ///   seen looking forward) - cut off at the lines and their ends;
    /// - with a lock, the right line is the missile's range scale: ticks at
    ///   its shortest and longest range, an arrow at the target's range;
    /// - over the box, the heading tape.
    /// In the Fox-2 mode (a heat-seeker selected) the horizon and ladder
    /// are left out - the seeker's diamond has the middle - and the G shows
    /// bottom left.
    fn update_hud(&self, scene: &Scene, app: &mut App) {
        const COLOR: [f32; 4] = [0.0, 1.0, 0.3, 0.9];
        const WIDTH: f32 = 1.5;
        const LADDER_STEP_DEG: i32 = 5;
        // Below the horizon the ladder's lines are dashed: each dash and the
        // space after it (px).
        const DASH: f32 = 5.0;
        const DASH_GAP: f32 = 4.0;
        // Between a ladder line's outer end and the middle of its number (px).
        const LABEL_OFFSET: f32 = 12.0;
        // How far they're laid out (m) - far enough to read as the sky.
        const DISTANCE: f32 = 10_000.0;
        // The HUD's size (fractions of the screen's width and height).
        const FRAME_WIDTH: f32 = 1.0 / 3.0;
        const FRAME_HEIGHT: f32 = 0.5;
        // Between a readout's box and its side line (px).
        const READOUT_GAP: f32 = 6.0;
        // How far the side lines' ends turn in, square to them (px).
        const END_IN: f32 = 10.0;
        // The range scale's ticks and arrow (px).
        const TICK: f32 = 8.0;
        const ARROW: f32 = 7.0;
        // The heading tape over the box: degrees either side of the middle
        // it shows (across the horizon's width), how far over the box its
        // bottom is, its ticks every 10° and 5° (standing that much over its
        // bottom), and the box round the heading in its middle - the tape's
        // own numbers left out where they'd run into it (px).
        const HEADING_SPAN_DEG: f32 = 30.0;
        const HEADING_GAP: f32 = 4.0;
        const HEADING_TICKS_UP: f32 = 6.0;
        const HEADING_MAJOR_TICK: f32 = 8.0;
        const HEADING_MINOR_TICK: f32 = 4.0;
        const HEADING_BOX: (f32, f32) = (42.0, 22.0);

        app.hud_horizon_lines.dynamic.clear();
        app.hud_screen_lines.dynamic.clear();

        let (screen_width, screen_height) = (app.window_manager.size.width as f32, app.window_manager.size.height as f32);
        let (frame_width, frame_height) = (screen_width * FRAME_WIDTH, screen_height * FRAME_HEIGHT);
        let (frame_x, frame_y) = ((screen_width - frame_width) * 0.5, (screen_height - frame_height) * 0.5);
        let (readout_width, readout_height) = play_ui::HUD_READOUT_SIZE;
        let left_line = frame_x + readout_width + READOUT_GAP;
        let right_line = frame_x + frame_width - readout_width - READOUT_GAP;
        let line_top = frame_y;
        let line_bottom = frame_y + frame_height;

        let game_ui_shown = Ui::get_ui_node(&mut app.ui.renderizable_elements, "game_ui").is_some_and(|node| node.is_active);
        // Only from the cockpit of the player's own jet - it's the pilot's
        // HUD, not the camera's.
        let in_cockpit = self.camera_focus == self.flight_manager.player_id()
            && scene.content.nodes.get("camera").and_then(|node| node.get_behavior::<Camera>()).is_some_and(Camera::is_cockpit);
        let shown = game_ui_shown && !self.hud_hidden_by_g && in_cockpit;
        // The HUD's mode, from the weapon selected: Fox-2 (a heat-seeking
        // missile) drops the horizon and ladder - the seeker's diamond has
        // the middle - and shows the G, bottom left. Otherwise the normal
        // HUD.
        let fox2 = self.selected_weapon.as_ref()
            .and_then(|weapon| self.weapon_specs.get(weapon))
            .is_some_and(|spec| spec.category == WeaponCategory::Missile && spec.guidance.seeker == Seeker::InfraRed);
        let g = scene.content.nodes.get(self.flight_manager.player_id())
            .and_then(|node| node.physics_state::<AircraftState>())
            .map(|state| state.flight_data.g_meter)
            .unwrap_or(1.0);
        if let Some(label) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "hud_g").and_then(|node| node.as_label_mut()) {
            label.set_text(&mut app.ui.text.font_system, &format!("{g:.1}G"), true);
        }
        // Speed on the left, altitude on the right. No captions.
        let speed_x = frame_x + readout_width * 0.5;
        let altitude_x = frame_x + frame_width - readout_width * 0.5;
        let middle_y = frame_y + frame_height * 0.5;
        // The heading tape, over the box: its bottom, its ticks' bottom, its
        // numbers' middle - and the heading's, a little higher, its digits
        // being bigger.
        let middle_x = (left_line + right_line) * 0.5;
        let tape_bottom = line_top - HEADING_GAP;
        let ticks_bottom = tape_bottom - HEADING_TICKS_UP;
        let label_y = ticks_bottom - HEADING_MAJOR_TICK - 2.0 - play_ui::HUD_SMALL_LABEL_SIZE.1 * 0.5;
        let heading_y = label_y - 4.0;
        let g_y = line_bottom - readout_height * 0.5;
        let weapon_y = line_bottom;
        // The wheels sit further right. Fuel and radar altitude are a
        // left-aligned pair just outside the right line's bottom corner.
        let side_x = frame_x + frame_width + 14.0 + 24.0;
        let wheels_y = middle_y;
        // Center of the square's bottom half.
        let warning_y = (line_top + line_bottom * 3.0) * 0.25;
        const CORNER_WIDTH: f32 = 130.0;
        let corner_x = right_line + 8.0 + CORNER_WIDTH * 0.5;
        // The pair sits outside the line, its bottom edge on the line's bottom.
        let radar_y = line_bottom - readout_height * 0.5;
        let fuel_y = radar_y - readout_height - 2.0;
        let cms_y = line_bottom - 40.0;
        let player_id = self.flight_manager.player_id();
        let (fuel_kg, fuel_capacity, radar_warn_m) = scene.content.nodes.get(player_id)
            .and_then(|node| node.get_behavior::<Plane>())
            .map(|plane| { let (kg, capacity) = plane.fuel(); (kg, capacity, plane.radar_warn_m) })
            .unwrap_or((0.0, 0.0, super::map::DEFAULT_RADAR_WARN_M));
        let radar_altitude = scene.content.renderizable_instances.get(player_id)
            .map(|instance| self.world_collision.height_below(instance.instance.transform.position, app.water.ocean.sea_level()))
            .unwrap_or(0.0);
        let low_altitude = shown && radar_altitude < radar_warn_m;
        let bingo_text = self.bingo_warn_until.is_some_and(|until| self.game_time < until);
        // The cue after a countermeasure setting change: a few fast blinks,
        // a short hold, then a fade.
        let cms_cue = self.cms_cue.as_ref().and_then(|(text, started)| {
            let age = self.game_time - *started;
            const BLINK_END: f64 = 0.5;
            const HOLD_END: f64 = 1.7;
            const FADE_END: f64 = 2.4;
            if !(0.0..FADE_END).contains(&age) {
                return None;
            }
            let (on, alpha) = if age < BLINK_END {
                ((age * 8.0).fract() < 0.5, 1.0)
            } else if age < HOLD_END {
                (true, 1.0)
            } else {
                (true, (1.0 - ((age - HOLD_END) / (FADE_END - HOLD_END)) as f32).clamp(0.0, 1.0))
            };
            Some((text.clone(), on, alpha))
        });
        // About twice a second. Low radar altitude blinks WARN and the
        // radar figure. For the four seconds after bingo, BINGO and the
        // fuel figure blink too, and the fuel figure does not blink again.
        const ALTITUDE_BLINK_SECONDS: f64 = 0.5;
        let blink_on = (self.game_time % ALTITUDE_BLINK_SECONDS) < ALTITUDE_BLINK_SECONDS * 0.5;
        let radar_red = low_altitude && blink_on;
        let fuel_red = bingo_text && blink_on;
        let warning_shown = shown && blink_on && (bingo_text || low_altitude);
        let (warn_width, warn_height) = play_ui::HUD_WARN_SIZE;
        let placed = [
            ("hud_speed", speed_x, middle_y, shown, readout_width, readout_height),
            ("hud_altitude", altitude_x, middle_y, shown, readout_width, readout_height),
            ("hud_heading", middle_x, heading_y, shown, readout_width, readout_height),
            ("hud_g", speed_x, g_y, shown && fox2, readout_width, readout_height),
            ("hud_weapon", middle_x, weapon_y, shown && self.selected_weapon.is_some(), 160.0, readout_height),
            ("hud_altitude_warning", middle_x, warning_y, warning_shown, warn_width, warn_height),
            ("hud_corner_fuel", corner_x, fuel_y, shown && fuel_capacity > 0.0, CORNER_WIDTH, readout_height),
            ("hud_radar_altitude", corner_x, radar_y, shown, CORNER_WIDTH, readout_height),
            ("hud_cms_cue", middle_x, cms_y, shown && cms_cue.as_ref().is_some_and(|(_, on, _)| *on), 160.0, readout_height),
        ];
        for (key, x, y, visible, width, height) in placed {
            let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, key) else { continue };
            node.set_active(visible);
            if key == "hud_radar_altitude" {
                node.style.text_color = Some(if radar_red { UiColor::Rgb(255, 64, 64) } else { UiColor::Rgb(0, 255, 77) });
                if let Some(label) = node.as_label_mut() {
                    label.set_text(&mut app.ui.text.font_system, &format!("R: {radar_altitude:.0}"), true);
                }
            }
            if key == "hud_corner_fuel" {
                node.style.text_color = Some(if fuel_red { UiColor::Rgb(255, 64, 64) } else { UiColor::Rgb(0, 255, 77) });
                if let Some(label) = node.as_label_mut() {
                    label.set_text(&mut app.ui.text.font_system, &format!("FUEL: {fuel_kg:.0}"), true);
                }
            }
            if key == "hud_weapon" {
                if let Some(label) = node.as_label_mut() {
                    let name = self.selected_weapon.as_deref().map(weapon_label).unwrap_or_default();
                    label.set_text(&mut app.ui.text.font_system, &name, true);
                }
            }
            if key == "hud_altitude_warning" {
                if let Some(label) = node.as_label_mut() {
                    label.set_text(&mut app.ui.text.font_system, if bingo_text { "BINGO" } else { "WARN" }, true);
                }
            }
            if key == "hud_cms_cue" {
                if let Some((text, _, alpha)) = &cms_cue {
                    node.style.text_color = Some(UiColor::Rgba(0, 255, 77, (alpha * 255.0) as u8));
                    if let Some(label) = node.as_label_mut() {
                        label.set_text(&mut app.ui.text.font_system, text, true);
                    }
                }
            }
            node.transform.x = x - width / 2.0;
            node.transform.y = y - height / 2.0;
            node.transform.width = width;
            node.transform.height = height;
            node.transform.rect.left = node.transform.x;
            node.transform.rect.top = node.transform.y;
            node.transform.rect.right = node.transform.x + width;
            node.transform.rect.bottom = node.transform.y + height;
        }
        for (key, visible) in [
            ("game_ui/hud_stores", shown),
            ("game_ui/hud_stores/weapons", shown && !self.weapon_list.is_empty()),
        ] {
            if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, key) {
                node.set_active(visible);
            }
        }
        // The ladder's and the heading tape's numbers - hidden until placed
        // below.
        let pitch_keys = (0..play_ui::HUD_PITCH_LABELS).map(play_ui::hud_pitch_label_key);
        let heading_keys = (0..play_ui::HUD_HEADING_LABELS).map(play_ui::hud_heading_label_key);
        for key in pitch_keys.chain(heading_keys) {
            if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &key) {
                node.set_active(false);
            }
        }
        app.ui.has_changed = true;
        if !shown {
            return;
        }

        // The side lines and the range scale - in screen pixels (the UI's,
        // scaled to the screen's real ones).
        let to_pixels = app.window_manager.pixel_size.height as f32 / screen_height.max(1.0);
        // On a pixel's middle, so the lines that meet line up exactly.
        let snap = |value: f32| (value * to_pixels).round() + 0.5;
        let mut screen_line = |start: (f32, f32), end: (f32, f32), width: f32| app.hud_screen_lines.dynamic.push(LineInstance {
            start: [snap(start.0), snap(start.1), 0.0],
            end: [snap(end.0), snap(end.1), 0.0],
            color: COLOR,
            width,
        });
        // The shader runs each line half its width past its ends, so the
        // ends meet the side line in a square corner.
        for (line, inward) in [(left_line, 1.0), (right_line, -1.0)] {
            screen_line((line, line_top), (line, line_bottom), WIDTH);
            for y in [line_top, line_bottom] {
                screen_line((line, y), (line + inward * END_IN, y), WIDTH);
            }
        }
        // The readouts' boxes. The radar altitude under the altitude number
        // has none, and neither does WARN.
        {
            let mut readout_box = |x: f32, y: f32, width: f32| {
                let (left, right) = (x - width * 0.5, x + width * 0.5);
                let (top, bottom) = (y - readout_height * 0.5, y + readout_height * 0.5);
                screen_line((left, top), (right, top), WIDTH);
                screen_line((right, top), (right, bottom), WIDTH);
                screen_line((right, bottom), (left, bottom), WIDTH);
                screen_line((left, bottom), (left, top), WIDTH);
            };
            readout_box(speed_x, middle_y, readout_width);
            readout_box(altitude_x, middle_y, readout_width);
        }
        // Three wheels, in the right-hand column, while the gear is out.
        // Each ring is wide enough, and broken into enough pieces, that it
        // reads as a circle rather than a handful of squares.
        let (gear_deploy, brake_deploy) = scene.content.nodes.get(self.flight_manager.player_id())
            .and_then(|node| node.physics_state::<AircraftState>())
            .map(|state| (state.gear_deploy, state.speed_brake))
            .unwrap_or((0.0, 0.0));
        if gear_deploy > 0.02 {
            // Big enough that each piece of the ring is a short arc, not a
            // square. A line through the middle is the axle.
            let radius = 11.0;
            let nose = (side_x, wheels_y - 14.0);
            let left_main = (side_x - 16.0, wheels_y + 8.0);
            let right_main = (side_x + 16.0, wheels_y + 8.0);
            const STEPS: usize = 24;
            for center in [nose, left_main, right_main] {
                for step in 0..STEPS {
                    let a0 = step as f32 / STEPS as f32 * std::f32::consts::TAU;
                    let a1 = (step + 1) as f32 / STEPS as f32 * std::f32::consts::TAU;
                    screen_line(
                        (center.0 + radius * a0.cos(), center.1 + radius * a0.sin()),
                        (center.0 + radius * a1.cos(), center.1 + radius * a1.sin()),
                        1.0,
                    );
                }
                screen_line((center.0 - radius * 0.55, center.1), (center.0 + radius * 0.55, center.1), 1.0);
            }
        }
        if brake_deploy > 0.02 {
            let hinge = (side_x + 32.0, wheels_y);
            let spread = 4.0 + brake_deploy * 6.0;
            screen_line(hinge, (hinge.0 + 2.0, hinge.1 - spread), WIDTH);
            screen_line(hinge, (hinge.0 + 2.0, hinge.1 + spread), WIDTH);
        }
        if let Some(cue) = &self.range_cue {
            // Bottom of the line is 0 m, the top the seeker's range.
            let height_of = |range: f32| line_bottom - (range / cue.scale_top).clamp(0.0, 1.0) * (line_bottom - line_top);
            for range in [cue.min_range, cue.max_range] {
                let y = height_of(range);
                screen_line((right_line - TICK, y), (right_line, y), WIDTH);
            }
            // The arrow, pointing at the line from inside - closed into a
            // triangle while the target's in range.
            let y = height_of(cue.range);
            let tip = (right_line - 2.0, y);
            let (upper, lower) = ((tip.0 - ARROW * 1.5, y - ARROW), (tip.0 - ARROW * 1.5, y + ARROW));
            screen_line(upper, tip, WIDTH);
            screen_line(lower, tip, WIDTH);
            if cue.in_range {
                screen_line(upper, lower, WIDTH);
            }
        }

        let active = scene.cameras.active();
        let look = active.camera.rotation_modifier * active.camera.calc_forward_direction();
        // Looking straight up or down there's no "toward" on the horizon.
        let Some(toward) = Vector3::new(look.x, 0.0, look.z).try_normalize(1e-3) else { return };
        // The space between the side lines in fifths: the horizon's two
        // halves take the 2nd and 4th, the gap between them the 3rd. The
        // other lines are half as long, around the same gap.
        let fifth = (right_line - left_line) / 5.0;
        let gap = fifth * 0.5;
        let horizon_half_length = gap + fifth;
        let ladder_half_length = gap + fifth * 0.5;

        // The heading tape - where the camera looks, north +Z and east -X
        // (as the F10 map has them): a tick every 5°, longer every 10° with
        // its number (or N/E/S/W) over it - but not under the heading itself,
        // bigger in the middle.
        let per_degree = horizon_half_length / HEADING_SPAN_DEG;
        let heading = (-toward.x).atan2(toward.z).to_degrees().rem_euclid(360.0);
        if let Some(label) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "hud_heading").and_then(|node| node.as_label_mut()) {
            label.set_text(&mut app.ui.text.font_system, &((heading.round() as i32) % 360).to_string(), true);
        }
        let mut heading_labels = Vec::new();
        let first = ((heading - HEADING_SPAN_DEG) / 5.0).ceil() as i32;
        let last = ((heading + HEADING_SPAN_DEG) / 5.0).floor() as i32;
        for mark in first..=last {
            let degrees = mark * 5;
            let x = middle_x + (degrees as f32 - heading) * per_degree;
            let major = degrees.rem_euclid(10) == 0;
            screen_line((x, ticks_bottom), (x, ticks_bottom - if major { HEADING_MAJOR_TICK } else { HEADING_MINOR_TICK }), WIDTH);
            // Its number's box clear of the heading's.
            let clear = (x - middle_x).abs() >= (HEADING_BOX.0 + play_ui::HUD_SMALL_LABEL_SIZE.0) * 0.5;
            if major && clear {
                let text = match degrees.rem_euclid(360) {
                    0 => "N".to_owned(),
                    90 => "E".to_owned(),
                    180 => "S".to_owned(),
                    270 => "W".to_owned(),
                    other => other.to_string(),
                };
                heading_labels.push((x, label_y, text));
            }
        }
        // The heading's box.
        let (left, right) = (middle_x - HEADING_BOX.0 * 0.5, middle_x + HEADING_BOX.0 * 0.5);
        let (top, bottom) = (heading_y - HEADING_BOX.1 * 0.5, heading_y + HEADING_BOX.1 * 0.5);
        screen_line((left, top), (right, top), WIDTH);
        screen_line((right, top), (right, bottom), WIDTH);
        screen_line((right, bottom), (left, bottom), WIDTH);
        screen_line((left, bottom), (left, top), WIDTH);

        // The horizon and the ladder - in the world, toward where the camera
        // looks, so they stay on the HUD's middle line as it rolls and
        // pitches; only seen between the side lines (screen pixels).
        let ladder_box = [(left_line, line_top), (right_line, line_top), (right_line, line_bottom), (left_line, line_bottom)]
            .map(|(x, y)| [x * to_pixels, y * to_pixels]);
        app.hud_horizon_lines.mask = convex_mask(&ladder_box, 2.0 * to_pixels);
        let across = Vector3::y().cross(&toward);
        let eye = active.camera.position().coords;
        let meters_per_pixel = 2.0 * DISTANCE * (active.projection.fovy.to_radians() * 0.5).tan() / screen_height.max(1.0);
        let mut labels = Vec::new();
        // None of it in the Fox-2 mode.
        let steps = if fox2 { 1..=0 } else { (-90 / LADDER_STEP_DEG)..=(90 / LADDER_STEP_DEG) };
        for step in steps {
            let degrees = step * LADDER_STEP_DEG;
            let pitch = (degrees as f32).to_radians();
            let half_length = if step == 0 { horizon_half_length } else { ladder_half_length };
            let center = eye + (toward * pitch.cos() + Vector3::y() * pitch.sin()) * DISTANCE;
            // Only the ones in front of the camera - the mask cuts off what's
            // out of the box.
            let in_view = app.camera_resources.world_to_screen(active, Point3::from(center), screen_width as u32, screen_height as u32).is_some();
            if !in_view {
                continue;
            }
            // Solid, or below the horizon in dashes (px from the middle).
            let pieces: Vec<(f32, f32)> = if step < 0 {
                std::iter::successors(Some(gap), |from| Some(from + DASH + DASH_GAP))
                    .take_while(|from| *from < half_length)
                    .map(|from| (from, (from + DASH).min(half_length)))
                    .collect()
            } else {
                vec![(gap, half_length)]
            };
            for side in [-1.0, 1.0] {
                for (from, to) in &pieces {
                    let start = center + across * (side * from * meters_per_pixel);
                    let end = center + across * (side * to * meters_per_pixel);
                    app.hud_horizon_lines.dynamic.push(LineInstance { start: start.into(), end: end.into(), color: COLOR, width: WIDTH });
                }
                // Its number past each end - not the horizon's.
                if step != 0 {
                    let at = center + across * (side * (half_length + LABEL_OFFSET) * meters_per_pixel);
                    if let Some(point) = app.camera_resources.world_to_screen(active, Point3::from(at), screen_width as u32, screen_height as u32) {
                        labels.push((point.x as f32, point.y as f32, degrees.to_string()));
                    }
                }
            }
        }

        // The ladder's numbers, only where they're wholly inside its box;
        // the heading tape's.
        let (label_width, label_height) = play_ui::HUD_SMALL_LABEL_SIZE;
        let inside = |x: f32, y: f32| (left_line + label_width * 0.5..=right_line - label_width * 0.5).contains(&x)
            && (line_top + label_height * 0.5..=line_bottom - label_height * 0.5).contains(&y);
        let pitch_placed = labels.into_iter().filter(|(x, y, _)| inside(*x, *y)).take(play_ui::HUD_PITCH_LABELS)
            .enumerate().map(|(index, label)| (play_ui::hud_pitch_label_key(index), label));
        let heading_placed = heading_labels.into_iter().take(play_ui::HUD_HEADING_LABELS)
            .enumerate().map(|(index, label)| (play_ui::hud_heading_label_key(index), label));
        for (key, (x, y, text)) in pitch_placed.chain(heading_placed) {
            let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &key) else { continue };
            node.set_active(true);
            node.transform.x = x - label_width / 2.0;
            node.transform.y = y - label_height / 2.0;
            node.transform.rect.left = node.transform.x;
            node.transform.rect.top = node.transform.y;
            node.transform.rect.right = node.transform.x + label_width;
            node.transform.rect.bottom = node.transform.y + label_height;
            if let Some(label) = node.as_label_mut() {
                label.set_text(&mut app.ui.text.font_system, &text, true);
            }
        }
    }

    /// One HUD symbol around `point` in the world - `half_size` pixels
    /// across each way, upright to `up`: a diamond (thick), or a square box.
    /// Not drawn behind the camera.
    fn hud_symbol(scene: &Scene, app: &mut App, point: Vector3<f32>, up: Vector3<f32>, half_size: f32, diamond: bool) {
        const COLOR: [f32; 4] = [0.0, 1.0, 0.3, 0.95];
        // Its lines' width (px) - the diamond's, the box's.
        const DIAMOND_WIDTH: f32 = 4.0;
        const BOX_WIDTH: f32 = 1.2;
        let line_width = if diamond { DIAMOND_WIDTH } else { BOX_WIDTH };
        let active = scene.cameras.active();
        let (width, height) = (app.window_manager.size.width, app.window_manager.size.height);
        if app.camera_resources.world_to_screen(active, Point3::from(point), width, height).is_none() {
            return;
        }
        let view = point - active.camera.position().coords;
        let distance = view.magnitude();
        let (Some(right), true) = (view.cross(&up).try_normalize(1e-6), distance > 1.0) else { return };
        let screen_up = right.cross(&(view / distance));
        let pixels = app.window_manager.pixel_size.height.max(1) as f32;
        let meters = half_size * 2.0 * distance * (active.projection.fovy.to_radians() * 0.5).tan() / pixels;
        let corners = if diamond {
            [point + screen_up * meters, point + right * meters, point - screen_up * meters, point - right * meters]
        } else {
            [point + (screen_up + right) * meters, point + (right - screen_up) * meters, point - (screen_up + right) * meters, point + (screen_up - right) * meters]
        };
        for index in 0..4 {
            let (start, end) = (corners[index], corners[(index + 1) % 4]);
            app.hud_lines.dynamic.push(LineInstance { start: start.into(), end: end.into(), color: COLOR, width: line_width });
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
        let ui_elapsed = app.throttling.last_ui_update.elapsed().min(Duration::from_millis(100));
        if ui_elapsed >= app.throttling.ui_update_interval {
            let ui_delta_time = ui_elapsed.as_secs_f32();

            // Reads the "player" node once up front (plain Copy values, so
            // nothing borrowed from `scene` needs to stay alive afterward)
            // instead of fetching it again at every label below - throttle
            // from its Plane behavior, everything else from the latest state
            // its physics half published.
            let player_id = self.flight_manager.player_id();
            let player = scene.content.nodes.get(player_id);
            let throttle = player.and_then(|node| node.get_behavior::<Plane>()).map(|plane| plane.controls.throttle).unwrap_or(0.0);
            let (g_meter, altimeter, speedometer, velocity, stall) = player
                .and_then(|node| node.physics_state::<AircraftState>())
                .map(|state| (state.flight_data.g_meter, state.flight_data.altimeter, state.flight_data.speedometer, Some(state.flight_data.velocity), state.flight_data.stall))
                .unwrap_or((0.0, 0.0, 0.0, None, false));

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
            let (fuel_kg, fuel_capacity) = player.and_then(|node| node.get_behavior::<Plane>()).map(|plane| plane.fuel()).unwrap_or((0.0, 0.0));
            if let Some(label) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "game_ui/data_box/fuel").and_then(|n| n.as_label_mut()) {
                let text = if fuel_capacity > 0.0 { format!("Fuel: {:.0} / {:.0} kg", fuel_kg, fuel_capacity) } else { "Fuel: —".to_owned() };
                label.set_text(&mut app.ui.text.font_system, &text, true);
            }

            if let Some(label) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "game_ui/data_box/weapons").and_then(|n| n.as_label_mut()) {
                label.set_text(&mut app.ui.text.font_system, &self.weapons_status, true);
            }
            if let Some(label) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "game_ui/data_box/countermeasures").and_then(|n| n.as_label_mut()) {
                label.set_text(&mut app.ui.text.font_system, &self.countermeasures_status, true);
            }
            play_ui::update_weapon_list(app, &self.weapon_list);

            if let Some(label) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "hud_altitude").and_then(|node| node.as_label_mut()) {
                label.set_text(&mut app.ui.text.font_system, &format!("{:.0}", altimeter), true);
            }

            // F3 debug view - two separate single-line labels (see
            // play::ui::build_debug_panel's own doc comment on why not one
            // multi-line label), each updated the same way as every other
            // label in this block.
            if let Some(label) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "debug_panel/stats_box/fps").and_then(|n| n.as_label_mut()) {
                label.set_text(&mut app.ui.text.font_system, &format!("{:.0} FPS", app.time.get_fps()), true);
            }
            if let Some(pos) = scene.content.renderizable_instances.get(player_id).map(|i| i.instance.transform.position) {
                if let Some(label) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "debug_panel/stats_box/position").and_then(|n| n.as_label_mut()) {
                    label.set_text(&mut app.ui.text.font_system, &format!("Player position: ({:.1}, {:.1}, {:.1})", pos.x, pos.y, pos.z), true);
                }
            }
            if let Some(rotation) = scene.content.renderizable_instances.get(player_id).map(|i| i.instance.transform.rotation) {
                let text = match bank_angle(&rotation) {
                    Some(bank) => format!("Bank: {bank:.1}° ({})", if bank.abs() < 0.5 { "level" } else if bank > 0.0 { "right wing down" } else { "left wing down" }),
                    None => "Bank: - (nose straight up/down)".to_owned(),
                };
                if let Some(label) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "debug_panel/stats_box/bank").and_then(|n| n.as_label_mut()) {
                    label.set_text(&mut app.ui.text.font_system, &text, true);
                }
            }
            // Ocean culling results (see WaterRenderData::visible_ranges) -
            // F8 freezes culling to see it in the world.
            let ocean = app.water.draw_stats();
            if let Some(label) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "debug_panel/stats_box/ocean").and_then(|n| n.as_label_mut()) {
                label.set_text(&mut app.ui.text.font_system, &format!(
                    "Ocean: {}/{} tiles, {:.1}M/{:.1}M triangles",
                    ocean.tiles_drawn, ocean.tiles_total,
                    ocean.triangles_drawn as f64 / 1e6, ocean.triangles_total as f64 / 1e6,
                ), true);
            }

            if let Some(label) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "hud_speed").and_then(|n| n.as_label_mut()) {
                label.set_text(&mut app.ui.text.font_system, &format!("{:.0}", speedometer), true);
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
            if let Some(velocity) = &velocity {
                if velocity.magnitude() > 0.1 {
                    if let Some(player) = scene.content.renderizable_instances.get(player_id) {
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

            // Every aircraft's name over it - see flight_manager::update_name_tags.
            self.flight_manager.update_name_tags(scene, app, self.hud_hidden_by_g);

            app.ui.has_changed = true; // Mark UI as changed so it gets processed
            app.throttling.last_ui_update = Instant::now();
        }
    }

    fn map_to_range(x: f64, in_min: f64, in_max: f64, out_min: f64, out_max: f64) -> f64 {
        (x - in_min) * (out_max - out_min) / (in_max - in_min) + out_min
    }
}

/// How far a jet turned `rotation` is banked against the horizon (degrees):
/// the angle its wings make with the level line square to its nose - +
/// right wing down, - left wing down, ±180 upside down. None with the nose
/// straight up or down (no horizon to measure against).
fn bank_angle(rotation: &UnitQuaternion<f32>) -> Option<f32> {
    let nose = rotation * Vector3::z();
    let up = rotation * Vector3::y();
    // Level, square to the nose: toward the left wing (+X) and up.
    let level_left = Vector3::y().cross(&nose).try_normalize(1e-3)?;
    let level_up = nose.cross(&level_left);
    // Banking right tips the jet's up toward its right (-left).
    Some((-up.dot(&level_left)).atan2(up.dot(&level_up)).to_degrees())
}

/// How long the HUD takes to fade in after a cinematic (s).
const HUD_BOOT_FADE: f32 = 0.32;

fn hud_boot_length() -> f32 {
    HUD_BOOT_FADE
}

/// 0 hidden, 1 fully on. `t` is seconds since the cockpit HUD started
/// coming on: a short fade, then steady.
fn hud_boot_alpha(t: f32) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    if t >= HUD_BOOT_FADE {
        return 1.0;
    }
    let u = t / HUD_BOOT_FADE;
    u * u * (3.0 - 2.0 * u)
}

#[cfg(test)]
mod hud_boot_tests {
    use super::*;

    #[test]
    fn hud_fades_in_then_stays_on() {
        assert_eq!(hud_boot_alpha(0.0), 0.0);
        let early = hud_boot_alpha(HUD_BOOT_FADE * 0.5);
        assert!(early > 0.0 && early < 1.0, "{early}");
        assert_eq!(hud_boot_alpha(hud_boot_length()), 1.0);
        assert_eq!(hud_boot_alpha(hud_boot_length() + 2.0), 1.0);
    }
}

#[cfg(test)]
mod bank_tests {
    use super::*;

    #[test]
    fn bank_is_measured_against_the_horizon() {
        let level = UnitQuaternion::identity();
        assert!(bank_angle(&level).unwrap().abs() < 1e-3);
        // Rolled about the nose (+Z): +30 lifts the left wing (+X)... check
        // by which way up tips: up toward -X is right wing down.
        let right_down = UnitQuaternion::from_axis_angle(&Vector3::z_axis(), 30f32.to_radians());
        let up = right_down * Vector3::y();
        let expected = if up.x < 0.0 { 30.0 } else { -30.0 };
        assert!((bank_angle(&right_down).unwrap() - expected).abs() < 1e-3);
        // Pitched up 40 degrees, then rolled: still the same bank.
        let pitched = UnitQuaternion::from_axis_angle(&Vector3::x_axis(), -40f32.to_radians()) * right_down;
        assert!((bank_angle(&pitched).unwrap() - expected).abs() < 1e-2, "{:?}", bank_angle(&pitched));
        // Upside down.
        let inverted = UnitQuaternion::from_axis_angle(&Vector3::z_axis(), 180f32.to_radians());
        assert!((bank_angle(&inverted).unwrap().abs() - 180.0).abs() < 1e-3);
        // Straight up: no bank.
        let vertical = UnitQuaternion::from_axis_angle(&Vector3::x_axis(), -90f32.to_radians());
        assert_eq!(bank_angle(&vertical), None);
    }
}

/// Where F10's map is on screen (logical px: x, y, width, height) - the
/// editor view's middle, between its panels, or the whole screen.
fn tactical_viewport(app: &App) -> [f32; 4] {
    app.tactical_viewport.unwrap_or([0.0, 0.0, app.window_manager.size.width as f32, app.window_manager.size.height as f32])
}

/// The heat view's color for a source `glow` bright (0..1) - an "ironbow"
/// thermal palette: dark purple-red, through orange, to white-hot.
fn heat_color(glow: f32) -> [f32; 3] {
    let mix = |a: [f32; 3], b: [f32; 3], t: f32| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
    const COOL: [f32; 3] = [0.45, 0.0, 0.35];
    const WARM: [f32; 3] = [1.0, 0.45, 0.0];
    const HOT: [f32; 3] = [1.0, 1.0, 0.85];
    let glow = glow.clamp(0.0, 1.0);
    if glow < 0.5 { mix(COOL, WARM, glow * 2.0) } else { mix(WARM, HOT, glow * 2.0 - 1.0) }
}

/// A filled disc `radius` UI pixels round `center` (UI pixels), in `color` -
/// as stripes of screen-space line (see ThickLines::screen_space), so it
/// rides the HUD's own lines.
fn heat_blob(lines: &mut Vec<LineInstance>, center: (f32, f32), radius: f32, color: [f32; 4], to_pixels: f32) {
    // Stripes 2 px apart, 2 px thick - solid, and the soft edges blend them.
    let (cx, cy, r) = (center.0 * to_pixels, center.1 * to_pixels, radius * to_pixels);
    let mut dy = -r;
    while dy <= r {
        let half = (r * r - dy * dy).max(0.0).sqrt();
        lines.push(LineInstance { start: [cx - half, cy + dy, 0.0], end: [cx + half, cy + dy, 0.0], color, width: 2.0 });
        dy += 2.0;
    }
}

/// How long a kill stays in the kill feed (s).
const KILL_FEED_SECONDS: f32 = 8.0;

/// One line of the kill feed - see GameLogic::update_kill_feed: its pieces
/// of text (killer, "killed", victim, "with ..."), each with its color and
/// how wide it is (px).
struct KillFeedEntry {
    pieces: [(String, UiColor, f32); play_ui::KILL_FEED_PIECES],
    seconds_left: f32,
}

/// A camera key's change of view - see GameLogic::update_camera_focus.
enum CameraSwitch {
    To(CameraState),
    Next,
}

/// The HUD's range scale with a lock (m) - see GameLogic::update_hud.
struct RangeCue {
    /// To the target.
    range: f32,
    /// The missile's shortest and longest range at it, right now.
    min_range: f32,
    max_range: f32,
    /// The scale's top - its seeker's range.
    scale_top: f32,
    in_range: bool,
}

/// How a weapon reads on the HUD - its folder name, "missiles/aim-9x" ->
/// "AIM-9X".
fn weapon_label(id: &str) -> String {
    id.rsplit('/').next().unwrap_or(id).to_uppercase()
}

impl SceneBehaviour for GameLogic {
    fn update(&mut self, scene: &mut Scene, app: &mut App, ctx: &mut FrameContext) {
        self.update(scene, app, ctx.physics_command_tx, ctx.physics_data);
    }

    fn late_update(&mut self, scene: &mut Scene, app: &mut App, ctx: &mut FrameContext) {
        self.late_update(scene, app, ctx.physics_command_tx);
    }
}

/// The rotation looking along `forward` with `up` as near up as it gets.
fn facing_with_up(forward: Vector3<f32>, up: Vector3<f32>) -> UnitQuaternion<f32> {
    match (forward.try_normalize(1e-6), up.try_normalize(1e-6)) {
        (Some(forward), Some(up)) if forward.cross(&up).norm() > 1e-4 => UnitQuaternion::face_towards(&forward, &up),
        (Some(forward), _) => crate::engine::scene_manager::scene_nodes::facing(forward).unwrap_or_else(UnitQuaternion::identity),
        _ => UnitQuaternion::identity(),
    }
}
