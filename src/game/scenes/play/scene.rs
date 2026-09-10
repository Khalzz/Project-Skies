use std::{collections::HashMap, f32::consts::PI, time::{Duration, Instant}};

use glyphon::FontSystem;
use nalgebra::{vector, Point3, UnitQuaternion, Vector3};
use crate::{app::App, engine::audio::subtitles::Subtitle, engine::input::input, engine::physics::physics_handler::{MetadataType, PhysicsCommand, PhysicsTick, RenderMessage}, engine::physics::physics_resources::PhysicsObjectDef, engine::primitive::manual_vertex::ManualVertex, engine::rendering::{camera::camera::yaw_pitch_rotation, enviroment::environment::Environment, ui::ui::Ui}, engine::scene_manager::{node::Node, properties::{Model, Transform3D}, scene::{FrameContext, Scene, SceneBehaviour}}, transform::Transform, engine::ui::{color::UiColor, ui_node::{UiNode, UiNodeContent}, ui_transform::{Anchor, Orientation, PositionValue, SizeValue}}};
use crate::engine::game_nodes::game_object::{Camera as GameObjectCamera, Cameras, ColliderType, Lighting, Physics as PhysicsProperty, RigidBodyData};
use super::{camera::camera::Camera, event_handling::EventSystem, plane::{physics_logic::PlanePhysicsLogic, plane::Plane}};
use std::sync::mpsc::Sender;
use crate::game::scenes::play::plane::plane::PlaneControls;
use crate::game::selected_level::SELECTED_LEVEL;
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
}

pub struct PreparedPlayAssets {
    environment: resources::PreparedEnvironment,
    ground_trimesh: (Vec<Vector3<f32>>, Vec<[u32; 3]>),
    runway_trimesh: (Vec<Vector3<f32>>, Vec<[u32; 3]>),
}

impl GameLogic {
    pub fn prepare(device: &wgpu::Device, queue: &wgpu::Queue, camera_bind_group_layout: &wgpu::BindGroupLayout, config: &wgpu::SurfaceConfiguration, environment: Environment) -> PreparedPlayAssets {
        let environment = resources::prepare_environment(device, queue, camera_bind_group_layout, config, environment);
        let ground_trimesh = resources::load_trimesh_geometry("ground/ground.glb").unwrap_or_else(|error| {
            eprintln!("play: ground trimesh from 'ground/ground.glb' couldn't be loaded: {error}");
            (vec![], vec![])
        });
        let runway_trimesh = resources::load_trimesh_geometry("Runway/Runway.glb").unwrap_or_else(|error| {
            eprintln!("play: runway trimesh from 'Runway/Runway.glb' couldn't be loaded: {error}");
            (vec![], vec![])
        });
        PreparedPlayAssets { environment, ground_trimesh, runway_trimesh }
    }

