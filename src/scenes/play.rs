//! The play scene - a level being flown: the world, the map's aircraft,
//! the player's jet and its HUD, weapons, the editor and the tactical map.
//! `Play` is the scene's behaviour (SceneBehaviour); what has a life of its
//! own lives in its own module below - the cameras in prefabs::camera, the
//! aircraft in flight_manager, the missiles in prefabs::missile...

pub mod animation_tracks;
pub mod animations;
mod armament;
pub mod crash_effects;
pub mod debug_tools;
mod editor;
pub mod event_handling;
pub mod flight_manager;
mod hud;
pub mod map;
pub mod map_nodes;
pub mod path_follow;
mod pilot_view;
mod tactical;
pub mod tactical_map;
pub mod tactical_ui;
pub mod ui;
pub mod weapon;
pub mod world_collision;

// The scene's parts, each with its own state (see each module).
use self::armament::*;
use self::editor::*;
use self::hud::*;
pub use self::hud::{draw_hud_layout, HudLayout};
use self::pilot_view::*;
use self::tactical::*;
use crate::ext::ui_layers::{self, GameUi};
use std::{
    collections::HashMap,
    f32::consts::PI,
    time::{Duration, Instant},
};

use self::animations::Animations;
use self::crash_effects::{Crash, CrashEffects};
use self::debug_tools::DebugTools;
use self::editor::flight_editor::next_path_point;
use self::editor::flight_editor::FlightEditor;
use self::editor::flight_editor::{
    EditorRequest, NodeKind, Playback, SceneNodeInfo, Telemetry, TimelineRequest,
};
use self::flight_manager::{flight_manager as FlightManager, LiveAircraft};
use self::map::AnimAction;
use self::map::Difficulty;
use self::map::MapSpec;
use self::map_nodes::MapNodes;
use self::tactical_map::{contour_lines, TacticalMap, View as TacticalView};
use self::tactical_ui::TacticalUi;
use self::tactical_ui::{AircraftStatus, FlightPlan, PlanTarget, RosterCoalition, RosterEntry};
use self::weapon::countermeasures::{Countermeasures, Release};
use self::weapon::fire_control::{self, Contact, FireControl, OwnShip};
use self::weapon::fuze::{self, FuzeCollider};
use self::weapon::heat;
use self::weapon::weapon_spec::{Seeker, WeaponCategory, WeaponSpec};
use self::world_collision::WorldCollision;
use self::{
    event_handling::EventSystem,
    plane::{
        messages::{AircraftEvent, AircraftState},
        plane::Plane,
    },
};
use crate::ext::game_object::ColliderType;
use crate::ext::input::InputExt;
use crate::ext::physics_behavior::PhysicsNodeExt;
use crate::ext::subtitles::Subtitle;
use crate::game_settings::GAME_SETTINGS;
use crate::prefabs::camera::{
    self, facing_with_up, CameraControl, Followable, ViewRequest, ANIMATION_CAMERA, COCKPIT_CAMERA,
    PLAYER_CAMERA,
};
use crate::prefabs::island;
use crate::prefabs::missile::{self, Airspace, Missile, TargetTrack};
use crate::prefabs::ocean::{self, Ocean};
use crate::prefabs::plane;
use crate::prefabs::plane::ai::autopilot::Speed;
use crate::prefabs::plane::ai::defense::AiDefense;
use crate::prefabs::plane::aircraft_spec::AircraftSpec;
use crate::prefabs::plane::cruise_speed;
use crate::prefabs::plane::messages::ProfileSwitch;
use crate::scenes::main_menu::ui as main_menu_ui;
use crate::scenes::play::ui as play_ui;
use crate::screen_effects::{ScreenEffects, G_VISION};
use crate::ui::HUD_FONT;
use nalgebra::{vector, Point3, UnitQuaternion, Vector3};
use pankarta_software::prelude::*;
use pankarta_software::rendering::sdf_text::TextFrame;
use pankarta_software::rendering::thick_lines::{convex_mask, LineInstance};
use pankarta_software::scene_manager::node::properties::degrees_from_rotation;
use pankarta_software::scene_manager::node::properties::{Parent, Space, SpawnedLive};
use pankarta_software::tooling::debug_console;
use pankarta_software::ui::ui_node::{UiNode, UiNodeContent};
use std::sync::Arc;

