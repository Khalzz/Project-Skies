use std::{collections::HashMap, f32::consts::PI, time::{Duration, Instant}};

use glyphon::FontSystem;
use nalgebra::{vector, Point3, UnitQuaternion, Vector3};
use crate::{app::App, engine::{audio::subtitles::Subtitle, input::input, physics::physics_handler::{PhysicsCommand, RenderMessage}, primitive::manual_vertex::ManualVertex, rendering::{camera::camera::yaw_pitch_rotation, enviroment::environment::Environment, ui::ui::Ui}, scene_manager::{node::Node, properties::{Model, Transform3D}, scene::{FrameContext, LoadReporter, Scene, SceneBehaviour}}, ui::{color::UiColor, ui_node::{UiNode, UiNodeContent}, ui_transform::{Anchor, Orientation, PositionValue, SizeValue}}}, transform::Transform};
use crate::engine::game_nodes::game_object::{Camera as GameObjectCamera, Cameras, ColliderType, Lighting, Physics as PhysicsProperty, RigidBodyData};
use crate::game::scenes::main_menu::ui as main_menu_ui;
use super::flight_editor::EditorRequest;
use super::flight_manager::{flight_manager, LiveAircraft};
use super::map::MapSpec;
use super::map_view::{terrain_triangles, TerrainKind};
use super::tactical_map::{contour_lines, TacticalMap, View as TacticalView};
use super::flight_editor::next_path_point;
use super::tactical_ui::{AircraftStatus, FlightPlan, RosterCoalition, RosterEntry};
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
    was_cinematic_active: bool,
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
    // Every plane in the scene - see flight_manager.
    flight_manager: flight_manager,
    // Whether the F6 flight editor was open last frame - see
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
    // The countermeasures line on the HUD: flares and chaff left, what's
    // released - see update_weapons.
    countermeasures_status: String,
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
}

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

        app.window_manager.context.mouse().set_relative_mouse_mode(true);
        app.debug_mouse_free = false;

        scene.create_camera("main", Transform::new(Vector3::new(0.0, 0.0, 0.0), yaw_pitch_rotation(-90.0, -20.0), Vector3::new(1.0, 1.0, 1.0)), 45.0);
        scene.select_camera("main");

        app.is_paused = false;

        let screen_width = app.window_manager.size.width as f32;
        let screen_height = app.window_manager.size.height as f32;

        let mut game_ui = play_ui::build_game_ui(app);
        game_ui.resolve(screen_width, screen_height);
        app.ui.add_to_ui("game_ui".to_owned(), game_ui);
        for (key, text, font_size) in play_ui::HUD_READOUTS {
            let mut readout = play_ui::build_hud_readout(app, text, font_size);
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
            app.ui.add_to_ui(play_ui::heat_label_key(index), heat_label);
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
            was_cinematic_active: false,
            was_paused: false,
            esc_hold_time: 0.0,
            is_dead: false,
            hud_hidden_by_g: false,
            crash_time: 0.0,
            crash_seen_from_cockpit: false,
            went_in_at: HashMap::new(),
            heat_view: false,
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
            countermeasures: Countermeasures::default(),
            ai_defense: AiDefense::default(),
            range_cue: None,
            selected_weapon: None,
            weapon_list: Vec::new(),
            world_collision,
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
        // The F2 editor's map: the land and the runway from above, from the
        // same meshes their colliders are built from below.
        let sea_level = app.water.ocean.sea_level();
        app.flight_editor.terrain = [
            terrain_triangles(&ground_trimesh.0, &ground_trimesh.1, ground_position, ground_scale, sea_level, TerrainKind::Land),
            terrain_triangles(&runway_trimesh.0, &runway_trimesh.1, runway_position, runway_scale, sea_level, TerrainKind::Runway),
        ].concat();
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

        // No Transform3D/Model - a pure logic node, same idea as "player"'s
        // own Plane behavior but with no mesh/physics of its own to carry.
        // See play::camera::camera::Camera's own doc comment for how it gets
        // fed "player"'s position each frame despite a Behavior having no
        // way to reach another node's data on its own.
        scene.spawn_node(app,
            Node::new("camera").add_behavior(Camera::new())
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
        if let Some(world) = scene.content.renderizable_instances.get_mut("world") {
            world.instance.transform.position.x = camera_position.x;
            world.instance.transform.position.z = camera_position.z;
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
    /// Also decides whether the HUD should be hidden (see hide_hud) - while
    /// the view is completely gone, and for good once crashed.
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
        if self.heat_view && !app.tactical_map_active {
            effects = ScreenEffects::default();
            effects.desaturation = 1.0;
            effects.darkening = 0.75;
        }
        app.renderer.blur.effects = effects;

        let hide = effects.tunnel >= 0.97 || plane.wrecked || plane.pilot.is_dead() || app.tactical_map_active;
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
        // F6 - the live map editor; paused or not.
        self.update_flight_editor(scene, app, physics_command_tx);
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
            return;
        }

        self.game_time += app.time.delta_time as f64;

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
                if let Some(player) = scene.content.renderizable_instances.get(player_id) {
                    let transform = player.instance.transform;
                    let _ = physics_command_tx.send(PhysicsCommand::SetTransform {
                        name: player_id.to_owned(),
                        translation: transform.position,
                        rotation: transform.rotation.into_inner(),
                        // Matches data.ron's own player initial_velocity - a
                        // reasonable cruise speed to resume normal flight at
                        // regardless of exactly what the cinematic's own path was.
                        linvel: Vector3::new(0.0, 0.0, 0.0),
                    });
                }
                let _ = physics_command_tx.send(PhysicsCommand::TogglePause);
            }
        }

        if cinematic_just_ended {
            if let Some(camera) = scene.content.nodes.get_mut("camera").and_then(|node| node.get_behavior_mut::<Camera>()) {
                camera.snapshot_cinematic_return(&scene.cameras);
            }
        }

        self.was_cinematic_active = cinematic_active;

        // Everything else about the plane (input, physics feedback, model
        // animation) runs in its own Plane behavior - input_locked is the one
        // thing that depends on cinematic/event-system state only this
        // SceneBehaviour sees.
        if let Some(plane) = scene.content.nodes.get_mut(player_id).and_then(|node| node.get_behavior_mut::<Plane>()) {
            // Typing in the flight editor isn't flying.
            plane.input_locked = cinematic_active || input_lock_end.is_some() || app.flight_editor.typing;
        }

        Self::update_water_plane(scene);
        Self::update_water_wake(scene, app, player_id);
        self.check_crash(scene, app);
        let has_control = !cinematic_active && input_lock_end.is_none() && !app.flight_editor.typing;
        self.update_weapons(scene, app, has_control);

        if let Some(event_system) = &mut self.event_system {
            event_system.handle_events(self.game_time, app, &mut self.subtitle_data);
        }
        self.subtitle_data.update(app);
        self.ui_control(scene, app);

        if let Some(event_system) = &self.event_system {
            event_system.apply_tracks(self.game_time, scene, app);
        }
        // Hung weapons onto wherever the tracks just put their aircraft.
        self.flight_manager.update_stores(scene);
        self.update_hud(scene, app);
        self.update_heat_view(scene, app);
        self.hide_hud(app);

    }

    /// Which instance the camera follows, and the view to switch it to -
    /// F1: the player's aircraft, in the cockpit - pressed again there, its
    /// next view (see play::camera::camera::CameraState); F2: the next of
    /// the other aircraft, round and round, in the free view; F3/F4: the
    /// next ground unit / launched missile (there are no ground units yet);
    /// or an aircraft picked in the F6 flight editor's hierarchy. Back to
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

    /// F10's tactical map (see play::tactical_map): opens/closes it - the
    /// camera into its orbit view and the renderer onto its lines - keeps
    /// every aircraft's trail, and while it's open builds this frame's
    /// lines.
    fn update_tactical_map(&mut self, scene: &mut Scene, app: &mut App, physics_command_tx: Option<&Sender<PhysicsCommand>>) {
        if input::is_action_just_pressed("toggle_tactical_map") {
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
        // bot flies the new path) - and into the F6 editor's copy too, so it
        // doesn't put the old one back.
        // A flight plan point dragged on the map: across under the pointer,
        // at its own height - or, with Shift, up and down. Into the path like
        // an edit in its window (below).
        if let Some(drag) = app.tactical_ui.point_drag.take() {
            let selected = app.tactical_ui.selected.clone();
            let path = selected.as_deref()
                .and_then(|name| self.flight_manager.map().find(name))
                .map(|(_, placement)| placement.path.clone());
            if let (Some(name), Some(mut path)) = (selected, path) {
                if let Some(point) = path.get_mut(drag.index) {
                    let active = scene.cameras.active();
                    let (width, height) = (app.window_manager.size.width, app.window_manager.size.height);
                    if drag.vertical {
                        // A pixel's worth of height at the point's distance.
                        let distance = (Vector3::new(point.0, point.1, point.2) - active.camera.position().coords).magnitude();
                        let meters_per_pixel = 2.0 * distance * (active.projection.fovy.to_radians() * 0.5).tan() / height.max(1) as f32;
                        point.1 = (point.1 - drag.delta.y * meters_per_pixel).clamp(0.0, 20_000.0);
                    } else if let Some((origin, direction)) = app.camera_resources.screen_ray(active, drag.pointer.x, drag.pointer.y, width, height) {
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
                    app.tactical_ui.path_edit = Some((name, path));
                }
            }
        }

        if let Some((name, path)) = app.tactical_ui.path_edit.take() {
            let mut map = self.flight_manager.map().clone();
            if let Some(placement) = map.find_mut(&name) {
                placement.path = path.clone();
                if let Err(error) = self.flight_manager.sync(scene, app, physics_command_tx, &map) {
                    eprintln!("F10 flight plan: {error}");
                }
                if let Some(placement) = app.flight_editor.map.find_mut(&name) {
                    placement.path = path;
                }
            }
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
        let screen_height = app.window_manager.size.height as f32;
        if let Some(camera) = scene.content.nodes.get_mut("camera").and_then(|node| node.get_behavior_mut::<Camera>()) {
            camera.set_tactical_follow(following);
            if pan != (0.0, 0.0) {
                camera.pan_tactical(pan, screen_height);
            }
        }

        let active = scene.cameras.active();
        let focus = scene.content.renderizable_instances.get(&self.camera_focus)
            .map(|instance| instance.instance.transform.position)
            .unwrap_or_else(|| active.camera.position().coords);
        let view = TacticalView {
            camera_position: active.camera.position().coords,
            fovy_deg: active.projection.fovy,
            screen_height: app.window_manager.pixel_size.height as f32,
            focus,
        };
        let sea_level = app.water.ocean.sea_level();
        let selected = app.tactical_ui.selected.clone();
        let hovered = app.tactical_ui.hovered.clone();
        app.thick_lines.dynamic = self.tactical_map.lines(self.flight_manager.map(), &live, self.flight_manager.player_id(), selected.as_deref(), hovered.as_deref(), &view, sea_level);
        self.fill_tactical_ui(scene, app, &live, selected.as_deref());
    }

    /// What the tactical map's egui half needs this frame (see play::
    /// tactical_ui): where every aircraft is on screen (to click), and the
    /// selected one's flight plan and its points' numbers.
    fn fill_tactical_ui(&self, scene: &Scene, app: &mut App, live: &HashMap<String, LiveAircraft>, selected: Option<&str>) {
        let (width, height) = (app.window_manager.size.width, app.window_manager.size.height);
        let active = scene.cameras.active();
        let on_screen = |position: Vector3<f32>| {
            app.camera_resources.world_to_screen(active, Point3::from(position), width, height)
                .map(|point| egui::pos2(point.x as f32, point.y as f32))
        };

        let pickables = live.iter()
            .filter(|(_, aircraft)| !aircraft.wrecked)
            .filter_map(|(name, aircraft)| Some((name.clone(), on_screen(aircraft.position)?)))
            .collect();

        let mut labels = Vec::new();
        let mut point_pickables = Vec::new();
        let plan = selected.and_then(|name| self.flight_manager.map().find(name)).map(|(coalition, placement)| {
            let now = live.get(&placement.name).copied();
            let next = now.and_then(|now| now.next_path_point);
            for (index, &(x, altitude, z)) in placement.path.iter().enumerate() {
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
                name: placement.name.clone(),
                coalition: coalition.name.clone(),
                color,
                plane: placement.plane.clone(),
                is_player,
                live: now,
                points: placement.path.clone(),
                next,
                next_suggestion: next_path_point(placement, now),
            }
        });

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

    /// The F6 flight editor (see play::flight_editor): opens/closes it on
    /// F6 (freeing the mouse while it's open), carries out what it was asked
    /// to (save, reload, respawn...), and applies its map to the world -
    /// every frame it's open (see flight_manager::sync).
    fn update_flight_editor(&mut self, scene: &mut Scene, app: &mut App, physics_command_tx: Option<&Sender<PhysicsCommand>>) {
        if input::is_action_just_pressed("toggle_flight_editor") {
            app.flight_editor.open = !app.flight_editor.open;
        }
        // Refilled below while the editor is open (the aircraft's paths).
        app.world_lines.clear();
        let open = app.flight_editor.open;
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
        // Every aircraft's path, drawn in the world while the editor is open.
        app.world_lines = editor.path_lines();
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

    /// F8's heat view - what an IR seeker sees: the world dimmed to grey
    /// (see update_g_vision) and every heat source on it - each aircraft
    /// (its engine, afterburner or fire) and the sun - as a glowing blob,
    /// bigger and whiter the brighter it looks from the camera, labelled
    /// with its heat and how bright it looks (the number a seeker compares,
    /// see play::weapon::heat). Drawn into the HUD's screen lines, after
    /// update_hud has filled them.
    fn update_heat_view(&mut self, scene: &Scene, app: &mut App) {
        // Past this many meters from the camera - not the jet it's sitting in.
        const MIN_DISTANCE: f32 = 30.0;
        if input::is_action_just_pressed("toggle_heat_view") {
            self.heat_view = !self.heat_view;
        }
        for index in 0..play_ui::HEAT_LABELS {
            if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &play_ui::heat_label_key(index)) {
                node.set_active(false);
            }
        }
        if !self.heat_view || app.tactical_map_active {
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
            heat_blob(&mut app.hud_screen_lines.dynamic, (x, y), radius * 2.0, [color[0], color[1], color[2], 0.25 * glow + 0.05], to_pixels);
            heat_blob(&mut app.hud_screen_lines.dynamic, (x, y), radius, [color[0], color[1], color[2], 0.95], to_pixels);

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
        for name in events.destroyed {
            if let Some(plane) = scene.content.nodes.get_mut(&name).and_then(|node| node.get_behavior_mut::<Plane>()) {
                if !plane.wrecked {
                    println!("Splash - {name} destroyed");
                    plane.shot_down = true;
                }
            }
        }
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
        self.weapon_list = std::iter::once(("OFF".to_owned(), self.selected_weapon.is_none()))
            .chain(kinds.iter().map(|(kind, count)| (format!("{} x{count}", weapon_label(kind)), self.selected_weapon.as_ref() == Some(kind))))
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
            app.hud_lines.dynamic.clear();
            return;
        };

        // Countermeasures: the key choosing flares, chaff or both, one
        // release, the burst program - and the program's releases due now.
        let mut released = Vec::new();
        self.countermeasures_status = match self.flight_manager.countermeasures_mut(&player_id) {
            Some(set) => {
                if has_control && input::is_action_just_pressed("countermeasure_mode") {
                    set.release = set.release.next();
                }
                if has_control && input::is_action_just_pressed("countermeasure_single") {
                    released.extend(set.release_one());
                }
                if has_control && input::is_action_just_pressed("countermeasure_program") {
                    released.extend(set.start_program());
                }
                released.extend(set.update(delta_time));
                format!("FLR {}  CHF {}  {}", set.flares, set.chaff, set.release.label())
            }
            None => String::new(),
        };
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
                        self.missiles.launch(scene, app, spec.clone(), position, rotation, own.velocity, target);
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
        let shown = game_ui_shown && !self.hud_hidden_by_g;
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
        // Speed against the left edge, altitude against the right, each with
        // its caption over it.
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
        let placed = [
            ("hud_speed", speed_x, middle_y, shown),
            ("hud_altitude", altitude_x, middle_y, shown),
            ("hud_speed_caption", speed_x, middle_y - readout_height, shown),
            ("hud_altitude_caption", altitude_x, middle_y - readout_height, shown),
            ("hud_heading", middle_x, heading_y, shown),
            ("hud_g", speed_x, g_y, shown && fox2),
        ];
        for (key, x, y, visible) in placed {
            let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, key) else { continue };
            node.set_active(visible);
            node.transform.x = x - readout_width / 2.0;
            node.transform.y = y - readout_height / 2.0;
            node.transform.rect.left = node.transform.x;
            node.transform.rect.top = node.transform.y;
            node.transform.rect.right = node.transform.x + readout_width;
            node.transform.rect.bottom = node.transform.y + readout_height;
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
        // The readouts' boxes.
        for x in [speed_x, altitude_x] {
            let (left, right) = (x - readout_width * 0.5, x + readout_width * 0.5);
            let (top, bottom) = (middle_y - readout_height * 0.5, middle_y + readout_height * 0.5);
            screen_line((left, top), (right, top), WIDTH);
            screen_line((right, top), (right, bottom), WIDTH);
            screen_line((right, bottom), (left, bottom), WIDTH);
            screen_line((left, bottom), (left, top), WIDTH);
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

            if let Some(label) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "game_ui/data_box/weapons").and_then(|n| n.as_label_mut()) {
                label.set_text(&mut app.ui.text.font_system, &self.weapons_status, true);
            }
            if let Some(label) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "game_ui/data_box/countermeasures").and_then(|n| n.as_label_mut()) {
                label.set_text(&mut app.ui.text.font_system, &self.countermeasures_status, true);
            }
            play_ui::update_weapon_list(app, &self.weapon_list);

            if let Some(label) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "hud_altitude").and_then(|n| n.as_label_mut()) {
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
}