    pub fn finish(scene: &mut Scene, app: &mut App, prepared: PreparedPlayAssets) -> Self {
        scene.environment.skybox = prepared.environment.skybox;
        scene.environment.clear_color = prepared.environment.clear_color;
        app.scene_openned = Some("./assets/scenes/test_chamber".to_owned());

        Self::spawn_world(scene, app, prepared.ground_trimesh, prepared.runway_trimesh);

        app.window_manager.context.mouse().set_relative_mouse_mode(true);

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
        }
    }

    fn spawn_world(scene: &mut Scene, app: &mut App, ground_trimesh: (Vec<Vector3<f32>>, Vec<[u32; 3]>), runway_trimesh: (Vec<Vector3<f32>>, Vec<[u32; 3]>)) {
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

        // Pushed off the origin (was 0,0,0) to make room for the runway
        // island, which now sits at 0,0,0 - see the "runway" node below. Both
        // the mesh and its trimesh collider move with this Transform3D's
        // position, so nothing else needs adjusting.
        let ground_position = Vector3::new(0.0, 0.0, 50_000.0);
        let ground_scale = Vector3::new(30_000.0, 30_000.0, 30_000.0);
        scene.spawn_node(app,
            Node::new("ground")
                .add_property(Transform3D {
                    position: ground_position,
                    rotation: UnitQuaternion::identity(),
                    scale: ground_scale,
                })
                .add_property(Model { model_ref: "Ground".to_owned() })
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
        let runway_scale = Vector3::new(60.0, 60.0, 60.0);
        scene.spawn_node(app,
            Node::new("runway")
                .add_property(Transform3D {
                    position: Vector3::new(0.0, 100.0, 0.0),
                    rotation: UnitQuaternion::identity(),
                    scale: runway_scale,
                })
                .add_property(Model { model_ref: "Runway".to_owned() })
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

        scene.spawn_node(app,
            Node::new("player")
                .add_behavior(Plane::new())
                .add_property(Transform3D {
                    position: Vector3::new(0.0, 100.0, -3400.0),
                    rotation: UnitQuaternion::identity(),
                    scale: Vector3::new(14.0, 14.0, 14.0),
                })
                .add_property(Model { model_ref: "F16".to_owned() })
                .add_property(PhysicsProperty {
                    rigidbody: RigidBodyData {
                        is_static: false,
                        mass: 8900.0,
                        center_of_mass: Vector3::new(0.0, 0.0, 0.5),
                        initial_velocity: Vector3::new(0.0, 0.0, 0.0),
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
            cameras.insert("cockpit".to_owned(), GameObjectCamera { position: Vector3::new(0.0, 2.158, 13.324), fov: 70.0 });
            cameras.insert("cinematic".to_owned(), GameObjectCamera { position: Vector3::new(0.0, 1000.0, 900.0), fov: 60.0 });
            cameras.insert("frontal".to_owned(), GameObjectCamera { position: Vector3::new(0.0, 6.0, 35.0), fov: 40.0 });
            player.instance.metadata.cameras = Some(cameras);
        }

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
                    scale: Vector3::new(2_000.0, 1.0, 2_000.0),
                })
                .add_property(Model { model_ref: "WaterPlane".to_owned() })
        ).expect("play should only spawn 'world' once");

        // Separate "WaterPlaneFar" model (see main.rs's own registration) -
        // same huge footprint the old single "WaterPlane" had, covers what
        // "world" above can't (foam/fog/horizon still need a water surface
        // out that far). Its Y-scale (0.03) isn't a visual size - water.wgsl's
        // vs_main reads it back as y_scale to force this instance's own waves
        // down to near-zero amplitude everywhere, i.e. calm by construction
        // rather than by distance falloff alone. The -4.0 Y offset keeps it
        // reliably under "world"'s fuller-amplitude surface wherever the two
        // overlap. Deliberately a distinct model_ref rather than another
        // "WaterPlane" instance - see App::water_debug_hidden_models for why.
        scene.spawn_node(app,
            Node::new("world_far")
                .add_property(Transform3D {
                    position: Vector3::new(0.0, -4.0, 0.0),
                    rotation: UnitQuaternion::identity(),
                    scale: Vector3::new(3_000_000.0, 0.03, 3_000_000.0),
                })
                .add_property(Model { model_ref: "WaterPlaneFar".to_owned() })
        ).expect("play should only spawn 'world_far' once");

        // Stand-in for a real splash/spray model - see
        // GameLogic::update_water_splash's own doc comment for why this
        // exists as a separate node instead of a water.wgsl effect. Starts
        // parked far below the world (WATER_SPLASH_HIDDEN_Y) - update_water_splash
        // repositions/reveals it once the player's actually low over water.
        scene.spawn_node(app,
            Node::new("water_splash")
                .add_property(Transform3D {
                    position: Vector3::new(0.0, Self::WATER_SPLASH_HIDDEN_Y, 0.0),
                    rotation: UnitQuaternion::identity(),
                    scale: Vector3::new(15.0, 15.0, 15.0),
                })
                .add_property(Model { model_ref: "WaterSplash".to_owned() })
        ).expect("play should only spawn 'water_splash' once");
    }

    // Hand-duplicated copy of water.wgsl's wave_height - must stay in sync
    // with that shader's DIR_*/AMPLITUDE_*/FREQUENCY_*/SPEED_*/PHASE_*
    // constants whenever they change (this file has no way to share code
    // with a WGSL shader).
    fn water_wave_height(x: f32, z: f32, t: f32) -> f32 {
        let mut h = 0.0;
        h += 0.242 * ((x * 0.985 + z * 0.174) * 0.3855 + t * 0.11 + 0.0).sin();
        h += 0.166 * ((x * 0.259 + z * 0.966) * 0.3190 + t * -0.08 + 2.1).sin();
        h += 0.119 * ((x * -0.643 + z * 0.766) * 0.2663 + t * 0.14 + 4.4).sin();
        h += 0.076 * ((x * -0.966 + z * -0.259) * 0.2252 + t * -0.10 + 1.3).sin();
        h += 0.048 * ((x * -0.342 + z * -0.940) * 0.1939 + t * 0.17 + 5.2).sin();
        h += 0.062 * ((x * 0.766 + z * -0.643) * 0.1624 + t * -0.06 + 3.6).sin();
        h + 0.713
    }

    // Recenters "world"/"world_far" (see spawn_world) under the camera's XZ
    // every frame - continuous, not snapped to a grid: the wave field is an
    // analytic function of position, not a sampled heightmap/texture, and
    // "world"'s ~2-unit cells still comfortably oversample even the shortest
    // wave term (~16-unit wavelength, see water.wgsl's own FREQUENCY_*
    // comment), so there's no texel-shimmer/vertex-swimming risk to guard
    // against at today's constants. Y is left untouched on both - only XZ
    // needs to track the camera.
    fn update_water_plane(scene: &mut Scene) {
        let camera_position = scene.cameras.active().camera.position().coords;
        if let Some(world) = scene.content.renderizable_instances.get_mut("world") {
            world.instance.transform.position.x = camera_position.x;
            world.instance.transform.position.z = camera_position.z;
        }
        if let Some(world_far) = scene.content.renderizable_instances.get_mut("world_far") {
            world_far.instance.transform.position.x = camera_position.x;
            world_far.instance.transform.position.z = camera_position.z;
        }
    }

    fn update_water_splash(scene: &mut Scene, app: &App) {
        let Some((player_position, player_rotation)) = scene.content.renderizable_instances.get("player").map(|player| (player.instance.transform.position, player.instance.transform.rotation)) else { return };
        let Some(splash) = scene.content.renderizable_instances.get_mut("water_splash") else { return };

        if player_position.y < Self::WATER_SPLASH_ALTITUDE {
            // True world position, matching water.wgsl's own true_xz fix -
            // wave_height no longer expects camera-relative input (see
            // check_water_death's own comment for the amplitude-falloff
            // simplification this makes too).
            let water_height = Self::water_wave_height(player_position.x, player_position.z, app.water_time());
            splash.instance.transform.position = Vector3::new(player_position.x, water_height, player_position.z);

            let forward = player_rotation * Vector3::new(0.0, 0.0, 1.0);
            let horizontal_forward = Vector3::new(forward.x, 0.0, forward.z);
            if horizontal_forward.norm() > 0.001 {
                if let Some(heading) = UnitQuaternion::rotation_between(&Vector3::new(0.0, 0.0, 1.0), &horizontal_forward) {
                    splash.instance.transform.rotation = heading;
                }
            }
        } else {
            splash.instance.transform.position.y = Self::WATER_SPLASH_HIDDEN_Y;
        }
    }

    fn check_water_death(&mut self, scene: &Scene, app: &mut App, physics_command_tx: Option<&Sender<PhysicsCommand>>) {
        if self.is_dead {
            return;
        }
        let Some(player) = scene.content.renderizable_instances.get("player") else { return };
        let player_position = player.instance.transform.position;

        // True world position (see water.wgsl's true_xz). Deliberately not
        // applying the shader's own FALLOFF_START/END amplitude falloff here
        // - the chase cam keeps the player within FALLOFF_START in normal
        // gameplay (falloff ~= 1.0 whenever this check actually matters), so
        // it's not worth a second hand-tuned constant to keep in sync with
        // water.wgsl's.
        let water_height = Self::water_wave_height(player_position.x, player_position.z, app.water_time());

        if player_position.y > water_height + Self::WATER_DEATH_MARGIN {
            return;
        }

        self.is_dead = true;
        if let Some(physics_command_tx) = physics_command_tx {
            let _ = physics_command_tx.send(PhysicsCommand::TogglePause);
        }
        play_ui::open_death_screen(app);
    }

    // How long ESC has to be held during an EventSystem::input_lock_end
    // window before it counts as a skip - see `update`'s own handling.
    const SKIP_HOLD_SECONDS: f32 = 1.2;

    // Where "water_splash" parks when not in use - far enough below the
    // world that it's never visible regardless of camera position/fog, see
    // spawn_world's own comment on it. Not literally required to be exactly
    // this value, just needs to be well outside anything the camera could
    // ever see.
    const WATER_SPLASH_HIDDEN_Y: f32 = -1_000_000.0;
    // Player altitude (world units above the water's own rest Y=0) below
    // which "water_splash" shows/follows - see update_water_splash's own
    // doc comment.
    const WATER_SPLASH_ALTITUDE: f32 = 60.0;
    // How far above the water's own real surface height still counts as
    // "touching" it, for check_water_death - a small allowance for the
    // plane's own size (its transform origin sits roughly at its center,
    // not its lowest point), not a difficulty/leniency knob.
    const WATER_DEATH_MARGIN: f32 = 5.0;

    // Cap on App::aero_debug_trail (see that field's own doc comment) -
    // oldest sample drops once this is exceeded, so it reads as a "recent
    // history" trail rather than growing forever while the overlay is on.
    const AERO_DEBUG_TRAIL_CAPACITY: usize = 500;

    // this is called every frame
    pub fn update(&mut self, scene: &mut Scene, app: &mut App, plane_control_tx: Option<&Sender<PlaneControls>>, physics_command_tx: Option<&Sender<PhysicsCommand>>, physics_data: &HashMap<String, RenderMessage>) {
        // See play::camera::camera::Camera::set_target's own doc comment -
        // has to run unconditionally, before ANY early return below
        // (app.is_paused, self.is_dead), so the "camera" node's own generic
        // Behavior tick (which always runs every frame regardless of what
        // this fn does - see App::run's own unconditional node-tick call)
        // never reads a target one frame stale relative to whatever physics
        // just wrote into "player"'s own renderable transform this frame.
        if let Some(player) = scene.content.renderizable_instances.get("player") {
            let position = player.instance.transform.position;
            let rotation = player.instance.transform.rotation;
            let named_cameras = player.instance.metadata.cameras.clone();
            if let Some(node) = scene.content.nodes.get_mut("camera") {
                if let Some(camera) = node.get_behavior_mut::<Camera>() {
                    camera.set_target(position, rotation, named_cameras);
                }
            }
        }

        // F3 debug view - shows/hides in lockstep with the console toggle,
        // same "set_active every frame, no separate dirty-tracking" idiom
        // skip_prompt below already uses (its own text is updated inside
        // ui_control, right alongside compass/speed/etc. - not from a
        // function of its own).
        if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "debug_panel") {
            node.set_active(debug_console::is_console_visible());
        }

        // F6 - hides "world_far" (see App::water_debug_hidden_models) so
        // "world"'s own near/detailed water plane can be checked against the
        // rest of the scene without the much-larger, forced-calm far plane
        // in the way.
        if input::is_action_just_pressed("toggle_water_debug_view") {
            app.water_debug_view = !app.water_debug_view;
        }

        // F7 - toggles the live in-window aero debug overlay (see
        // App::show_aero_debug_overlay's own doc comment).
        if input::is_action_just_pressed("toggle_aero_debug_recording") {
            app.show_aero_debug_overlay = !app.show_aero_debug_overlay;
            println!("Aero debug overlay: {}", if app.show_aero_debug_overlay { "ON" } else { "OFF" });
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
            return;
        }

        if self.is_dead {
            // Same idiom as the app.is_paused block above - freezes
            // everything else in this fn (game_time, camera follow, flight/
            // afterburner animation, ...) while still letting the death
            // screen's own Restart/Back to Main Menu buttons register
            // clicks (handled independently, see App::fire_ui_click_handlers).
            app.ui.has_changed = true;
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
                if let Some(player) = scene.content.renderizable_instances.get("player") {
                    let transform = player.instance.transform;
                    let _ = physics_command_tx.send(PhysicsCommand::SetTransform {
                        name: "player".to_owned(),
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

                    // See App::aero_debug_trail's own doc comment - the
                    // ACTUAL measured roll rate (Plane::flight_data's own,
                    // already computed by apply_physics_feedback just above)
                    // rather than the theoretical roll_authority_gain
                    // fraction - that's what render_pass.rs's own chart
                    // plots the curve in now too, both in deg/s, so trail
                    // and curve are directly comparable.
                    if app.show_aero_debug_overlay {
                        let speed_ms = physics_message.linvel.magnitude();
                        let aoa_y = plane.flight_data.aoa_y;
                        app.aero_debug_trail.push_back((speed_ms * 1.94384, aoa_y, plane.flight_data.roll_rate));
                        if app.aero_debug_trail.len() > Self::AERO_DEBUG_TRAIL_CAPACITY {
                            app.aero_debug_trail.pop_front();
                        }

                        // See App::wing_lift_trail's own doc comment.
                        let main_wing_lift_y = plane.wing_lift_forces.get("Left wing").map(|f| f.y).unwrap_or(0.0);
                        let elevator_wing_lift_y = plane.wing_lift_forces.get("Right elevator wing").map(|f| f.y).unwrap_or(0.0);
                        let next_index = app.wing_lift_trail.back().map(|(i, _, _)| i + 1.0).unwrap_or(0.0);
                        app.wing_lift_trail.push_back((next_index, main_wing_lift_y, elevator_wing_lift_y));
                        if app.wing_lift_trail.len() > Self::AERO_DEBUG_TRAIL_CAPACITY {
                            app.wing_lift_trail.pop_front();
                        }
                    }
                }
            }
        }

        Self::update_water_plane(scene);
        Self::update_water_splash(scene, app);
        self.check_water_death(scene, app, physics_command_tx);

        if let Some(event_system) = &mut self.event_system {
            event_system.handle_events(self.game_time, app, &mut self.subtitle_data);
        }
        self.subtitle_data.update(app);
        self.ui_control(scene, app);

        if let Some(event_system) = &self.event_system {
            event_system.apply_tracks(self.game_time, scene, app);
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
        self.update(scene, app, ctx.plane_control_tx, ctx.physics_command_tx, ctx.physics_data);
    }

    fn fixed_update(&self, scene: &Scene, app: &App) -> Option<(Vec<PhysicsObjectDef>, Box<dyn PhysicsTick + Send>)> {
        let _ = app;
        Some((scene.content.physics_bodies.clone(), Box::new(PlanePhysicsLogic::new())))
    }
}