pub struct BlinkingAlert {
    alert_state: bool,
    time_alert: f32,
}

pub struct Play {
    pub blinking_alerts: HashMap<String, BlinkingAlert>,
    pub gravity: Vector3<f32>,
    pub subtitle_data: Subtitle,
    pub start_time: Instant,
    pub event_system: Option<EventSystem>,
    pub game_time: f64,
    was_paused: bool,
    esc_hold_time: f32,
    is_dead: bool,
    // See play::pilot_view.
    pilot_view: PilotView,
    // See play::hud.
    hud: HudState,
    // Who downed whom, top right - see play::ui::kill_feed.
    kill_feed: play_ui::KillFeed,
    // Every plane in the scene - see flight_manager.
    flight_manager: FlightManager,
    // See play::editor.
    editor: EditorState,
    // What the player's camera follows and which camera is on screen - see
    // prefabs::camera.
    camera: CameraControl,
    // See play::tactical.
    tactical: TacticalState,
    // See play::armament.
    weapons: Armament,
    // Aircraft blowing up on the ground and splashing into the sea.
    crash_effects: CrashEffects,
    // Every flare and chaff in the air.
    countermeasures: Countermeasures,
    // The bots' reactions to missiles fired at them - see
    // update_ai_countermeasures.
    ai_defense: AiDefense,
    // The sea - its waves (see crate::prefabs::ocean).
    ocean: Ocean,
    // The land and runway - for the radar altimeter, and for the missiles
    // to fly into (shared with the airspace) - see spawn_world.
    world_collision: Arc<WorldCollision>,
    // The map's own nodes and every node's tracks, kept in the scene - see
    // play::map_nodes.
    map_nodes: MapNodes,
    // Every node's animations running (map.ron's animations) and the paths
    // followed meanwhile - see late_update.
    animations: Animations,
    // See play::debug_tools.
    debug: DebugTools,
}

/// The level's folder - its level_planning.ron (see EventSystem) and
/// map.ron (see flight_manager::spawn_map).
const LEVEL_FOLDER: &str = "./assets/scenes/test_chamber";

