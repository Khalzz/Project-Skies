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
use super::plane::aircraft_spec::AircraftSpec;
use super::camera::head_motion::HeadInput;
use super::{camera::camera::Camera, event_handling::EventSystem, plane::{messages::AircraftState, plane::Plane, quick_list}};
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
        Self::spawn_world(scene, app, &mut flight_manager, prepared.ground_trimesh, prepared.runway_trimesh);

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
            flight_manager,
            flight_editor_was_open: false,
            flight_editor_map_invalid: false,
            camera_focus: String::new(),
            editor_selection: None,
            tactical_map: TacticalMap::default(),
        }
    }

    fn spawn_world(scene: &mut Scene, app: &mut App, flight_manager: &mut flight_manager, ground_trimesh: (Vec<Vector3<f32>>, Vec<[u32; 3]>), runway_trimesh: (Vec<Vector3<f32>>, Vec<[u32; 3]>)) {
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
            cameras.insert("cinematic".to_owned(), GameObjectCamera { position: Vector3::new(-10.0, 3.0, 0.0), fov: 60.0 });
            cameras.insert("frontal".to_owned(), GameObjectCamera { position: Vector3::new(0.0, 6.0, 26.0), fov: 60.0 });
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
        let Some(plane) = scene.content.nodes.get(self.flight_manager.player_id()).and_then(|node| node.get_behavior::<Plane>()) else { return };
        let effects = plane.pilot.screen_effects();
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
        self.update_tactical_map(scene, app);
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
        // Which instance the camera follows - see update_camera_focus.
        let focus = self.update_camera_focus(scene, app, player_id);
        if let Some(followed) = scene.content.renderizable_instances.get(&focus) {
            let position = followed.instance.transform.position;
            let rotation = followed.instance.transform.rotation;
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

        if let Some(event_system) = &mut self.event_system {
            event_system.handle_events(self.game_time, app, &mut self.subtitle_data);
        }
        self.subtitle_data.update(app);
        self.ui_control(scene, app);

        if let Some(event_system) = &self.event_system {
            event_system.apply_tracks(self.game_time, scene, app);
        }
        self.hide_hud(app);

    }

    /// Which instance the camera follows - F1: the player's aircraft; F2:
    /// the next of the other aircraft, round and round; F3/F4: the next ground
    /// unit / launched missile (there are none yet); or an aircraft picked in
    /// the F6 flight editor's hierarchy. Back to the player when the one it
    /// followed is gone. Returns its node id.
    fn update_camera_focus(&mut self, scene: &Scene, app: &App, player_id: &str) -> String {
        let selection = app.flight_editor.selected_aircraft().map(str::to_owned);
        if selection != self.editor_selection {
            if let Some(name) = &selection {
                self.camera_focus = name.clone();
            }
            self.editor_selection = selection;
        }

        if input::is_action_just_pressed("camera_player") {
            self.camera_focus = player_id.to_owned();
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
                Some(name) => self.camera_focus = (*name).to_owned(),
                None => println!("Camera: no other aircraft to follow"),
            }
        }
        if input::is_action_just_pressed("camera_next_ground_unit") {
            println!("Camera: no ground units to follow yet");
        }
        if input::is_action_just_pressed("camera_next_missile") {
            println!("Camera: no missiles to follow yet");
        }

        if !scene.content.renderizable_instances.contains_key(&self.camera_focus) {
            self.camera_focus = player_id.to_owned();
        }
        self.camera_focus.clone()
    }

    /// F10's tactical map (see play::tactical_map): opens/closes it - the
    /// camera into its orbit view and the renderer onto its lines - keeps
    /// every aircraft's trail, and while it's open builds this frame's
    /// lines.
    fn update_tactical_map(&mut self, scene: &mut Scene, app: &mut App) {
        if input::is_action_just_pressed("toggle_tactical_map") {
            self.tactical_map.open = !self.tactical_map.open;
            app.tactical_map_active = self.tactical_map.open;
            app.ui.has_changed = true;
            if let Some(camera) = scene.content.nodes.get_mut("camera").and_then(|node| node.get_behavior_mut::<Camera>()) {
                camera.set_tactical_view(self.tactical_map.open);
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
        app.thick_lines.dynamic = self.tactical_map.lines(self.flight_manager.map(), &live, self.flight_manager.player_id(), &view, sea_level);
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

            if let Some(label) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "game_ui/altitude").and_then(|n| n.as_label_mut()) {
                label.set_text(&mut app.ui.text.font_system, &format!("ALT: {:.0}", altimeter), true);
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

impl SceneBehaviour for GameLogic {
    fn update(&mut self, scene: &mut Scene, app: &mut App, ctx: &mut FrameContext) {
        self.update(scene, app, ctx.physics_command_tx, ctx.physics_data);
    }
}