impl Play {
    /// Builds the level: its world, its aircraft, its HUD - while the
    /// loading screen shows (see crate::scenes::loading).
    pub fn new(scene: &mut Scene, app: &mut App) -> Self {
        // Play uses the procedural clear-day sea sky.
        app.renderer
            .set_environment(Environment::ProceduralSky { sun: true });
        // What a frame draws while flying, and the line and text sets it
        // draws - see crate::frame_plans.
        crate::frame_plans::register_flight_sets(app);
        app.renderer.frame_plan = crate::frame_plans::flight();
        let ground_trimesh =
            crate::ext::mesh_data::load_trimesh_geometry("models/ground/ground.glb")
                .unwrap_or_else(|error| {
                    eprintln!(
                "play: ground trimesh from 'models/ground/ground.glb' couldn't be loaded: {error}"
            );
                    (vec![], vec![])
                });
        let runway_trimesh = island::trimesh();

        let mut flight_manager = FlightManager::new();
        let mut world_collision = WorldCollision::default();
        Self::spawn_world(
            scene,
            app,
            &mut flight_manager,
            &mut world_collision,
            ground_trimesh,
            runway_trimesh,
        );
        // What the missiles fly through - that land, and (every frame, see
        // publish_airspace) what's in the air.
        let world_collision = Arc::new(world_collision);
        Airspace::spawn(scene, app, world_collision.clone());
        // The map's own nodes (cameras...) and its timeline's tracks.
        let mut map_nodes = MapNodes::default();
        map_nodes.sync(scene, app, flight_manager.map());
        let mut animations = Animations::default();
        animations.start_level(flight_manager.map());

        app.window.set_cursor_locked(true);

        // The player's view is the "camera" node (see spawn_world); the
        // level's animations cut to their own camera nodes (see
        // prefabs::camera::CameraControl::update_view).
        scene.select_camera(PLAYER_CAMERA);

        app.time.time_scale = 1.0;

        // The scene's screen UI - the HUD, the menus, the intro (see play::ui).
        play_ui::spawn(scene, app);
        debug_tools::spawn(scene, app);

        let subtitle_data = Subtitle::new();

        let mut blinking_alerts: HashMap<String, BlinkingAlert> = HashMap::new();
        blinking_alerts.insert(
            "altitude".to_owned(),
            BlinkingAlert {
                alert_state: false,
                time_alert: 0.0,
            },
        );
        blinking_alerts.insert(
            "stall".to_owned(),
            BlinkingAlert {
                alert_state: false,
                time_alert: 0.0,
            },
        );

        let gravity = vector![0.0, -9.81, 0.0];

        let event_system = match EventSystem::new(&Some(LEVEL_FOLDER.to_owned())) {
            Ok(system) => Some(system),
            Err(error) => {
                eprintln!("Error: {}", error);
                None
            }
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
            pilot_view: PilotView {
                hud_hidden_by_g: false,
                crash_time: 0.0,
                crash_seen_from_cockpit: false,
                screen_effects: ScreenEffects::default(),
            },
            hud: HudState {
                collimated_hud: false,
                player_body_velocity: None,
                hmd_enabled: true,
                animation_hides_hud: false,
                hud_boot: None,
                hud_opacity: 1.0,
                hud_layout: HudLayout::default(),
                last_ui_update: Instant::now(),
            },
            kill_feed: play_ui::KillFeed::default(),
            flight_manager,
            editor: EditorState {
                flight_editor_was_open: false,
                flight_editor_map_invalid: false,
                telemetry_last: None,
                flight_editor: FlightEditor::default(),
                editor_active: false,
                spectate_telemetry: None,
            },
            camera: CameraControl::new(),
            tactical: TacticalState {
                tactical_map: TacticalMap::default(),
                shown_plan: ShownPlan::Flight,
                tactical_map_active: false,
                tactical_ui: TacticalUi::default(),
            },
            weapons: Armament {
                fire_control: FireControl::new(),
                weapon_specs: HashMap::new(),
                last_fired_side: -1.0,
                launch_hold: 0.0,
                launch_hold_weapon: None,
                launch_spent: false,
                weapons_status: String::new(),
                countermeasures_status: String::new(),
                countermeasures_column: None,
                cms_cue: None,
                bingo_latched: false,
                bingo_warn_until: None,
                range_cue: None,
                hud_symbol: None,
                hmd_cue: std::cell::Cell::new(None),
                vertical_scan: false,
                lock_held: 0.0,
                lock_hold_used: false,
                selected_weapon: None,
                weapon_list: Vec::new(),
            },
            crash_effects: CrashEffects::default(),
            countermeasures: Countermeasures::default(),
            ai_defense: AiDefense::default(),
            ocean: Ocean::default(),
            world_collision,
            map_nodes,
            animations,
            debug: DebugTools::default(),
        }
    }

    fn spawn_world(
        scene: &mut Scene,
        app: &mut App,
        flight_manager: &mut FlightManager,
        world_collision: &mut WorldCollision,
        ground_trimesh: (Vec<Vector3<f32>>, Vec<[u32; 3]>),
        runway_trimesh: (Vec<Vector3<f32>>, Vec<[u32; 3]>),
    ) {
        // The sun - 40 degrees above the horizon.
        scene
            .spawn_node(
                app,
                crate::scenes::follow_test::scene::sun_node([0.7, 0.7, 0.8]),
            )
            .expect("play should only spawn 'sun' once");

        // Pushed off the origin (was 0,0,0) to make room for the runway
        // island, which now sits at 0,0,0 - see the "runway" node below. Both
        // the mesh and its trimesh collider move with this Transform3D's
        // position, so nothing else needs adjusting.
        let ground_position = Vector3::new(0.0, 0.0, 50_000.0);
        let ground_scale = Vector3::new(30_000.0, 30_000.0, 30_000.0);
        let runway_position = island::POSITION;
        let runway_scale = Vector3::repeat(island::SCALE);
        let sea_level = app.particles.sea_level().unwrap_or(0.0);
        // F10's tactical map: the same land as height contours, uploaded once.
        let contours = [
            contour_lines(
                &ground_trimesh.0,
                &ground_trimesh.1,
                ground_position,
                ground_scale,
                sea_level,
            ),
            contour_lines(
                &runway_trimesh.0,
                &runway_trimesh.1,
                runway_position,
                runway_scale,
                sea_level,
            ),
        ]
        .concat();
        app.renderer
            .render_sets
            .lines("tactical_lines")
            .set_static(&app.renderer.device, &contours);
        // The same land, solid for missiles - the aircraft's is on the
        // physics thread (the colliders below).
        world_collision.add_mesh(
            &ground_trimesh.0,
            &ground_trimesh.1,
            ground_position,
            ground_scale,
        );
        world_collision.add_mesh(
            &runway_trimesh.0,
            &runway_trimesh.1,
            runway_position,
            runway_scale,
        );
        app.renderer
            .render_sets
            .lines("tactical_lines")
            .dynamic
            .clear();

        // Runway island at the origin (the old "ground" island moved out to
        // ground_position above to make room) - solid, its own triangles its
        // collider (see prefabs::island).
        scene
            .spawn_node(app, island::island("runway", Some(runway_trimesh)))
            .expect("play should only spawn 'runway' once");

        // Every aircraft in the level's map.ron, the player's included -
        // see flight_manager::spawn_map.
        flight_manager.spawn_map(scene, app, LEVEL_FOLDER);

        // The player's camera and the animations' - see prefabs::camera.
        CameraControl::spawn(scene, app);

        scene
            .spawn_node(
                app,
                Node::new("fellow_aviator")
                    .with_property(Transform3D {
                        position: Vector3::new(50.0, 50.0, 0.0),
                        rotation: UnitQuaternion::identity(),
                        scale: Vector3::new(19.0, 19.0, 19.0),
                    })
                    .with_property(Model::file("models/F14/F14.gltf"))
                    .with_property(Physics {
                        rigidbody: RigidBodyData {
                            mass: 9000.0,
                            ..RigidBodyData::fixed()
                        },
                        colliders: vec![Collider::new(ColliderShape::Cuboid {
                            half_extents: (1.0, 1.0, 1.0),
                        })],
                    })
                    .with_property(camera::fellow_aviator_cameras()),
            )
            .expect("play should only spawn 'fellow_aviator' once");

        // The whole ocean surface, out past the horizon - kept centred under
        // the camera by Ocean::update.
        scene
            .spawn_node(
                app,
                Node::new("world")
                    .with_property(Transform3D::default())
                    .with_property(ocean::surface()),
            )
            .expect("play should only spawn 'world' once");
    }

    // The flight HUD: timer, FPS, G, power, altitude, speed, compass (all
    // under "game_ui") and the flight path marker.
    const FLIGHT_HUD: [&'static str; 2] = ["hud/game_ui", "hud/velocity_marker"];

    /// Whether the mouse is a pointer for clicking debug windows (F7, F10's
    /// flight editor, the M tactical map, or F11's HUD layout) instead of
    /// looking around.
    fn mouse_free(&self) -> bool {
        self.debug.mouse_free
            || self.tactical.tactical_map_active
            || self.editor.editor_active
            || self.hud.hud_layout.open
    }

    // How long ESC has to be held during an EventSystem::input_lock_end
    // window before it counts as a skip - see `update`'s own handling.
    const SKIP_HOLD_SECONDS: f32 = 1.2;

    // A survivable crash waits this long before the wreck camera - see
    // check_crash.
    const WRECK_VIEW_DELAY_SECONDS: f32 = 0.6;

    /// Stick or keys asking the plane to fly. The editor's fly button (right
    /// mouse) is the camera's, so it doesn't count.
    fn player_flight_input(app: &App) -> bool {
        if app.input.mouse_right_button_down() {
            return false;
        }
        [
            "pitch_up",
            "pitch_down",
            "roll_left",
            "roll_right",
            "rudder_left",
            "rudder_right",
            "throttle_up",
            "throttle_down",
        ]
        .iter()
        .any(|action| app.input.is_action_pressed(action))
    }

    /// The player is flying while an animation has the plane and isn't
    /// playing: let go of that hold and fly the map's own plan from here.
    fn release_player_to_live(&mut self, scene: &mut Scene, app: &App, player_id: &str) {
        if self.editor.flight_editor.typing
            || self.hud.hud_layout.typing
            || !Self::player_flight_input(app)
            || self.animations.playing_on(player_id)
        {
            return;
        }
        if CameraControl::player_camera(scene).is_some_and(|camera| camera.editor_looking()) {
            return;
        }
        let placement = self
            .flight_manager
            .map()
            .find(player_id)
            .map(|(_, placement)| placement.clone());
        let stopped = self.animations.stop_paused(player_id);
        let Some(plane) = scene
            .content
            .nodes
            .get_mut(player_id)
            .and_then(|node| node.get_behavior_mut::<Plane>())
        else {
            return;
        };
        if !stopped && !plane.held && !plane.scripted {
            return;
        }
        plane.held = false;
        plane.scripted = false;
        if let Some(placement) = placement {
            let (points, looped) = self.flight_manager.map().flight_plan(&placement);
            plane.set_path(
                &points,
                Speed::Hold(cruise_speed(&placement)),
                placement.difficulty,
                looped,
            );
        }
    }

    /// An animation is playing with `lock_input` - the player has no
    /// controls meanwhile.
    fn sequence_locks_input(&self) -> bool {
        self.animations.locks_input(self.flight_manager.map())
    }

    /// The level's animations' turn each frame, after every behavior (see
    /// SceneBehaviour::late_update): their clocks on, every animated node
    /// set to its keys, the paths followed, their events carried out, and
    /// the right camera on screen.
    fn run_animations(&mut self, scene: &mut Scene, app: &mut App) {
        // After Plane::update has read this frame's physics state.
        self.collect_pilot_kills(scene);
        if app.time.is_paused() {
            return;
        }
        let frame = self.animations.update(
            app.time.delta_time,
            scene,
            self.flight_manager.map(),
            &mut app.physics,
        );
        self::path_follow::sync_held_bodies(scene, &mut app.physics);
        // Aircraft an animation's placing: nothing the physics says about
        // them counts meanwhile (see Plane::held) - and one an animation's
        // playing on feels no G (Plane::calm).
        for node in scene.content.nodes.iter_mut() {
            let held = node.get_property::<self::path_follow::Held>().is_some();
            let calm = frame.calm.contains(&node.id);
            if let Some(plane) = node.get_behavior_mut::<Plane>() {
                plane.held = held;
                plane.calm = calm;
            }
        }
        let was_hiding_hud = self.hud.animation_hides_hud;
        self.hud.animation_hides_hud = frame.hides_hud;
        for event in frame.events {
            match event {
                AnimAction::PlayAudio(folder) => {
                    app.play_sound(&format!("{}/audio.ogg", crate::asset_relative(&folder)));
                }
                AnimAction::Subtitle { text, seconds } => {
                    let width = app.ui.size().0;
                    if let Some(layer) = scene.ui_layer(crate::ext::subtitles::SUBTITLES_PATH) {
                        self.subtitle_data
                            .add_text(&text, (seconds * 1000.0) as u64, layer, width);
                    }
                }
                // Done by the animations themselves.
                AnimAction::StartAnimation { .. } | AnimAction::StopAnimation { .. } => {}
            }
        }
        // The animation camera where its keys put it, and the right camera on
        // screen - see prefabs::camera.
        if let Some(pose) = &frame.camera {
            CameraControl::set_animation_pose(scene, pose);
        }
        self.camera.update_view(
            scene,
            ViewRequest {
                tactical_map: self.tactical.tactical_map_active,
                editor: self.editor.editor_active,
                preview_camera: self.editor.flight_editor.preview_camera.clone(),
                timeline_preview: self.editor.flight_editor.timeline_preview,
                animating: frame.camera.is_some(),
            },
        );
        self.sync_hud_boot(scene, app, was_hiding_hud);
        // What the camera's on, while spectating - for the editor, or the
        // debug view (F5/F9) in game.
        let debug_view = app.renderer.debug.physics;
        let telemetry = if self.editor.editor_active || debug_view {
            self.telemetry(scene, app.time.delta_time)
        } else {
            None
        };
        self.editor.spectate_telemetry = if self.editor.editor_active {
            None
        } else {
            telemetry.clone()
        };
        self.editor.flight_editor.telemetry = telemetry;
        self.update_editor_gizmos(scene, app);
    }

    /// Whether the aircraft `name` is shot down and still in the air.
    fn falling(&self, scene: &Scene, name: &str) -> bool {
        scene
            .content
            .nodes
            .get(name)
            .and_then(|node| node.get_behavior::<Plane>())
            .is_some_and(Plane::going_down)
    }
}

impl SceneBehaviour for Play {
    /// The scene's frame, before its nodes' behaviors (the planes, the
    /// player's camera...) run: input, the editor and the map, what the
    /// camera follows, weapons, the HUD's state.
    fn update(&mut self, scene: &mut Scene, app: &mut App, _ctx: &mut FrameContext) {
        // The velocity and the rotation of one physics step, together: turned
        // into the jet's frame, it can be carried by the jet's drawn rotation
        // (moved on to this frame) without stepping at the physics' rate.
        if let Some(body) = app.physics.body(self.flight_manager.player_id()) {
            self.hud.player_body_velocity = Some(body.rotation().inverse() * body.linvel());
        }
        // Planes' data.ron edits, applied live - see flight_manager::update.
        self.flight_manager.update(scene, app.time.delta_time);
        // The live map editor (F10's editor view); paused or not.
        self.update_flight_editor(scene, app);
        // The map's own nodes and the tracks on every node, as the map (the
        // editor's, while it's up) has them.
        self.map_nodes.sync(scene, app, self.flight_manager.map());
        // F10 - the tactical map; live, paused or not.
        self.update_tactical_map(scene, app);
        // The aircraft the player flies - its name in map.ron (the editor
        // can switch it).
        let player_id = self.flight_manager.player_id().to_owned();
        let player_id = player_id.as_str();

        // See prefabs::camera::Camera::set_target's own doc comment -
        // has to run unconditionally, before ANY early return below
        // (app.time.is_paused(), self.is_dead), so the "camera" node's own generic
        // Behavior tick (which always runs every frame regardless of what
        // this fn does - see App::run's own unconditional node-tick call)
        // never reads a target one frame stale relative to whatever physics
        // just wrote into "player"'s own renderable transform this frame.
        //
        // What the missiles fly through this frame, before they fly
        // (their own behaviors, after this) - see publish_airspace.
        if !app.time.is_paused() {
            self.publish_airspace(scene, app);
            self.update_ai_countermeasures(scene, app);
            self.show_crashes(scene, app);
        }

        // What the camera follows and its target - see prefabs::camera.
        let aircraft: Vec<&str> = self
            .flight_manager
            .map()
            .aircraft()
            .map(|(_, aircraft)| aircraft.name.as_str())
            .collect();
        self.camera.update(
            scene,
            app,
            &Followable {
                player_id,
                aircraft,
                editor_selection: self.editor.flight_editor.selected_aircraft(),
                editor_active: self.editor.editor_active,
                editor_typing: self.editor.flight_editor.typing,
            },
        );

        // Every frame, paused or not - the effects only last one frame, and
        // the view shouldn't pop back to normal behind the pause menu.
        self.update_g_vision(scene, app);
        // The HUD's power-on, if a cinematic just handed the cockpit back.
        // Paused, it holds whatever brightness it had.
        self.tick_hud_boot(app, !app.time.is_paused());

        // F3, F5, F7 - see play::debug_tools.
        self.update_debug_tools(scene, app);

        // Whether player input is locked out right now (see EventSystem::
        // input_lock_end's own doc comment) - checked against this frame's
        // not-yet-incremented game_time, one frame stale at worst, which
        // doesn't matter for a coarse window check like this.
        let input_lock_end = self
            .event_system
            .as_ref()
            .and_then(|es| es.input_lock_end(self.game_time));
        if let Some(node) = scene.ui_node(play_ui::SKIP_PROMPT) {
            node.set_active(input_lock_end.is_some());
        }
        // Locked (the intro, an animation playing - from its first frame):
        // no flying and no looking round either - the camera eases back to
        // its center. And the editor camera flying (right button held) keeps
        // the plane's controls off too.
        let controls_locked = input_lock_end.is_some() || self.sequence_locks_input();
        let mut editor_flying = false;
        let (mouse_free, overlay_up) = (
            self.mouse_free(),
            self.debug_overlay_visible() || self.tactical.tactical_map_active,
        );
        let mut stop_timeline_preview = false;
        if let Some(camera) = CameraControl::player_camera_mut(scene) {
            camera.look_locked = controls_locked;
            camera.mouse_free = mouse_free;
            camera.overlay_up = overlay_up;
            stop_timeline_preview = std::mem::take(&mut camera.stop_timeline_preview);
            editor_flying = camera.editor_looking();
        }
        if stop_timeline_preview {
            self.editor.flight_editor.timeline_preview = false;
        }
        self.editor.flight_editor.pitch_inputs = (
            app.input.action_strength("pitch_down"),
            app.input.action_strength("pitch_up"),
        );
        // The engine's debug UI (egui) is up while any of the scene's windows
        // are - it frees the mouse while it is.
        app.debug_ui.visible = self.any_debug_window();
        // Mouse-look while flying; a pointer while a window wants it.
        if !app.time.is_paused() && !editor_flying {
            app.window.set_cursor_locked(!self.mouse_free());
        }

        if let Some(lock_end_ms) = input_lock_end {
            // Only a "hold ESC to skip" affordance works during a locked
            // sequence - not the pause menu itself (see EventSystem::
            // input_lock_end's own doc comment on why input locking is its
            // own concept rather than reusing is_cinematic_camera_active).
            if app.input.is_action_pressed("toggle_pause_menu") {
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
            // actually clears app.time.is_paused() again on resume.
            if app.input.is_action_just_pressed("toggle_pause_menu") {
                if app.time.is_paused() {
                    let mouse_free = self.mouse_free();
                    play_ui::close_pause_menu(scene, app);
                } else {
                    play_ui::open_pause_menu(scene, app);
                }
            }
        }

        // Rising/falling edge, same idiom as the cinematic pause/resume below -
        // no SetTransform teleport needed here (unlike the cinematic case,
        // nothing else is scripting the plane's position while paused, so
        // physics's own resting state is exactly where it should resume from).
        // Also has to run before the early-return, since it's what actually
        // (un)pauses physics.
        // (The engine stops stepping physics on its own while paused - time
        // scale 0, see play::ui::open_pause_menu.)
        self.was_paused = app.time.is_paused();

        if app.time.is_paused() {
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
            // main_menu::scene::Play::update already uses unconditionally
            // every frame, scoped here to just while paused instead.
            // The pause menu's Settings panel is main_menu's own - its
            // sliders need following while dragged here too.
            main_menu_ui::update_settings_sliders(scene, app);
            main_menu_ui::refresh_controller_states(scene, app);
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
        if let Some(plane) = scene
            .content
            .nodes
            .get_mut(player_id)
            .and_then(|node| node.get_behavior_mut::<Plane>())
        {
            // Typing in the flight editor isn't flying.
            plane.input_locked = input_lock_end.is_some()
                || self.editor.flight_editor.typing
                || self.hud.hud_layout.typing
                || self.sequence_locks_input()
                || editor_flying;
        }

        self.check_crash(scene, app);
        let has_control = input_lock_end.is_none()
            && !self.editor.flight_editor.typing
            && !self.hud.hud_layout.typing
            && !self.sequence_locks_input();
        self.update_weapons(scene, app, has_control);

        if let Some(event_system) = &mut self.event_system {
            event_system.handle_events(self.game_time, scene, app, &mut self.subtitle_data);
        }
        if let Some(layer) = scene.ui_layer(crate::ext::subtitles::SUBTITLES_PATH) {
            self.subtitle_data.update(layer);
        }
        self.ui_control(scene, app);

        if let Some(event_system) = &self.event_system {
            event_system.apply_tracks(self.game_time, scene);
        }
        if app.input.is_action_just_pressed("toggle_collimated_hud") {
            self.hud.collimated_hud = !self.hud.collimated_hud;
        }
        if app.input.is_action_just_pressed("toggle_hmd") {
            self.hud.hmd_enabled = !self.hud.hmd_enabled;
        }
        if app.input.is_action_just_pressed("toggle_hud_layout")
            && !self.editor.flight_editor.typing
            && !self.hud.hud_layout.typing
        {
            self.hud.hud_layout.open = !self.hud.hud_layout.open;
            if !self.hud.hud_layout.open {
                self.hud.hud_layout.typing = false;
            }
            if !app.time.is_paused() {
                app.window.set_cursor_locked(!self.mouse_free());
            }
        }
        self.update_lock_arrow(scene, app);
        self.update_heat_view(scene, app);
        // Not over F10's map or the editor.
        let delta_time = if app.time.is_paused() {
            0.0
        } else {
            app.time.delta_time
        };
        let shown = !(self.tactical.tactical_map_active || self.editor.editor_active);
        self.kill_feed
            .update(scene, delta_time, app.window.size.width as f32, shown);
        self.hide_hud(scene, app);
    }

    /// After every behavior: the level's animations (see run_animations),
    /// then the named emitters onto the engine's.
    fn late_update(&mut self, scene: &mut Scene, app: &mut App, _ctx: &mut FrameContext) {
        self.run_animations(scene, app);
        // The game's named emitters onto the engine's emitter nodes - after
        // every behavior has driven them this frame.
        crate::ext::emitters::sync(scene, app);
    }

    fn after_cameras(&mut self, scene: &mut Scene, app: &mut App) {
        let camera = scene.cameras.active().camera.position().coords;
        // The map's lines are drawn relative to its camera.
        crate::frame_plans::set_tactical_camera_position(camera);
        // The sea's waves, around where the camera is drawn from.
        self.ocean.update(scene, "world", app.time.delta_time);
        // The G-effects and the HUD's brightness, into the flight plan's
        // Fullscreen step - this frame's effects used up (see update_g_vision).
        app.renderer.shaders.set_instance(
            G_VISION,
            G_VISION,
            &std::mem::take(&mut self.pilot_view.screen_effects).params(self.hud.hud_opacity),
        );
        // The M map draws instead of the flight view - its lines alone (3D),
        // or the world seen straight down under them (Chart). Its own frame
        // plan, swapped in whenever it's opened, closed or switched.
        let wanted = if !self.tactical.tactical_map_active {
            ShownPlan::Flight
        } else if self.tactical.tactical_ui.top_down {
            ShownPlan::Chart
        } else {
            ShownPlan::Map
        };
        if wanted != self.tactical.shown_plan {
            self.tactical.shown_plan = wanted;
            app.renderer.frame_plan = match wanted {
                ShownPlan::Flight => crate::frame_plans::flight(),
                ShownPlan::Map => crate::frame_plans::tactical_map(),
                ShownPlan::Chart => crate::frame_plans::tactical_chart(),
            };
        }
        // While paused the HUD stays as the last flying frame left it.
        if !app.time.is_paused() {
            self.update_hud(scene, app);
        }
        // The cockpit camera polls the tracker just before this. A frame that
        // arrives during startup or Retry rewrites the settings dot here,
        // including while the pause menu is up.
        main_menu_ui::refresh_head_tracking_if_needed(scene, app);
    }

    fn debug_ui(&mut self, ctx: &egui::Context, scene: &mut Scene, app: &mut App) {
        self.draw_debug_ui(ctx, scene, app);
    }
}
