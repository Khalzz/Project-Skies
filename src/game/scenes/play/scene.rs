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
use super::plane::messages::ProfileSwitch;
use super::ai_defense::AiDefense;
use super::map::Difficulty;
use super::weapon::countermeasures::{Countermeasures, Release};
use super::weapon::heat;
use super::weapon::weapon_spec::{Seeker, WeaponCategory, WeaponSpec};
use super::weapon::fire_control::{self, Contact, FireControl, OwnShip};
use super::weapon::fuze::{self, FuzeCollider};
use super::weapon::missile::TargetTrack;
use super::weapon::missiles::{BlastEvent, Missiles};
use super::crash_effects::{Crash, CrashEffects};
use super::world_collision::WorldCollision;
use crate::engine::rendering::render_pipeline::blur_renderer::ScreenEffects;
use crate::engine::ui::components::label::text_width;
use crate::game::ui::HUD_FONT;
use crate::engine::rendering::sdf_text::TextFrame;
use crate::engine::rendering::thick_lines::{convex_mask, LineInstance};
use super::camera::head_motion::HeadInput;
use super::{camera::camera::{Camera, CameraState, COCKPIT_CAMERA}, event_handling::EventSystem, plane::{messages::{AircraftEvent, AircraftState}, plane::Plane, quick_list}};
use std::sync::mpsc::Sender;
use crate::game::game_settings::GAME_SETTINGS;
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
    // F6 - the center HUD is drawn far ahead of the jet and only shows
    // through its combiner glass. Off: that center stays stuck to the screen.
    collimated_hud: bool,
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
    // How long fire_weapon has been held toward a launch (s), for which
    // weapon, and whether this press already fired.
    launch_hold: f32,
    launch_hold_weapon: Option<String>,
    launch_spent: bool,
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
    // The seeker's symbol this frame (its diamond, or the box on what it's
    // locked) - set by update_weapons, drawn by update_hud.
    hud_symbol: Option<HudSymbol>,
    // The player's velocity in its own frame (x left, y up, z forward), from
    // the latest physics pose - see update_hud's velocity marker.
    player_body_velocity: Option<Vector3<f32>>,
    // Where the pilot looks while the helmet display is up (a world
    // direction) - its cross; None otherwise. Set by update_hud (after the
    // camera has moved), read by the next update_weapons to point the
    // seeker in the Fox-2 mode (see FireControl::cue).
    hmd_cue: std::cell::Cell<Option<Vector3<f32>>>,
    // Which of the scene's frame plans is up - the M map's or flying's (see
    // game::frame_plans) - so it's swapped only when the map opens or closes.
    showing_map_plan: bool,
    // The helmet display is switched on ("toggle_hmd") - see draw_hmd.
    hmd_enabled: bool,
    // The vertical lock is on: the first enemy through the strip over the
    // nose gets locked (see fire_control::in_vertical_scan) - for as long as
    // the lock key is held, see update_weapons.
    vertical_scan: bool,
    // How long the lock key has been held (s), and whether this hold already
    // went into the vertical lock - so letting go isn't a tap too.
    lock_held: f32,
    lock_hold_used: bool,
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

/// Live cockpit HUD layout, edited from the F11 panel. The numbers are what
/// the HUD used before the panel existed. They last for the session.
#[derive(Clone, Copy)]
pub struct HudLayout {
    pub open: bool,
    /// Width and height as fractions of the screen, before `scale`.
    pub width: f32,
    pub height: f32,
    /// Shrinks the whole HUD around its center, letters included.
    /// 1 is the built-in size. Width and height still set the frame.
    pub scale: f32,
    /// Pixels from the screen center. Positive X is right, positive Y is down.
    pub center_x: f32,
    pub center_y: f32,
    /// How much of the combiner the center HUD covers when F6 is on.
    /// 1 sits the frame on the pane. Higher magnifies it, and the glass clips
    /// whatever crosses the edge — past about 2 the side marks are cut off.
    pub glass_fill: f32,
    /// Metres in front of the design eye, when the HUD stays with the jet.
    pub plate_distance: f32,
    /// Metres right and up from that plate's center.
    pub plate_shift_right: f32,
    pub plate_shift_up: f32,
    /// A number field in the panel has focus — flight keys stay off.
    pub typing: bool,
}

impl Default for HudLayout {
    fn default() -> Self {
        Self {
            open: false,
            width: 0.333,
            height: 0.5,
            scale: 0.35,
            center_x: 0.0,
            center_y: 0.0,
            glass_fill: 0.9,
            plate_distance: 0.85,
            plate_shift_right: 0.0,
            plate_shift_up: 0.07,
            typing: false,
        }
    }
}

impl HudLayout {
    /// Back to the built-in layout. The panel stays however it was.
    fn reset_values(&mut self) {
        let open = self.open;
        let typing = self.typing;
        *self = Self::default();
        self.open = open;
        self.typing = typing;
    }

    /// The scale the HUD is drawn at. Stepped so a drag doesn't mint a new
    /// font size every frame (that fills the text atlas until it panics).
    fn draw_scale(self) -> f32 {
        let scale = self.scale.clamp(0.1, 4.0);
        (scale * 20.0).round() / 20.0
    }
}

fn hud_readout_font(key: &str) -> f32 {
    play_ui::HUD_READOUTS.iter().find(|(name, _, _, _)| *name == key).map(|(_, _, size, _)| *size).unwrap_or(12.0)
}

/// Font scale for the atlas: the layout scale times whatever the glass
/// zoom already applied, on the same 1/16 steps that zoom uses.
fn hud_font_scale(layout_scale: f32, print_scale: f32) -> f32 {
    let scale = (layout_scale * print_scale).clamp(0.1, 6.0);
    (scale * 16.0).round() / 16.0
}

/// F11's HUD layout panel. `Some` is text for the clipboard.
pub fn draw_hud_layout(ctx: &egui::Context, layout: &mut HudLayout) -> Option<String> {
    let mut copied = None;
    let mut open = layout.open;
    egui::Window::new("HUD layout")
        .open(&mut open)
        .default_pos(egui::pos2(20.0, 20.0))
        .resizable(false)
        .show(ctx, |ui| {
            ui.label("F11 closes this. Drag a number, or click it and type.");
            ui.separator();

            ui.label("Size");
            ui.add(egui::Slider::new(&mut layout.width, 0.02..=2.5).text("width"));
            ui.add(egui::Slider::new(&mut layout.height, 0.02..=2.5).text("height"));
            ui.label("Fractions of the screen. The frame can run past the edges.");
            ui.add(egui::Slider::new(&mut layout.scale, 0.15..=3.0).step_by(0.05).text("scale"));
            ui.label("Shrinks the whole HUD, letters included, without a separate text size.");
            ui.separator();

            ui.label("Center");
            ui.add(egui::Slider::new(&mut layout.center_x, -2500.0..=2500.0).text("X"));
            ui.add(egui::Slider::new(&mut layout.center_y, -2500.0..=2500.0).text("Y"));
            ui.label("Pixels from the screen center. Positive X is right, positive Y is down.");
            ui.separator();

            ui.label("Combiner glass (F6)");
            ui.add(egui::Slider::new(&mut layout.glass_fill, 0.05..=5.0).text("fill"));
            ui.separator();

            ui.label("Jet plate");
            ui.label("Used when Video has the HUD stay with the jet. It's drawn at infinity (collimated), like a real HUD; distance only sets how far the shifts turn it.");
            ui.add(egui::Slider::new(&mut layout.plate_distance, 0.05..=6.0).text("distance").suffix(" m"));
            ui.add(egui::Slider::new(&mut layout.plate_shift_right, -1.5..=1.5).text("right").suffix(" m"));
            ui.add(egui::Slider::new(&mut layout.plate_shift_up, -1.5..=1.5).text("up").suffix(" m"));
            ui.separator();

            ui.horizontal(|ui| {
                if ui.button("Reset").clicked() {
                    layout.reset_values();
                }
                if ui.button("Copy values").clicked() {
                    copied = Some(format!(
                        "width: {:.3}\nheight: {:.3}\nscale: {:.2}\ncenter: ({:.0}, {:.0}) px\nglass fill: {:.2}\nplate distance: {:.2} m\nplate shift: right {:.3}, up {:.3} m",
                        layout.width, layout.height, layout.scale, layout.center_x, layout.center_y,
                        layout.glass_fill, layout.plate_distance, layout.plate_shift_right, layout.plate_shift_up,
                    ));
                }
            });
        });
    layout.open = open;
    layout.typing = open && ctx.wants_keyboard_input();
    copied
}

/// The center HUD's layout, drawn on the combiner glass itself (the jet's
/// frame). Copied into the line closure so the glass outline stays free
/// for the labels.
#[derive(Clone, Copy)]
struct GlassMap {
    aircraft_pos: Vector3<f32>,
    aircraft_rot: UnitQuaternion<f32>,
    /// The eye pitch and heading are measured from, in the jet's frame: on
    /// the glass (F6) the resting cockpit eye, so head bobbing doesn't slide
    /// the marks on the pane; on the collimated plate the eye this frame,
    /// which the plate moves with (see `fixed_hud_plane`).
    eye: Vector3<f32>,
    center: Vector3<f32>,
    /// Glass plane, in the jet's frame: through `center`, facing the eye.
    boresight: Vector3<f32>,
    right: Vector3<f32>,
    up: Vector3<f32>,
    r_min: f32,
    r_max: f32,
    u_min: f32,
    u_max: f32,
    left: f32,
    right_px: f32,
    top: f32,
    bottom: f32,
}

impl GlassMap {
    /// Where HUD pixel `(x, y)` sits in the world, on the glass.
    fn at(&self, x: f32, y: f32) -> Vector3<f32> {
        let u = (x - self.left) / (self.right_px - self.left);
        let v = (y - self.top) / (self.bottom - self.top);
        let along_right = self.r_min + (self.r_max - self.r_min) * u;
        let along_up = self.u_max + (self.u_min - self.u_max) * v;
        self.on_glass(self.center + self.right * along_right + self.up * along_up)
    }

    /// Meters on the glass of one HUD pixel across it.
    fn meters_per_pixel(&self) -> f32 {
        let glass_width = (self.r_max - self.r_min).max(1e-3);
        let hud_width = (self.right_px - self.left).max(1.0);
        glass_width / hud_width
    }

    /// Text centered on `center` (in the world, on the glass), lying on the
    /// glass, `font_size` HUD pixels tall - so it grows and shrinks with
    /// the glass on screen, like the marks drawn on it.
    fn text_frame(&self, center: Vector3<f32>, font_size: f32) -> TextFrame {
        let meters_per_pixel_up = (self.u_max - self.u_min).max(1e-3) / (self.bottom - self.top).max(1.0);
        TextFrame {
            center,
            right: self.aircraft_rot * self.right * (font_size * self.meters_per_pixel()),
            down: -(self.aircraft_rot * self.up) * (font_size * meters_per_pixel_up),
        }
    }

    /// The design eye, in the world.
    fn eye_world(&self) -> Vector3<f32> {
        self.aircraft_pos + self.aircraft_rot * self.eye
    }

    fn on_glass(&self, local: Vector3<f32>) -> Vector3<f32> {
        self.aircraft_pos + self.aircraft_rot * local
    }

    fn plane_normal(&self) -> Vector3<f32> {
        self.aircraft_rot * self.boresight
    }

    /// Where the ray from the design eye along `dir_world` meets the glass.
    fn ray_to_glass(&self, dir_world: Vector3<f32>) -> Option<Vector3<f32>> {
        self.ray_from(self.eye_world(), dir_world)
    }

    /// Where the ray from `eye` (in the world) along `dir_world` meets the
    /// glass.
    fn ray_from(&self, eye: Vector3<f32>, dir_world: Vector3<f32>) -> Option<Vector3<f32>> {
        let normal = self.plane_normal();
        let denom = normal.dot(&dir_world);
        if denom.abs() < 1e-5 {
            return None;
        }
        let t = normal.dot(&(self.on_glass(self.center) - eye)) / denom;
        if t < 1e-3 {
            return None;
        }
        Some(eye + dir_world * t)
    }

    /// The nearest point of the glass plane to `world`.
    fn cling(&self, world: Vector3<f32>) -> Vector3<f32> {
        let origin = self.on_glass(self.center);
        let normal = self.plane_normal();
        world - normal * (world - origin).dot(&normal)
    }

    /// `world` on this plate, as metres right and up from the center.
    fn plane_coords(&self, world: Vector3<f32>) -> (f32, f32) {
        let local = self.aircraft_rot.inverse() * (world - self.aircraft_pos);
        let delta = local - self.center;
        (delta.dot(&self.right), delta.dot(&self.up))
    }

    fn from_plane(&self, x: f32, y: f32) -> Vector3<f32> {
        self.on_glass(self.center + self.right * x + self.up * y)
    }

    /// The part of the segment that lies inside the HUD square. None when it misses.
    fn clip_to_frame(&self, start: Vector3<f32>, end: Vector3<f32>) -> Option<(Vector3<f32>, Vector3<f32>)> {
        let (x0, y0) = self.plane_coords(start);
        let (x1, y1) = self.plane_coords(end);
        let (x0, y0, x1, y1) = clip_segment_rect(x0, y0, x1, y1, self.r_min, self.r_max, self.u_min, self.u_max)?;
        Some((self.from_plane(x0, y0), self.from_plane(x1, y1)))
    }

    /// Where the eye's line along `dir_world` crosses the plate, held inside
    /// the HUD square `margin` metres in from its edge: past the edge (or
    /// pointing away from the plate altogether), it's brought in toward the
    /// square's middle along the same way across it - so it stops at the
    /// edge on the side it went off.
    fn ray_held_in_frame(&self, dir_world: Vector3<f32>, margin: f32) -> Vector3<f32> {
        let (x, y, _) = self.held_in_frame(dir_world, margin);
        self.from_plane(x, y)
    }

    /// `ray_held_in_frame` on the plate (metres right and up from its
    /// center), and which way across the plate the line went from the
    /// square's middle (a unit vector, right and up) - straight up when it
    /// goes through the middle.
    fn held_in_frame(&self, dir_world: Vector3<f32>, margin: f32) -> (f32, f32, nalgebra::Vector2<f32>) {
        let (x, y) = match self.ray_to_glass(dir_world) {
            Some(hit) => self.plane_coords(hit),
            None => {
                // Away from the plate: only which way across it counts.
                let local = self.aircraft_rot.inverse() * dir_world;
                (local.dot(&self.right) * 1e6, local.dot(&self.up) * 1e6)
            }
        };
        let (middle_x, middle_y) = ((self.r_min + self.r_max) * 0.5, (self.u_min + self.u_max) * 0.5);
        let half_x = ((self.r_max - self.r_min) * 0.5 - margin).max(0.0);
        let half_y = ((self.u_max - self.u_min) * 0.5 - margin).max(0.0);
        let (dx, dy) = (x - middle_x, y - middle_y);
        let shrink_x = if dx.abs() > half_x { half_x / dx.abs() } else { 1.0 };
        let shrink_y = if dy.abs() > half_y { half_y / dy.abs() } else { 1.0 };
        let shrink = shrink_x.min(shrink_y);
        let way = nalgebra::Vector2::new(dx, dy).try_normalize(1e-9).unwrap_or_else(|| nalgebra::Vector2::new(0.0, 1.0));
        (middle_x + dx * shrink, middle_y + dy * shrink, way)
    }

    /// Whether `world` sits inside the HUD square, `margin` metres in from the edge.
    fn inside_frame(&self, world: Vector3<f32>, margin: f32) -> bool {
        let (x, y) = self.plane_coords(world);
        x >= self.r_min + margin && x <= self.r_max - margin && y >= self.u_min + margin && y <= self.u_max - margin
    }
}

/// Liang–Barsky clip of a segment to an axis-aligned rectangle.
fn clip_segment_rect(x0: f32, y0: f32, x1: f32, y1: f32, min_x: f32, max_x: f32, min_y: f32, max_y: f32) -> Option<(f32, f32, f32, f32)> {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let mut t0 = 0.0;
    let mut t1 = 1.0;
    let mut clip = |p: f32, q: f32| -> bool {
        if p.abs() < 1e-8 {
            return q >= 0.0;
        }
        let t = q / p;
        if p < 0.0 {
            if t > t1 {
                return false;
            }
            if t > t0 {
                t0 = t;
            }
        } else {
            if t < t0 {
                return false;
            }
            if t < t1 {
                t1 = t;
            }
        }
        true
    };
    if !clip(-dx, x0 - min_x) || !clip(dx, max_x - x0) || !clip(-dy, y0 - min_y) || !clip(dy, max_y - y0) {
        return None;
    }
    Some((x0 + t0 * dx, y0 + t0 * dy, x0 + t1 * dx, y0 + t1 * dy))
}

/// The jet's combiner glass this frame: the far-plane map, and the glass's
/// outline on screen (the clip). `visible` is false when the glass isn't in
/// front of the camera - the center HUD is then not drawn.
struct CollimatedHud {
    map: GlassMap,
    visible: bool,
    gpu_mask: Option<Vec<[f32; 3]>>,
    /// Line widths, relative to the screen layout. Tracks how big the glass
    /// is on screen, so zooming in grows the strokes with the pane (the text
    /// lies on the glass itself - see `GlassMap::text_frame`).
    print_scale: f32,
}

/// Where `world` lands in UI pixels. Unlike `world_to_screen`, a point in
/// front of the camera is returned even when it falls outside the screen,
/// so the glass outline stays intact when it crosses the edge.
fn project_hud_point(active: &crate::engine::rendering::camera::handler::CameraInstance, world: Vector3<f32>, screen_width: f32, screen_height: f32) -> Option<(f32, f32)> {
    let camera_to_point = world - active.camera.position().coords;
    let forward = active.camera.rotation_modifier * active.camera.calc_forward_direction();
    if camera_to_point.dot(&forward) <= 1e-3 {
        return None;
    }
    // The uploaded view_proj is still the previous frame's here. This is the
    // matrix the upcoming draw will use, so the clip matches the glass.
    let clip = (active.projection.calc_matrix() * active.camera.calc_matrix()) * Point3::from(camera_to_point).to_homogeneous();
    if clip.w <= 1e-4 {
        return None;
    }
    let ndc = clip.xyz() / clip.w;
    // A point grazing the camera divides out to a huge screen position.
    // That was being read as the glass's on-screen size, and the labels'
    // font followed it until the text atlas overflowed (AtlasFull).
    if !ndc.x.is_finite() || !ndc.y.is_finite() || ndc.x.abs() > 8.0 || ndc.y.abs() > 8.0 {
        return None;
    }
    let x = (ndc.x + 1.0) * 0.5 * screen_width.max(1.0);
    let y = (1.0 - (ndc.y + 1.0) * 0.5) * screen_height.max(1.0);
    Some((x, y))
}

/// How big the plate is on screen against the layout it was drawn for - its
/// height, top to bottom through its middle, so rolling the head doesn't
/// change it. Zooming in grows it; the line widths follow (the text is on the
/// plate itself - see `GlassMap::text_frame`).
fn plate_print_scale(active: &crate::engine::rendering::camera::handler::CameraInstance, map: &GlassMap, screen_width: f32, screen_height: f32) -> f32 {
    let middle = (map.left + map.right_px) * 0.5;
    let Some((x0, y0)) = project_hud_point(active, map.at(middle, map.top), screen_width, screen_height) else { return 1.0 };
    let Some((x1, y1)) = project_hud_point(active, map.at(middle, map.bottom), screen_width, screen_height) else { return 1.0 };
    let plate_span = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
    (plate_span / (map.bottom - map.top).max(1.0)).clamp(0.1, 10.0)
}

/// How far out the collimated plate is drawn (m) - far enough that a head
/// moving a few cm shifts it by nothing you could see (5 cm: 0.003°), near
/// enough to stay well inside the camera's far plane.
const COLLIMATED_DISTANCE: f32 = 1000.0;

/// The HUD as a real one shows it: collimated, its image at optical
/// infinity. It turns with the jet (aimed along the resting cockpit view)
/// but sits out at COLLIMATED_DISTANCE from the eye wherever the eye is, so
/// moving the head doesn't slide it against the world - the flight path
/// marker stays on the flight path, the horizon line on the horizon. What
/// the head does move is the glass it's seen through (update_hud's
/// `plate_mask`). Rolling the camera leaves it where it is too. The F11
/// layout keeps its meaning: the plate covers the same angles a plate
/// `plate_distance` in front of the eye, shifted by `plate_shift_*`, would.
/// Layout pixels `(left, top)`..`(right, bottom)` cover the plate.
fn fixed_hud_plane(scene: &Scene, player_id: &str, screen_width: f32, screen_height: f32, left: f32, right: f32, top: f32, bottom: f32, layout: &HudLayout) -> Option<GlassMap> {
    let instance = scene.content.renderizable_instances.get(player_id)?;
    let aircraft_pos = instance.instance.transform.position;
    let aircraft_rot = instance.instance.transform.rotation;
    // The eye this frame, in the jet's frame - head motion and all.
    let eye = aircraft_rot.inverse() * (scene.cameras.active().camera.position().coords - aircraft_pos);
    let view_pitch = GAME_SETTINGS.lock().unwrap().cockpit_view_pitch_deg;
    // Same sign as the cockpit camera: a positive view angle looks up, and
    // a positive pitch about X looks down.
    let rest = UnitQuaternion::from_axis_angle(&Vector3::x_axis(), (-view_pitch).to_radians());
    let boresight = (rest * Vector3::z()).try_normalize(1e-4)?;
    // The layout's plate, scaled out about the eye to COLLIMATED_DISTANCE -
    // every angle from the eye stays what the layout gives.
    let reference = layout.plate_distance.max(0.05);
    let out = COLLIMATED_DISTANCE / reference;
    let distance = COLLIMATED_DISTANCE;
    let up_raw = rest * Vector3::y();
    let up = (up_raw - boresight * up_raw.dot(&boresight)).try_normalize(1e-4)?;
    let right_axis = boresight.cross(&up);
    let center = eye + (boresight * reference + right_axis * layout.plate_shift_right + up * layout.plate_shift_up) * out;
    // The plate's size is the layout's angle at the resting field of view,
    // so it matches the on-screen HUD when the head is centered.
    let fovy = 70.0_f32.to_radians();
    let span_x = (right - left).max(1.0);
    let span_y = (bottom - top).max(1.0);
    let screen_height = screen_height.max(1.0);
    let screen_width = screen_width.max(1.0);
    let angle_y = fovy * span_y / screen_height;
    let angle_x = fovy * (screen_width / screen_height) * span_x / screen_width;
    let half_r = distance * (angle_x * 0.5).tan();
    let half_u = distance * (angle_y * 0.5).tan();
    Some(GlassMap {
        aircraft_pos,
        aircraft_rot,
        eye,
        center,
        boresight,
        right: right_axis,
        up,
        r_min: -half_r,
        r_max: half_r,
        u_min: -half_u,
        u_max: half_u,
        left,
        right_px: right,
        top,
        bottom,
    })
}

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
        // What a frame draws while flying, and the line and text sets it
        // draws - see game::frame_plans.
        crate::game::frame_plans::register_flight_sets(app);
        app.frame_plan = crate::game::frame_plans::flight();
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
        app.hud_layout.open = false;
        app.hud_layout.typing = false;

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
            if key == "hud_radar_altitude" {
                readout = readout.set_align(glyphon::cosmic_text::Align::Left);
            }
            if key == "hud_corner_fuel" {
                readout = readout.set_align(glyphon::cosmic_text::Align::Right);
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
            collimated_hud: false,
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
            launch_hold: 0.0,
            launch_hold_weapon: None,
            launch_spent: false,
            weapons_status: String::new(),
            countermeasures_status: String::new(),
            countermeasures_column: None,
            cms_cue: None,
            bingo_latched: false,
            bingo_warn_until: None,
            countermeasures: Countermeasures::default(),
            ai_defense: AiDefense::default(),
            range_cue: None,
            hud_symbol: None,
            player_body_velocity: None,
            hmd_cue: std::cell::Cell::new(None),
            showing_map_plan: false,
            hmd_enabled: true,
            vertical_scan: false,
            lock_held: 0.0,
            lock_hold_used: false,
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
        app.render_sets.lines("tactical_lines").set_static(&app.renderer.device, &contours);
        // The same land, solid for missiles - the aircraft's is on the
        // physics thread (the colliders below).
        world_collision.add_mesh(&ground_trimesh.0, &ground_trimesh.1, ground_position, ground_scale);
        world_collision.add_mesh(&runway_trimesh.0, &runway_trimesh.1, runway_position, runway_scale);
        app.render_sets.lines("tactical_lines").dynamic.clear();
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
            cameras.insert("cockpit".to_owned(), GameObjectCamera { position: Vector3::new(0.0, 2.3, 14.3), fov: 60.0, head_limits: None });
            cameras.insert("front".to_owned(), GameObjectCamera { position: Vector3::new(0.0, 6.0, 26.0), fov: 60.0, head_limits: None });
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
                .add_property(crate::engine::scene_manager::properties::Material::shader(crate::engine::rendering::shader_library::WATER))
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
    // physics_data: the poses the physics thread sent since last frame
    // (empty on a frame it sent none) - only read here for the player's
    // velocity in its own frame (see player_body_velocity); Plane::
    // fixed_update reacts to physics results generically.
    pub fn update(&mut self, scene: &mut Scene, app: &mut App, physics_command_tx: Option<&Sender<PhysicsCommand>>, physics_data: &HashMap<String, RenderMessage>) {
        // The velocity and the rotation of one physics step, together: turned
        // into the jet's frame, it can be carried by the jet's drawn rotation
        // (moved on to this frame) without stepping at the physics' 120 Hz.
        if let Some(pose) = physics_data.get(self.flight_manager.player_id()) {
            self.player_body_velocity = Some(nalgebra::Unit::new_normalize(pose.rotation).inverse() * pose.linvel);
        }
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
            plane.input_locked = input_lock_end.is_some() || app.flight_editor.typing || app.hud_layout.typing || self.sequence_locks_input() || editor_flying;
        }

        Self::update_water_plane(scene);
        Self::update_water_wake(scene, app, player_id);
        self.check_crash(scene, app);
        let has_control = input_lock_end.is_none() && !app.flight_editor.typing && !app.hud_layout.typing && !self.sequence_locks_input();
        self.update_weapons(scene, app, has_control);

        if let Some(event_system) = &mut self.event_system {
            event_system.handle_events(self.game_time, app, &mut self.subtitle_data);
        }
        self.subtitle_data.update(app);
        self.ui_control(scene, app);

        if let Some(event_system) = &self.event_system {
            event_system.apply_tracks(self.game_time, app);
        }
        if input::is_action_just_pressed("toggle_collimated_hud") {
            self.collimated_hud = !self.collimated_hud;
        }
        if input::is_action_just_pressed("toggle_hmd") {
            self.hmd_enabled = !self.hmd_enabled;
        }
        if input::is_action_just_pressed("toggle_hud_layout") && !app.flight_editor.typing && !app.hud_layout.typing {
            app.hud_layout.open = !app.hud_layout.open;
            if !app.hud_layout.open {
                app.hud_layout.typing = false;
            }
            if !app.is_paused {
                app.window_manager.context.mouse().set_relative_mouse_mode(!app.mouse_free());
            }
        }
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
            app.render_sets.lines("tactical_lines").dynamic.clear();
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
        app.render_sets.lines("tactical_lines").dynamic = self.tactical_map.lines(self.flight_manager.map(), &live, self.flight_manager.player_id(), plan_of, hovered.as_deref(), &view, sea_level);
        if let Some(path) = &picked_path {
            tactical_map::flight_plan(&mut app.render_sets.lines("tactical_lines").dynamic, &path.points, path.looped, None, sea_level, &view);
        }
        // The missiles in the air, in their shooter's colors.
        for missile in self.missiles.in_flight() {
            let rgb = match self.flight_manager.map().find(&missile.shooter) {
                Some((_, placement)) if placement.player => [1.0, 0.82, 0.30],
                Some((coalition, _)) => [coalition.color.0, coalition.color.1, coalition.color.2].map(|channel| channel as f32 / 255.0),
                None => [1.0, 1.0, 1.0],
            };
            tactical_map::missile_marker(&mut app.render_sets.lines("tactical_lines").dynamic, missile.position, missile.velocity, rgb, &view);
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
                    Some(now) if now.wrecked || now.pilot_dead => AircraftStatus::Destroyed,
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
                EditorRequest::SetProfile { aircraft, label, on } => {
                    if let Some(node) = scene.content.nodes.get_mut(&aircraft) {
                        node.push_physics_event(ProfileSwitch { label, on });
                    }
                }
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
        editor.profiles = self.flight_manager.map().aircraft()
            .filter_map(|(_, aircraft)| {
                let profiles = scene.content.nodes.get(&aircraft.name)?.physics_state::<AircraftState>()?.profiles.clone();
                Some((aircraft.name.clone(), profiles))
            })
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
        } else if live.pilot_dead {
            rows.push(warn("State", "pilot killed".to_owned()));
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
        if app.flight_editor.typing || app.hud_layout.typing || !Self::player_flight_input() || self.animations.playing_on(player_id) {
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
        // After Plane::update has read this frame's physics state.
        self.collect_pilot_kills(scene, app);
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
            app.render_sets.lines("editor_lines").dynamic.clear();
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
        app.render_sets.lines("editor_lines").dynamic = lines;
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
                // A broken surface's slab is gone, so a missile can't fuze on it.
                let gone = state.map(|state| state.profiles.iter().filter(|(_, hp)| *hp <= 0.0).map(|(label, _)| label.clone()).collect::<Vec<_>>()).unwrap_or_default();
                let colliders: Vec<FuzeCollider> = plane.map(|plane| {
                    plane.fuze_colliders().iter().filter(|collider| match collider {
                        ColliderType::ConvexHull { wing: Some(label), .. } => !gone.iter().any(|gone| gone == label),
                        _ => true,
                    }).filter_map(FuzeCollider::from_collider).collect()
                }).unwrap_or_default();
                let reach = fuze::reach_of(&colliders);
                Some(TargetTrack { name: placement.name.clone(), position: live.position, velocity, heat, decoy: false, rotation: live.rotation, colliders, reach })
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
        // With the HUD fixed to the jet, the HUD's own arrow (at its edge)
        // and the helmet display's (by its cross) point the way instead.
        if (app.tactical_map_active || app.editor_active) || self.hud_hidden_by_g || self.hud_on_plate(scene) {
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
            app.render_sets.lines("hud_screen_lines").dynamic.push(LineInstance { start: [start.x, start.y, 0.0], end: [end.x, end.y, 0.0], color: COLOR, width: WIDTH });
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
        app.render_sets.lines("heat_screen_lines").dynamic.clear();
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
            heat_blob(&mut app.render_sets.lines("heat_screen_lines").dynamic, (x, y), radius * 2.0, [color[0], color[1], color[2], 0.25 * glow + 0.05], to_pixels);
            heat_blob(&mut app.render_sets.lines("heat_screen_lines").dynamic, (x, y), radius, [color[0], color[1], color[2], 0.95], to_pixels);

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
        self.deliver_blasts(scene, &events.blasts);
    }

    /// Every blast onto every aircraft still flying. The physics half spends
    /// wing HP and notices a seat inside the sphere; a wing hit does not
    /// shot-down the jet.
    fn deliver_blasts(&mut self, scene: &mut Scene, blasts: &[BlastEvent]) {
        let names: Vec<String> = self.flight_manager.map().aircraft().map(|(_, placement)| placement.name.clone()).collect();
        for blast in blasts {
            for name in &names {
                let Some(node) = scene.content.nodes.get_mut(name) else { continue };
                if node.get_behavior::<Plane>().is_some_and(|plane| plane.wrecked) {
                    continue;
                }
                if let Some(plane) = node.get_behavior_mut::<Plane>() {
                    plane.note_blast(&blast.shooter, &blast.weapon);
                }
                node.push_physics_event(AircraftEvent::Blast { at: blast.at, radius: blast.radius, damage: blast.damage, impulse: blast.impulse });
            }
        }
    }

    /// Pilot kills and per-part blast damage, into the kill feed.
    fn collect_pilot_kills(&mut self, scene: &mut Scene, app: &mut App) {
        let names: Vec<String> = self.flight_manager.map().aircraft().map(|(_, placement)| placement.name.clone()).collect();
        const PLAIN: UiColor = UiColor::Rgb(230, 230, 230);
        for name in names {
            let Some(plane) = scene.content.nodes.get_mut(&name).and_then(|node| node.get_behavior_mut::<Plane>()) else { continue };
            let hits = plane.take_part_hits();
            let kill = plane.take_kill_report();
            for hit in hits {
                let damage = if hit.destroyed {
                    format!("{} -{:.0} destroyed", hit.part, hit.dealt)
                } else {
                    format!("{} -{:.0}", hit.part, hit.dealt)
                };
                println!("Hit - {name}'s {} took {:.0} from {}'s {}", hit.part, hit.dealt, hit.shooter, hit.weapon);
                self.push_kill_feed(app, [
                    (hit.shooter.clone(), self.team_color(&hit.shooter)),
                    ("damaged".to_owned(), PLAIN),
                    (name.clone(), self.team_color(&name)),
                    (damage, PLAIN),
                ]);
            }
            if let Some((shooter, weapon)) = kill {
                println!("Splash - {name} killed by {shooter}'s {weapon}");
                self.push_kill_feed(app, [
                    (shooter.clone(), self.team_color(&shooter)),
                    ("killed".to_owned(), PLAIN),
                    (name.clone(), self.team_color(&name)),
                    (format!("with {}", weapon_label(&weapon)), PLAIN),
                ]);
            }
        }
    }

    /// One line on the kill feed. Newest stays on top.
    fn push_kill_feed(&mut self, app: &mut App, texts: [(String, UiColor); play_ui::KILL_FEED_PIECES]) {
        let pieces = texts.map(|(text, color)| {
            let width = text_width(&mut app.ui.text.font_system, &text, play_ui::KILL_FEED_FONT_SIZE, HUD_FONT);
            (text, color, width)
        });
        self.kill_feed.push(KillFeedEntry { pieces, seconds_left: KILL_FEED_SECONDS });
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
            self.hud_symbol = None;
            self.vertical_scan = false;
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
        // The Fox-2 mode (a heat-seeker selected): the helmet display, while
        // it's up, points the seeker where the pilot looks - see draw_hmd.
        let fox2 = selected.as_ref().is_some_and(|(_, spec)| spec.category == WeaponCategory::Missile && spec.guidance.seeker == Seeker::InfraRed);
        self.fire_control.cue = if fox2 { self.hmd_cue.get() } else { None };
        // The lock key: lets the lock go, or locks the seeker on what's where
        // it rests (in the diamond - the nose, or the helmet's cross).
        // Held past VERTICAL_SCAN_HOLD_SECONDS instead, it's the vertical
        // lock, for as long as it stays held - letting go ends it, and the
        // key is back to its taps (a lock it caught stays). So a tap acts
        // when it's let go, and a hold isn't a tap too.
        const VERTICAL_SCAN_HOLD_SECONDS: f32 = 0.5;
        if has_control && input::is_action_pressed("toggle_lock") {
            self.lock_held += delta_time;
            if !self.lock_hold_used && self.lock_held >= VERTICAL_SCAN_HOLD_SECONDS && seeker.is_some() {
                self.lock_hold_used = true;
                self.fire_control.drop_locks();
                self.vertical_scan = true;
            }
        } else {
            if self.lock_held > 0.0 && !self.lock_hold_used {
                if self.fire_control.seeker_target.is_some() {
                    self.fire_control.drop_locks();
                } else if let Some(seeker) = &seeker {
                    self.fire_control.lock(&own, &contacts, seeker);
                }
            }
            self.vertical_scan = false;
            self.lock_held = 0.0;
            self.lock_hold_used = false;
        }
        // The vertical lock: the first enemy through the strip over the nose,
        // in range, is locked, and that's it done for this hold. Without a
        // seeker (another weapon, or none) there's nothing to lock with.
        match &seeker {
            Some(seeker) if self.vertical_scan => {
                if let Some(name) = fire_control::in_vertical_scan(&own, &contacts, seeker) {
                    self.fire_control.seeker_target = Some(name);
                    self.vertical_scan = false;
                }
            }
            _ => self.vertical_scan = false,
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

        // The trigger: hold it for the missile's `hold_to_launch`, then the
        // next one off - from the other side than the last, outboard first -
        // at whatever the seeker's locked on (or nothing: it flies on
        // straight). Releasing early cancels. One launch per press.
        let selected_id = selected.as_ref().map(|(weapon, _)| weapon.clone());
        let holding = has_control && player_alive && input::is_action_pressed("fire_weapon");
        if !holding || self.launch_hold_weapon.as_ref() != selected_id.as_ref() {
            self.launch_hold = 0.0;
            self.launch_spent = false;
            self.launch_hold_weapon = holding.then(|| selected_id.clone()).flatten();
        } else if !self.launch_spent {
            self.launch_hold += delta_time;
        }
        let hold_for = selected.as_ref().map(|(_, spec)| spec.hold_to_launch).unwrap_or(0.0);
        if holding && !self.launch_spent && self.launch_hold >= hold_for {
            self.launch_spent = true;
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
                            seeker.as_ref().and_then(|seeker| fire_control::in_seeker_view(&own, &contacts, seeker, self.fire_control.rest_direction(&own, seeker)))
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
        // diamond out ahead where it looks - the nose. Drawn by update_hud,
        // once the camera has moved.
        self.hud_symbol = None;
        if seeker.is_none() || self.hud_hidden_by_g {
            return;
        }
        let up = own.rotation * Vector3::y();
        let target = self.fire_control.seeker_target.as_ref()
            .and_then(|name| contacts.iter().find(|contact| contact.name == *name));
        match target {
            Some(target) => {
                let flash_off = shoot && (self.game_time * 4.0).fract() > 0.5;
                if !flash_off {
                    self.hud_symbol = Some(HudSymbol { point: target.position, track: Some(target.name.clone()), up, half_size: 18.0, diamond: false });
                }
            }
            None => {
                // The HUD's diamond is the boresight - fixed on the nose. While
                // the helmet display is up in the Fox-2 mode, the seeker looks
                // where the pilot does instead, shown on the helmet (see
                // draw_hmd).
                let nose_point = own.position + own.boresight() * 3000.0;
                self.hud_symbol = Some(HudSymbol { point: nose_point, track: None, up, half_size: 11.0, diamond: true });
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
        // The ladder's and the heading tape's numbers (px).
        const SMALL_LABEL_FONT: f32 = 10.0;
        // The velocity marker's "o" on the plate (px) - the screen one's.
        const VELOCITY_MARKER_FONT: f32 = 14.0;

        app.render_sets.lines("hud_horizon_lines").dynamic.clear();
        app.render_sets.lines("hud_screen_lines").dynamic.clear();
        app.render_sets.lines("hud_lines").dynamic.clear();
        app.render_sets.text("hud_text").clear();
        app.render_sets.text("hud_text").mask = None;
        app.render_sets.text("hmd_text").clear();
        self.hmd_cue.set(None);

        let (screen_width, screen_height) = (app.window_manager.size.width as f32, app.window_manager.size.height as f32);
        // The HUD's size (fractions of the screen), its center, and its scale,
        // from F11's layout panel. Scale shrinks the frame and every mark
        // around that center, letters included.
        let layout = app.hud_layout;
        let scale = layout.draw_scale();
        let (base_width, base_height) = (screen_width * layout.width, screen_height * layout.height);
        let (center_x, center_y) = (screen_width * 0.5 + layout.center_x, screen_height * 0.5 + layout.center_y);
        let (frame_width, frame_height) = (base_width * scale, base_height * scale);
        let (frame_x, frame_y) = (center_x - frame_width * 0.5, center_y - frame_height * 0.5);
        let (readout_width, readout_height) = (play_ui::HUD_READOUT_SIZE.0 * scale, play_ui::HUD_READOUT_SIZE.1 * scale);
        let readout_gap = READOUT_GAP * scale;
        let end_in = END_IN * scale;
        let line_w = WIDTH * scale;
        let dash = DASH * scale;
        let dash_gap = DASH_GAP * scale;
        let label_offset = LABEL_OFFSET * scale;
        let tick_len = TICK * scale;
        let arrow = ARROW * scale;
        let heading_gap = HEADING_GAP * scale;
        let heading_ticks_up = HEADING_TICKS_UP * scale;
        let heading_major = HEADING_MAJOR_TICK * scale;
        let heading_minor = HEADING_MINOR_TICK * scale;
        let heading_box = (HEADING_BOX.0 * scale, HEADING_BOX.1 * scale);
        let small_label = (play_ui::HUD_SMALL_LABEL_SIZE.0 * scale, play_ui::HUD_SMALL_LABEL_SIZE.1 * scale);
        let left_line = frame_x + readout_width + readout_gap;
        let right_line = frame_x + frame_width - readout_width - readout_gap;
        let line_top = frame_y;
        let line_bottom = frame_y + frame_height;

        let game_ui_shown = Ui::get_ui_node(&mut app.ui.renderizable_elements, "game_ui").is_some_and(|node| node.is_active);
        // Only from the cockpit of the player's own jet - it's the pilot's
        // HUD, not the camera's.
        let in_cockpit = self.in_own_cockpit(scene);
        let shown = game_ui_shown && !self.hud_hidden_by_g && in_cockpit;
        let hud_fixed = self.hud_on_plate(scene);
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
        let tape_bottom = line_top - heading_gap;
        let ticks_bottom = tape_bottom - heading_ticks_up;
        let label_y = ticks_bottom - heading_major - 2.0 * scale - small_label.1 * 0.5;
        let heading_y = label_y - 4.0 * scale;
        let g_y = line_bottom - readout_height * 0.5;
        let weapon_y = line_bottom;
        // Radar altitude sits just outside the right line's bottom corner.
        // The fuel bar is a gauge just outside the left line's.
        // Center of the square's bottom half.
        let warning_y = (line_top + line_bottom * 3.0) * 0.25;
        let corner_width = play_ui::HUD_RADAR_ALTITUDE_WIDTH * scale;
        let corner_x = right_line + 4.0 * scale + corner_width * 0.5;
        // Its bottom edge is the line's bottom.
        let radar_y = line_bottom - readout_height * 0.5;
        let cms_y = line_bottom - 40.0 * scale;
        let player_id = self.flight_manager.player_id();
        let (fuel_kg, fuel_capacity, bingo_kg, radar_warn_m) = scene.content.nodes.get(player_id)
            .and_then(|node| node.get_behavior::<Plane>())
            .map(|plane| { let (kg, capacity) = plane.fuel(); (kg, capacity, plane.bingo_kg, plane.radar_warn_m) })
            .unwrap_or((0.0, 0.0, None, super::map::DEFAULT_RADAR_WARN_M));
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
        let (warn_width, warn_height) = (play_ui::HUD_WARN_SIZE.0 * scale, play_ui::HUD_WARN_SIZE.1 * scale);
        // Fuel bar, just outside the left line, bottom-aligned with it.
        // Bottom is empty, top is the tanks' maximum. The percent sits
        // against the bar's left side. The red mark is bingo, the same
        // width as the end caps.
        let fuel_bar_width = 9.0 * scale;
        let fuel_bar_height = 60.0 * scale;
        let fuel_percent_width = play_ui::HUD_FUEL_PERCENT_WIDTH * scale;
        let fuel_bar_bottom = line_bottom;
        let fuel_bar_top = fuel_bar_bottom - fuel_bar_height;
        let fuel_bar_right = left_line - 6.0 * scale;
        let fuel_bar_left = fuel_bar_right - fuel_bar_width;
        let fuel_fraction = if fuel_capacity > 0.0 { (fuel_kg / fuel_capacity).clamp(0.0, 1.0) } else { 0.0 };
        let fuel_level_y = fuel_bar_bottom - fuel_fraction * fuel_bar_height;
        let fuel_percent_x = fuel_bar_left - 2.0 * scale - fuel_percent_width * 0.5;
        let fuel_percent_y = (fuel_bar_top + fuel_bar_bottom) * 0.5;
        let bingo_fraction = bingo_kg.filter(|kg| *kg > 0.0 && fuel_capacity > 0.0).map(|kg| (kg / fuel_capacity).clamp(0.0, 1.0));
        let fixed_map = if hud_fixed {
            fixed_hud_plane(scene, player_id, screen_width, screen_height, left_line, right_line, line_top, line_bottom, &layout)
        } else {
            None
        };
        // How big the plate is on screen - its line widths follow.
        let plate_print = fixed_map.as_ref().map_or(1.0, |map| plate_print_scale(scene.cameras.active(), map, screen_width, screen_height));
        // Everything on the plate only shows through the jet's HUD glass -
        // its outline on screen, in real pixels. The plate is laid out as
        // ever; the glass just cuts it, so moving the head moves the window,
        // not the symbols. With the glass out of view nothing shows; a jet
        // without a glass outline isn't cut.
        let glass = self.flight_manager.hud_glass(player_id).filter(|glass| !glass.is_empty());
        let to_pixels = app.window_manager.pixel_size.height as f32 / screen_height.max(1.0);
        // World points on screen, in real pixels - None if any is behind the
        // camera.
        let project_all = |points: &mut dyn Iterator<Item = Vector3<f32>>| -> Option<Vec<[f32; 2]>> {
            points
                .map(|point| project_hud_point(scene.cameras.active(), point, screen_width, screen_height).map(|(x, y)| [x * to_pixels, y * to_pixels]))
                .collect()
        };
        // The glass on screen, in real pixels - None while any of its corners
        // is behind the camera (then it's out of view).
        let glass_on_screen = fixed_map.as_ref().zip(glass).map(|(map, glass)| project_all(&mut glass.iter().map(|point| map.on_glass(*point))));
        let plate_mask = glass_on_screen.as_ref().map(|points| {
            points.as_ref().and_then(|points| convex_mask(points, 0.0)).unwrap_or_else(|| vec![[0.0, 0.0, -1.0]])
        });
        app.render_sets.lines("hud_lines").mask = plate_mask.clone();
        app.render_sets.text("hud_text").mask = plate_mask.clone();
        // The seeker's symbol - from any view, on the plate while it's up.
        if let Some(symbol) = self.hud_symbol.clone() {
            Self::draw_hud_symbol(scene, app, symbol, fixed_map.as_ref().map(|map| (map, plate_print)));
        }
        // The vertical lock's line: dashed, straight up through the nose,
        // laid out far ahead so it sits on the sky like the HUD's other marks.
        // On the plate it runs the whole height of the glass (the glass cuts
        // it); on the screen, just the strip it locks in (see
        // fire_control::VERTICAL_SCAN_*). The strip is the same either way.
        if let (true, Some(instance)) = (self.vertical_scan, scene.content.renderizable_instances.get(player_id)) {
            const DASH_DEG: f32 = 1.5;
            const DASH_GAP_DEG: f32 = 1.0;
            const DISTANCE: f32 = 3000.0;
            let (bottom, top) = if plate_mask.is_some() {
                (-80.0, 80.0)
            } else {
                (-fire_control::VERTICAL_SCAN_DOWN_DEG, fire_control::VERTICAL_SCAN_UP_DEG)
            };
            let (position, rotation) = (instance.instance.transform.position, instance.instance.transform.rotation);
            let at = |degrees: f32| position + rotation * Vector3::new(0.0, degrees.to_radians().sin(), degrees.to_radians().cos()) * DISTANCE;
            let mut from = bottom;
            while from < top {
                let to = (from + DASH_DEG).min(top);
                app.render_sets.lines("hud_lines").dynamic.push(LineInstance { start: at(from).into(), end: at(to).into(), color: COLOR, width: 1.5 * scale * plate_print });
                from += DASH_DEG + DASH_GAP_DEG;
            }
        }
        // The velocity marker: on the plate, where the jet's path crosses it
        // seen from the eye (the screen one is hidden meanwhile - see
        // ui_control); held at the HUD square's edge once the path is past
        // it, on the side it went off. The path is the physics' velocity in
        // the jet's frame, turned by the jet as it's drawn this frame - the
        // raw velocity is up to two physics steps old while the drawn jet
        // (and the plate on it) is moved on to now, so it would jitter
        // against the plate at the physics' rate.
        let marker_shown = Ui::get_ui_node(&mut app.ui.renderizable_elements, "velocity_marker").is_some_and(|node| node.is_active);
        if let (Some(map), Some(body_velocity), true) = (&fixed_map, self.player_body_velocity, shown && marker_shown) {
            if let Some(direction) = (map.aircraft_rot * body_velocity).try_normalize(0.1) {
                let at = map.ray_held_in_frame(direction, VELOCITY_MARKER_FONT * 0.5 * scale * map.meters_per_pixel());
                let color = UiColor::Rgb(0, 255, 77).to_rgba_f32();
                app.render_sets.text("hud_text").push(&mut app.ui.text.font_system, "o", HUD_FONT, map.text_frame(at, VELOCITY_MARKER_FONT * scale), color);
            }
        }
        // The helmet display, looking away from the HUD.
        if let (Some(map), true) = (&fixed_map, shown && self.hmd_enabled) {
            self.draw_hmd(scene, app, map, screen_width, screen_height);
        }
        // Same angle for every readout: the plate's, about each label's own
        // center. None leaves the letters upright with the head.
        let placed = [
            ("hud_speed", speed_x, middle_y, shown, readout_width, readout_height),
            ("hud_altitude", altitude_x, middle_y, shown, readout_width, readout_height),
            ("hud_heading", middle_x, heading_y, shown, readout_width, readout_height),
            ("hud_g", speed_x, g_y, shown && fox2, readout_width, readout_height),
            ("hud_weapon", middle_x, weapon_y, shown && self.selected_weapon.is_some(), 160.0 * scale, readout_height),
            ("hud_altitude_warning", middle_x, warning_y, warning_shown, warn_width, warn_height),
            ("hud_corner_fuel", fuel_percent_x, fuel_percent_y, shown && fuel_capacity > 0.0, fuel_percent_width, readout_height),
            ("hud_radar_altitude", corner_x, radar_y, shown, corner_width, readout_height),
            ("hud_cms_cue", middle_x, cms_y, shown && cms_cue.as_ref().is_some_and(|(_, on, _)| *on), 160.0 * scale, readout_height),
        ];
        for (key, x, y, visible, width, height) in placed {
            let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, key) else { continue };
            // On the plate the readout is drawn on it (below), not as the
            // screen label.
            node.set_active(visible && fixed_map.is_none());
            node.style.font_size = Some(hud_readout_font(key) * scale);
            if key == "hud_radar_altitude" {
                node.style.text_color = Some(if radar_red { UiColor::Rgb(255, 64, 64) } else { UiColor::Rgb(0, 255, 77) });
                if let Some(label) = node.as_label_mut() {
                    label.set_text(&mut app.ui.text.font_system, &format!("R: {radar_altitude:.0}"), true);
                }
            }
            if key == "hud_corner_fuel" {
                node.style.text_color = Some(if fuel_red { UiColor::Rgb(255, 64, 64) } else { UiColor::Rgb(0, 255, 77) });
                if let Some(label) = node.as_label_mut() {
                    label.set_text(&mut app.ui.text.font_system, &format!("{:.0}%", fuel_fraction * 100.0), true);
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
            // On the plate: lying on it at the readout's own point, so it
            // turns, grows and shrinks with the plate.
            if let (Some(map), true) = (&fixed_map, visible) {
                let color = node.style.text_color.unwrap_or(UiColor::Rgb(0, 255, 77)).to_rgba_f32();
                let font_size = hud_readout_font(key) * scale;
                if let Some(label) = node.as_label_mut() {
                    app.render_sets.text("hud_text").push(&mut app.ui.text.font_system, label.text(), label.font(), map.text_frame(map.at(x, y), font_size), color);
                }
            }
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
        let line_color = std::cell::Cell::new(COLOR);
        // F6 draws the center on the combiner glass. The video toggle instead
        // puts the whole HUD on a plate fixed to the jet, camera pose aside.
        let collimated = if let Some(map) = fixed_map {
            Some(CollimatedHud { map, visible: true, gpu_mask: plate_mask.clone(), print_scale: plate_print })
        } else { self.collimated_hud.then(|| {
            let instance = scene.content.renderizable_instances.get(player_id)?;
            let glass = self.flight_manager.hud_glass(player_id)?;
            let aircraft_pos = instance.instance.transform.position;
            let aircraft_rot = instance.instance.transform.rotation;
            let eye = instance.instance.metadata.cameras.as_ref()
                .and_then(|cameras| cameras.get(COCKPIT_CAMERA))
                .map(|camera| camera.position)
                .unwrap_or_else(|| aircraft_rot.inverse() * (scene.cameras.active().camera.position().coords - aircraft_pos));
            if glass.len() < 3 {
                return None;
            }
            let center = glass.iter().copied().sum::<Vector3<f32>>() / glass.len() as f32;
            let boresight = (center - eye).try_normalize(1e-4)?;
            let up = (Vector3::y() - boresight * Vector3::y().dot(&boresight)).try_normalize(1e-4)?;
            let right = boresight.cross(&up);
            let (mut r_min, mut r_max) = (f32::MAX, f32::MIN);
            let (mut u_min, mut u_max) = (f32::MAX, f32::MIN);
            for point in glass {
                let along_right = (point - center).dot(&right);
                let along_up = (point - center).dot(&up);
                r_min = r_min.min(along_right);
                r_max = r_max.max(along_right);
                u_min = u_min.min(along_up);
                u_max = u_max.max(along_up);
            }
            if r_max - r_min < 1e-3 || u_max - u_min < 1e-3 {
                return None;
            }
            // Larger than the pane, so the glass edge — not an inner margin —
            // is what cuts the symbols off.
            let grow = |min: f32, max: f32| {
                let mid = (min + max) * 0.5;
                let half = (max - min) * 0.5 * layout.glass_fill;
                (mid - half, mid + half)
            };
            let (r_min, r_max) = grow(r_min, r_max);
            let (u_min, u_max) = grow(u_min, u_max);
            let top = label_y - small_label.1 * 0.5;
            let map = GlassMap {
                aircraft_pos,
                aircraft_rot,
                eye,
                center,
                boresight,
                right,
                up,
                r_min,
                r_max,
                u_min,
                u_max,
                left: left_line,
                right_px: right_line,
                top,
                bottom: line_bottom,
            };
            let active = scene.cameras.active();
            let mut ui_points = Vec::new();
            let mut gpu_points = Vec::new();
            for point in glass {
                let world = aircraft_pos + aircraft_rot * point;
                let Some((x, y)) = project_hud_point(active, world, screen_width, screen_height) else { continue };
                ui_points.push([x, y]);
                gpu_points.push([x * to_pixels, y * to_pixels]);
            }
            let gpu_mask = convex_mask(&gpu_points, 0.0);
            let visible = gpu_mask.is_some();
            // The glass's height on screen against the layout it was drawn
            // for. Zoom changes both the pane and this, so the line widths
            // stay the same on the glass. Clamped: looking away stretches
            // the projection.
            let print_scale = if ui_points.len() >= 2 {
                let (mut min_y, mut max_y) = (f32::MAX, f32::MIN);
                for point in &ui_points {
                    min_y = min_y.min(point[1]);
                    max_y = max_y.max(point[1]);
                }
                let glass_span = (max_y - min_y).max(1.0);
                let layout_span = (line_bottom - top).max(1.0);
                (glass_span / layout_span).clamp(0.25, 4.0)
            } else {
                1.0
            };
            Some(CollimatedHud { map, visible, gpu_mask, print_scale })
        }).flatten() };
        if let Some(hud) = &collimated {
            app.render_sets.lines("hud_horizon_lines").mask = hud.gpu_mask.clone();
            app.render_sets.text("hud_text").mask = hud.gpu_mask.clone();
        }
        let collimating = collimated.is_some();
        let map_for_lines = collimated.as_ref().filter(|hud| hud.visible).map(|hud| hud.map);
        let print_scale = collimated.as_ref().map(|hud| hud.print_scale).unwrap_or(1.0);
        // `on_glass`: this segment is part of the center. With F6 on it is
        // drawn on the combiner glass (and left out when the glass is behind
        // the camera) instead of stuck to the screen. With the plate on, every
        // mark — readouts included — is drawn on that plate.
        let mut screen_line = |start: (f32, f32), end: (f32, f32), width: f32, on_glass: bool| {
            if collimating && (on_glass || hud_fixed) {
                if let Some(map) = map_for_lines {
                    let start = map.at(start.0, start.1);
                    let end = map.at(end.0, end.1);
                    app.render_sets.lines("hud_horizon_lines").dynamic.push(LineInstance { start: start.into(), end: end.into(), color: line_color.get(), width: width * print_scale });
                }
                return;
            }
            app.render_sets.lines("hud_screen_lines").dynamic.push(LineInstance {
                start: [snap(start.0), snap(start.1), 0.0],
                end: [snap(end.0), snap(end.1), 0.0],
                color: line_color.get(),
                width,
            });
        };
        // The shader runs each line half its width past its ends, so the
        // ends meet the side line in a square corner.
        for (line, inward) in [(left_line, 1.0), (right_line, -1.0)] {
            screen_line((line, line_top), (line, line_bottom), line_w, true);
            for y in [line_top, line_bottom] {
                screen_line((line, y), (line + inward * end_in, y), line_w, true);
            }
        }
        // The readouts' boxes. The radar altitude under the altitude number
        // has none, and neither does WARN.
        {
            let mut readout_box = |x: f32, y: f32, width: f32| {
                let (left, right) = (x - width * 0.5, x + width * 0.5);
                let (top, bottom) = (y - readout_height * 0.5, y + readout_height * 0.5);
                screen_line((left, top), (right, top), line_w, false);
                screen_line((right, top), (right, bottom), line_w, false);
                screen_line((right, bottom), (left, bottom), line_w, false);
                screen_line((left, bottom), (left, top), line_w, false);
            };
            readout_box(speed_x, middle_y, readout_width);
            readout_box(altitude_x, middle_y, readout_width);
        }
        let alert = [1.0, 0.25, 0.25, 0.9];
        line_color.set(COLOR);
        // Fuel bar. The top and bottom caps, and the bingo mark, share one
        // horizontal span and one stroke so their ends line up.
        if fuel_capacity > 0.0 {
            let cap = 2.0 * scale;
            let mark_left = fuel_bar_left - cap;
            let mark_right = fuel_bar_right + cap;
            screen_line((fuel_bar_left, fuel_bar_top), (fuel_bar_left, fuel_bar_bottom), line_w, false);
            screen_line((fuel_bar_right, fuel_bar_top), (fuel_bar_right, fuel_bar_bottom), line_w, false);
            screen_line((mark_left, fuel_bar_top), (mark_right, fuel_bar_top), line_w, false);
            screen_line((mark_left, fuel_bar_bottom), (mark_right, fuel_bar_bottom), line_w, false);
            // The shader runs a line half its width past each end, so the
            // fill is shortened to stay inside the caps.
            let fill_width = fuel_bar_width - 3.0 * scale;
            let half = fill_width * 0.5;
            let y_bottom = fuel_bar_bottom - half;
            let y_top = fuel_level_y + half;
            if y_bottom - y_top > 0.5 {
                line_color.set(if fuel_red { alert } else { COLOR });
                let center_x = (fuel_bar_left + fuel_bar_right) * 0.5;
                screen_line((center_x, y_bottom), (center_x, y_top), fill_width, false);
            }
            if let Some(bingo_fraction) = bingo_fraction {
                let bingo_y = fuel_bar_bottom - bingo_fraction * fuel_bar_height;
                line_color.set([1.0, 0.25, 0.25, 1.0]);
                screen_line((mark_left, bingo_y), (mark_right, bingo_y), line_w, false);
            }
            line_color.set(COLOR);
        }
        if let Some(cue) = &self.range_cue {
            // Bottom of the line is 0 m, the top the seeker's range.
            let height_of = |range: f32| line_bottom - (range / cue.scale_top).clamp(0.0, 1.0) * (line_bottom - line_top);
            for range in [cue.min_range, cue.max_range] {
                let y = height_of(range);
                screen_line((right_line - tick_len, y), (right_line, y), line_w, true);
            }
            // The arrow, pointing at the line from inside - closed into a
            // triangle while the target's in range.
            let y = height_of(cue.range);
            let tip = (right_line - 2.0 * scale, y);
            let (upper, lower) = ((tip.0 - arrow * 1.5, y - arrow), (tip.0 - arrow * 1.5, y + arrow));
            screen_line(upper, tip, line_w, true);
            screen_line(lower, tip, line_w, true);
            if cue.in_range {
                screen_line(upper, lower, line_w, true);
            }
        }

        let active = scene.cameras.active();
        let look = active.camera.rotation_modifier * active.camera.calc_forward_direction();
        // The tape and the ladder follow the camera, unless F6 has them
        // fixed to the jet's nose. Straight up or down, no heading.
        let camera_toward = Vector3::new(look.x, 0.0, look.z).try_normalize(1e-3);
        let jet_toward = scene.content.renderizable_instances.get(player_id).and_then(|instance| {
            let forward = instance.instance.transform.rotation * Vector3::z();
            Vector3::new(forward.x, 0.0, forward.z).try_normalize(1e-3)
        });
        // The tape and the ladder follow the head, unless the HUD is pinned
        // to the jet (or F6 has already put them on the glass).
        let toward = if collimated.is_some() || hud_fixed { jet_toward } else { camera_toward };
        let Some(toward) = toward else { return };
        // The space between the side lines in fifths: the horizon's two
        // halves take the 2nd and 4th, the gap between them the 3rd. The
        // other lines are half as long, around the same gap. On the glass
        // that inset is empty pane, so the marks run out to the frame.
        let fifth = (right_line - left_line) / 5.0;
        let gap = fifth * 0.5;
        let half_span = (right_line - left_line) * 0.5;
        let (horizon_half_length, ladder_half_length) = if collimated.is_some() {
            (half_span - end_in, half_span * 0.78)
        } else {
            (gap + fifth, gap + fifth * 0.5)
        };

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
            screen_line((x, ticks_bottom), (x, ticks_bottom - if major { heading_major } else { heading_minor }), line_w, true);
            // Its number's box clear of the heading's.
            let clear = (x - middle_x).abs() >= (heading_box.0 + small_label.0) * 0.5;
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
        let (left, right) = (middle_x - heading_box.0 * 0.5, middle_x + heading_box.0 * 0.5);
        let (top, bottom) = (heading_y - heading_box.1 * 0.5, heading_y + heading_box.1 * 0.5);
        screen_line((left, top), (right, top), line_w, true);
        screen_line((right, top), (right, bottom), line_w, true);
        screen_line((right, bottom), (left, bottom), line_w, true);
        screen_line((left, bottom), (left, top), line_w, true);

        // The heading number rides the glass with the tape, cut off by the
        // glass's edge (`hud_text.mask`). On the plate it was already drawn
        // with the other readouts.
        if !hud_fixed {
        if let Some(hud) = &collimated {
            if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "hud_heading") {
                node.set_active(false);
                let color = node.style.text_color.unwrap_or(UiColor::Rgb(0, 255, 77)).to_rgba_f32();
                if let (Some(label), true) = (node.as_label_mut(), hud.visible) {
                    let frame = hud.map.text_frame(hud.map.at(middle_x, heading_y), hud_readout_font("hud_heading") * scale);
                    app.render_sets.text("hud_text").push(&mut app.ui.text.font_system, label.text(), label.font(), frame, color);
                }
            }
        } else if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "hud_heading") {
            node.style.font_size = Some(16.0 * scale);
        }
        }

        // The horizon and the ladder. On the screen they follow the camera
        // and stop at the side lines. With F6 they are the same angles,
        // carried onto the combiner glass, and the glass is what cuts them off.
        let ladder_box = [(left_line, line_top), (right_line, line_top), (right_line, line_bottom), (left_line, line_bottom)]
            .map(|(x, y)| [x * to_pixels, y * to_pixels]);
        if collimated.is_none() {
            app.render_sets.lines("hud_horizon_lines").mask = convex_mask(&ladder_box, 2.0 * to_pixels);
        }
        let across = Vector3::y().cross(&toward);
        let eye = collimated.as_ref().map(|hud| hud.map.eye_world()).unwrap_or_else(|| active.camera.position().coords);
        let meters_per_pixel = collimated.as_ref()
            .map(|hud| hud.map.meters_per_pixel())
            .unwrap_or_else(|| 2.0 * DISTANCE * (active.projection.fovy.to_radians() * 0.5).tan() / screen_height.max(1.0));
        // The numbers: on the screen (x, y), or with F6 or the plate, on the
        // glass in the world.
        let mut labels = Vec::new();
        let mut glass_labels: Vec<(Vector3<f32>, String)> = Vec::new();
        // None of it in the Fox-2 mode, nor while the glass is behind the camera.
        let glass_hidden = collimated.as_ref().is_some_and(|hud| !hud.visible);
        let steps = if fox2 || glass_hidden { 1..=0 } else { (-90 / LADDER_STEP_DEG)..=(90 / LADDER_STEP_DEG) };
        for step in steps {
            let degrees = step * LADDER_STEP_DEG;
            let pitch = (degrees as f32).to_radians();
            let half_length = if step == 0 { horizon_half_length } else { ladder_half_length };
            let dir = toward * pitch.cos() + Vector3::y() * pitch.sin();
            let center = if let Some(hud) = &collimated {
                let Some(hit) = hud.map.ray_to_glass(dir) else { continue };
                hit
            } else {
                eye + dir * DISTANCE
            };
            // In front of the camera. The mask cuts off whatever misses the
            // box - or, with F6, the glass. Off-screen still counts then, so
            // a line can enter the glass at the edge of the view.
            let ahead = (center - active.camera.position().coords).dot(&look) > 0.0;
            let in_view = collimated.is_some() || app.camera_resources.world_to_screen(active, Point3::from(center), screen_width as u32, screen_height as u32).is_some();
            if !ahead || !in_view {
                continue;
            }
            // Solid, or below the horizon in dashes (px from the middle).
            let pieces: Vec<(f32, f32)> = if step < 0 {
                std::iter::successors(Some(gap), |from| Some(from + dash + dash_gap))
                    .take_while(|from| *from < half_length)
                    .map(|from| (from, (from + dash).min(half_length)))
                    .collect()
            } else {
                vec![(gap, half_length)]
            };
            for side in [-1.0, 1.0] {
                for (from, to) in &pieces {
                    let mut start = center + across * (side * from * meters_per_pixel);
                    let mut end = center + across * (side * to * meters_per_pixel);
                    if let Some(hud) = &collimated {
                        start = hud.map.cling(start);
                        end = hud.map.cling(end);
                    }
                    // On the plate, stop the pitch line at the HUD square.
                    // The screen HUD does this with a mask; the plate's other
                    // marks share that line list, so the square can't be a mask.
                    if hud_fixed {
                        let Some((clipped_start, clipped_end)) = collimated.as_ref().and_then(|hud| hud.map.clip_to_frame(start, end)) else { continue };
                        start = clipped_start;
                        end = clipped_end;
                    }
                    let width = if collimated.is_some() { line_w * print_scale } else { line_w };
                    app.render_sets.lines("hud_horizon_lines").dynamic.push(LineInstance { start: start.into(), end: end.into(), color: COLOR, width });
                }
                // Its number past each end - not the horizon's.
                if step != 0 {
                    let mut at = center + across * (side * (half_length + label_offset) * meters_per_pixel);
                    if let Some(hud) = &collimated {
                        at = hud.map.cling(at);
                    }
                    if hud_fixed && collimated.as_ref().is_some_and(|hud| {
                        let margin = small_label.0.max(small_label.1) * 0.5 * meters_per_pixel;
                        !hud.map.inside_frame(at, margin)
                    }) {
                        continue;
                    }
                    if collimated.is_some() {
                        glass_labels.push((at, degrees.to_string()));
                    } else if let Some(point) = app.camera_resources.world_to_screen(active, Point3::from(at), screen_width as u32, screen_height as u32) {
                        labels.push((point.x as f32, point.y as f32, degrees.to_string()));
                    }
                }
            }
        }

        // On the glass (F6) or the plate: the ladder's numbers and the heading
        // tape's lie on it, cut off by the glass's edge (`hud_text.mask`).
        let (label_width, label_height) = small_label;
        if let Some(hud) = &collimated {
            if hud.visible {
                glass_labels.extend(heading_labels.into_iter().map(|(x, y, text)| (hud.map.at(x, y), text)));
            }
            let color = UiColor::Rgb(0, 255, 77).to_rgba_f32();
            for (at, text) in glass_labels {
                app.render_sets.text("hud_text").push(&mut app.ui.text.font_system, &text, HUD_FONT, hud.map.text_frame(at, SMALL_LABEL_FONT * scale), color);
            }
            return;
        }

        // On the screen: the ladder's numbers, only where they're wholly
        // inside its box, and the heading tape's.
        let inside_box = |x: f32, y: f32| (left_line + label_width * 0.5..=right_line - label_width * 0.5).contains(&x)
            && (line_top + label_height * 0.5..=line_bottom - label_height * 0.5).contains(&y);
        let pitch_placed = labels.into_iter().filter(|(x, y, _)| inside_box(*x, *y)).take(play_ui::HUD_PITCH_LABELS)
            .enumerate().map(|(index, label)| (play_ui::hud_pitch_label_key(index), label));
        let heading_placed = heading_labels.into_iter().take(play_ui::HUD_HEADING_LABELS)
            .enumerate().map(|(index, label)| (play_ui::hud_heading_label_key(index), label));
        for (key, (x, y, text)) in pitch_placed.chain(heading_placed) {
            let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &key) else { continue };
            node.set_active(true);
            node.style.font_size = Some(SMALL_LABEL_FONT * hud_font_scale(scale, 1.0));
            node.transform.x = x - label_width / 2.0;
            node.transform.y = y - label_height / 2.0;
            node.transform.width = label_width;
            node.transform.height = label_height;
            node.transform.rect.left = node.transform.x;
            node.transform.rect.top = node.transform.y;
            node.transform.rect.right = node.transform.x + label_width;
            node.transform.rect.bottom = node.transform.y + label_height;
            if let Some(label) = node.as_label_mut() {
                label.set_text(&mut app.ui.text.font_system, &text, true);
            }
        }
    }

    /// The camera is in the player's own cockpit - the HUD is the pilot's,
    /// not the camera's.
    fn in_own_cockpit(&self, scene: &Scene) -> bool {
        self.camera_focus == self.flight_manager.player_id()
            && scene.content.nodes.get("camera").and_then(|node| node.get_behavior::<Camera>()).is_some_and(Camera::is_cockpit)
    }

    /// The HUD is a plate in the jet's frame ("HUD stays with the jet", from
    /// the cockpit), so the camera can roll and move around it - not glued
    /// to the screen.
    fn hud_on_plate(&self, scene: &Scene) -> bool {
        GAME_SETTINGS.lock().unwrap().hud_stays_with_jet && self.in_own_cockpit(scene)
    }

    /// An arrow just inside the HUD square's edge on the plate (`map`),
    /// pointing the way `direction` (world, from the eye) goes off it - for
    /// a lock outside the HUD. Sized in HUD pixels (`scale` - the layout's)
    /// with the plate, its lines `print_scale` wide like the plate's.
    fn draw_plate_arrow(app: &mut App, map: &GlassMap, direction: Vector3<f32>, scale: f32, print_scale: f32, color: [f32; 4]) {
        // Its length, half its width at the back, and its lines (HUD px).
        const LENGTH: f32 = 16.0;
        const HALF_WIDTH: f32 = 8.0;
        const LINE_WIDTH: f32 = 1.5;
        let meters_per_pixel = map.meters_per_pixel();
        let (length, half_width) = (LENGTH * scale * meters_per_pixel, HALF_WIDTH * scale * meters_per_pixel);
        let (x, y, way) = map.held_in_frame(direction, length);
        let across = nalgebra::Vector2::new(-way.y, way.x);
        let middle = nalgebra::Vector2::new(x, y);
        let tip = middle + way * (length * 0.5);
        let base = middle - way * (length * 0.5);
        let corners = [tip, base + across * half_width, base - across * half_width].map(|corner| map.from_plane(corner.x, corner.y));
        for index in 0..3 {
            let (start, end) = (corners[index], corners[(index + 1) % 3]);
            app.render_sets.lines("hud_lines").dynamic.push(LineInstance { start: start.into(), end: end.into(), color, width: LINE_WIDTH * scale * print_scale });
        }
    }

    /// The helmet display (the F-16's JHMCS), for when the pilot looks away
    /// from the HUD: fixed to the screen - a small cross in its middle, the
    /// bearing in a box over it, the speed in a box left of it and the
    /// altitude in a box right of it, a box on what's locked - or, while
    /// it's out of view, an arrow out from the cross pointing the way to it.
    /// All of it or none: it's up while the pilot looks outside - blanked
    /// while the middle of the view is on the HUD's glass (the HUD's there;
    /// on a jet without a glass outline, the HUD's square on `map` - the
    /// plate), and while it's below the cockpit's sill (the jet's data.ron
    /// `cockpit_sill`: the panel, the floor, the belly and wings, the spine
    /// behind the seat). It fades over HMD_FADE_DEG at both edges. Where the
    /// head can turn isn't touched - only what shows.
    ///
    /// The bearing is where the head points, not the jet.
    ///
    /// In the Fox-2 mode (a heat-seeker selected), while it's up, the helmet
    /// is the seeker's boresight: it looks where the pilot looks (see
    /// FireControl::cue), shown by a small diamond in the middle of the view
    /// inside a circle the size of its field of view - what's in the circle
    /// is what locks. Past the missile's gimbal limit that's crossed out
    /// instead - it can't look there, and rests back on the nose. With the
    /// display down the seeker is on the nose, the HUD's diamond. Screen
    /// size in UI points.
    fn draw_hmd(&self, scene: &Scene, app: &mut App, map: &GlassMap, screen_width: f32, screen_height: f32) {
        const COLOR: [f32; 3] = [0.0, 1.0, 0.3];
        // How far past the glass's edge or the sill it fades in over (degrees).
        const HMD_FADE_DEG: f32 = 4.0;
        // Sizes in UI points: the cross's arms; the lines; the readouts'
        // letters and their boxes, how far out from the middle the side ones
        // sit (a fraction of the screen's width) and how far up the
        // bearing's sits (a fraction of its height); the lock box's and the
        // seeker diamond's half sizes.
        const CROSS_ARM: f32 = 5.0;
        const LINE_WIDTH: f32 = 1.2;
        const READOUT_FONT: f32 = 12.0;
        const READOUT_BOX: (f32, f32) = (48.0, 19.0);
        const READOUT_OUT: f32 = 0.12;
        const BEARING_UP: f32 = 0.16;
        const LOCK_HALF: f32 = 9.0;
        const DIAMOND_HALF: f32 = 4.0;
        // The seeker circle's segments.
        const CIRCLE_SEGMENTS: usize = 40;
        // A lock out of view: an arrow this far out from the cross, its
        // length and half its width at the back.
        const ARROW_OUT: f32 = 30.0;
        const ARROW_LENGTH: f32 = 12.0;
        const ARROW_HALF_WIDTH: f32 = 6.0;

        // Real pixels from here on - what the screen lines and the screen text
        // are in.
        let to_pixels = app.window_manager.pixel_size.height as f32 / screen_height.max(1.0);
        let (middle_x, middle_y) = (screen_width * 0.5 * to_pixels, screen_height * 0.5 * to_pixels);
        let out = READOUT_OUT * screen_width * to_pixels;
        let up = BEARING_UP * screen_height * to_pixels;
        let (box_half_width, box_half_height) = (READOUT_BOX.0 * 0.5 * to_pixels, READOUT_BOX.1 * 0.5 * to_pixels);

        let active = scene.cameras.active();
        let eye = active.camera.position().coords;
        let look = (active.camera.rotation_modifier * active.camera.calc_forward_direction()).normalize();

        // Off the HUD's glass: how far the middle of the view is outside its
        // outline (degrees, negative inside). Measured on a plane square to
        // the line to the glass's middle, where every point is its
        // direction from the eye (so its distances are tangents of angles).
        let player_id = self.flight_manager.player_id();
        let glass: Vec<Vector3<f32>> = match self.flight_manager.hud_glass(player_id).filter(|glass| glass.len() >= 3) {
            Some(glass) => glass.iter().map(|point| map.on_glass(*point)).collect(),
            None => [(map.left, map.top), (map.right_px, map.top), (map.right_px, map.bottom), (map.left, map.bottom)].into_iter().map(|(x, y)| map.at(x, y)).collect(),
        };
        let off_glass_deg = (|| {
            let middle = glass.iter().sum::<Vector3<f32>>() / glass.len() as f32;
            let toward = (middle - eye).try_normalize(1e-6)?;
            let across = toward.cross(&(map.aircraft_rot * Vector3::y())).try_normalize(1e-6)?;
            let upward = across.cross(&toward);
            let on_plane = |direction: Vector3<f32>| {
                let ahead = direction.dot(&toward);
                (ahead > 1e-3).then(|| [direction.dot(&across) / ahead, direction.dot(&upward) / ahead])
            };
            let outline: Vec<[f32; 2]> = glass.iter().map(|point| on_plane(point - eye)).collect::<Option<_>>()?;
            let edges = convex_mask(&outline, 0.0)?;
            // Looking away from the glass altogether: well off it.
            let Some([x, y]) = on_plane(look) else { return Some(90.0) };
            let inside = edges.iter().map(|edge| edge[0] * x + edge[1] * y + edge[2]).fold(f32::MAX, f32::min);
            Some(-inside.atan().to_degrees())
        })()
        // No outline to go by (behind the eye): not on it.
        .unwrap_or(90.0);
        // Above the sill: how far the view is over it (degrees), in the jet's
        // frame - round from the nose (either side), and up from level.
        let local = map.aircraft_rot.inverse() * look;
        let azimuth = local.x.atan2(local.z).to_degrees();
        let elevation = local.y.clamp(-1.0, 1.0).asin().to_degrees();
        let over_sill_deg = crate::game::scenes::play::plane::aircraft_spec::sill_elevation(self.flight_manager.cockpit_sill(player_id), azimuth)
            .map_or(90.0, |sill| elevation - sill);
        let alpha = (off_glass_deg / HMD_FADE_DEG).clamp(0.0, 1.0) * (over_sill_deg / HMD_FADE_DEG).clamp(0.0, 1.0);
        if alpha <= 0.0 {
            return;
        }
        let line_width = LINE_WIDTH * to_pixels;

        // The Fox-2 mode: the helmet's boresight - the seeker - in the middle
        // of the view.
        let gimbal_limit = self.selected_weapon.as_ref()
            .and_then(|weapon| self.weapon_specs.get(weapon))
            .filter(|spec| spec.category == WeaponCategory::Missile && spec.guidance.seeker == Seeker::InfraRed)
            .map(|spec| spec.guidance.gimbal_limit_deg);
        if let Some(limit) = gimbal_limit {
            self.hmd_cue.set(Some(look));
            let color = [COLOR[0], COLOR[1], COLOR[2], 0.9 * alpha];
            let mut boresight_line = |start: (f32, f32), end: (f32, f32)| {
                app.render_sets.lines("hud_screen_lines").dynamic.push(LineInstance { start: [start.0, start.1, 0.0], end: [end.0, end.1, 0.0], color, width: line_width });
            };
            let nose = map.aircraft_rot * Vector3::z();
            if nose.angle(&look).to_degrees() > limit {
                // Past the gimbal limit: it can't look there.
                let reach = CROSS_ARM * 1.6 * to_pixels;
                boresight_line((middle_x - reach, middle_y - reach), (middle_x + reach, middle_y + reach));
                boresight_line((middle_x - reach, middle_y + reach), (middle_x + reach, middle_y - reach));
            } else if self.fire_control.seeker_target.is_none() {
                // Its field of view on screen, and the diamond in its middle.
                let half_fov = (active.projection.fovy.to_radians() * 0.5).tan().max(1e-4);
                let radius = fire_control::SEEKER_FIELD_DEG.to_radians().tan() / half_fov * middle_y;
                let point = |index: usize| {
                    let angle = index as f32 / CIRCLE_SEGMENTS as f32 * std::f32::consts::TAU;
                    (middle_x + radius * angle.cos(), middle_y + radius * angle.sin())
                };
                for index in 0..CIRCLE_SEGMENTS {
                    boresight_line(point(index), point(index + 1));
                }
                let half = DIAMOND_HALF * to_pixels;
                let corners = [(middle_x, middle_y - half), (middle_x + half, middle_y), (middle_x, middle_y + half), (middle_x - half, middle_y)];
                for index in 0..4 {
                    boresight_line(corners[index], corners[(index + 1) % 4]);
                }
            }
        }

        let line_color = [COLOR[0], COLOR[1], COLOR[2], 0.9 * alpha];
        let text_color = [0.0, 1.0, 77.0 / 255.0, alpha];

        let mut lines: Vec<((f32, f32), (f32, f32))> = Vec::new();
        let rectangle = |lines: &mut Vec<((f32, f32), (f32, f32))>, x: f32, y: f32, half_width: f32, half_height: f32| {
            let corners = [(x - half_width, y - half_height), (x + half_width, y - half_height), (x + half_width, y + half_height), (x - half_width, y + half_height)];
            for index in 0..4 {
                lines.push((corners[index], corners[(index + 1) % 4]));
            }
        };
        let triangle = |lines: &mut Vec<((f32, f32), (f32, f32))>, middle: nalgebra::Vector2<f32>, way: nalgebra::Vector2<f32>, length: f32, half_width: f32| {
            let across = nalgebra::Vector2::new(-way.y, way.x);
            let tip = middle + way * (length * 0.5);
            let base = middle - way * (length * 0.5);
            let corners = [tip, base + across * half_width, base - across * half_width];
            for index in 0..3 {
                lines.push(((corners[index].x, corners[index].y), (corners[(index + 1) % 3].x, corners[(index + 1) % 3].y)));
            }
        };
        // Where a world point is on screen (real pixels), if it's on it.
        let on_screen = |point: Vector3<f32>| {
            project_hud_point(active, point, screen_width, screen_height)
                .map(|(x, y)| (x * to_pixels, y * to_pixels))
                .filter(|(x, y)| (0.0..=screen_width * to_pixels).contains(x) && (0.0..=screen_height * to_pixels).contains(y))
        };

        // The cross, following the view.
        let arm = CROSS_ARM * to_pixels;
        lines.push(((middle_x - arm, middle_y), (middle_x + arm, middle_y)));
        lines.push(((middle_x, middle_y - arm), (middle_x, middle_y + arm)));

        // The box on what's locked, where it is in the view.
        let locked = self.hud_symbol.as_ref()
            .filter(|symbol| !symbol.diamond)
            .and_then(|symbol| symbol.track.as_ref())
            .and_then(|name| scene.content.renderizable_instances.get(name))
            .map(|instance| instance.instance.transform.position);
        if let Some((x, y)) = locked.and_then(on_screen) {
            let half = LOCK_HALF * to_pixels;
            rectangle(&mut lines, x, y, half, half);
        }

        // A lock out of view: an arrow out from the cross, pointing the way
        // to turn to it (the view's own right and up; straight behind,
        // down - turn round).
        let target = self.fire_control.seeker_target.as_ref()
            .and_then(|name| scene.content.renderizable_instances.get(name))
            .map(|instance| instance.instance.transform.position);
        let (width, height) = (app.window_manager.size.width, app.window_manager.size.height);
        let out_of_view = target.filter(|target| app.camera_resources.world_to_screen(active, Point3::from(*target), width, height).is_none());
        if let Some(target) = out_of_view {
            let view_up = active.camera.rotation_modifier * active.camera.up;
            if let Some(right) = look.cross(&view_up).try_normalize(1e-6) {
                let view_up = right.cross(&look);
                let to_target = target - eye;
                let way = nalgebra::Vector2::new(to_target.dot(&right), -to_target.dot(&view_up))
                    .try_normalize(1e-6)
                    .unwrap_or_else(|| nalgebra::Vector2::new(0.0, 1.0));
                let middle = nalgebra::Vector2::new(middle_x, middle_y) + way * (ARROW_OUT * to_pixels);
                triangle(&mut lines, middle, way, ARROW_LENGTH * to_pixels, ARROW_HALF_WIDTH * to_pixels);
            }
        }

        // The bearing over the cross, the speed left of it and the altitude
        // right of it, each in its box - as the HUD reads them.
        let readouts = [("hud_heading", middle_x, middle_y - up), ("hud_speed", middle_x - out, middle_y), ("hud_altitude", middle_x + out, middle_y)];
        for (_, x, y) in readouts {
            rectangle(&mut lines, x, y, box_half_width, box_half_height);
        }

        for (start, end) in lines {
            app.render_sets.lines("hud_screen_lines").dynamic.push(LineInstance { start: [start.0, start.1, 0.0], end: [end.0, end.1, 0.0], color: line_color, width: line_width });
        }
        // The bearing where the head points (north +Z, east -X - as the HUD's
        // heading tape has it); straight up or down, the last one stands.
        let head_bearing = Vector3::new(look.x, 0.0, look.z).try_normalize(1e-3)
            .map(|toward| ((-toward.x).atan2(toward.z).to_degrees().rem_euclid(360.0).round() as i32 % 360).to_string());
        for (key, x, y) in readouts {
            let Some(label) = Ui::get_ui_node(&mut app.ui.renderizable_elements, key).and_then(|node| node.as_label_mut()) else { continue };
            let text = match (key, &head_bearing) {
                ("hud_heading", Some(bearing)) => bearing.clone(),
                _ => label.text().to_owned(),
            };
            app.render_sets.text("hmd_text").push(&mut app.ui.text.font_system, &text, HUD_FONT, TextFrame::screen(x, y, READOUT_FONT * to_pixels), text_color);
        }
    }

    /// The seeker's symbol (see `HudSymbol`). Over the world, it sits on its
    /// point at a fixed size on screen. On the plate (`plate` - the HUD
    /// fixed to the jet, and how big it is on screen), it is where the line
    /// from the camera to the point crosses the plate - so it sits on the
    /// point whatever the head does (the plate is close: from the design eye
    /// a few cm of head offset would put it degrees off) - sized and turned
    /// with the plate, and left out once that's off the HUD's square. Not
    /// drawn behind the camera.
    fn draw_hud_symbol(scene: &Scene, app: &mut App, symbol: HudSymbol, plate: Option<(&GlassMap, f32)>) {
        const COLOR: [f32; 4] = [0.0, 1.0, 0.3, 0.95];
        // Its lines' width (px) - the diamond's, the box's.
        const DIAMOND_WIDTH: f32 = 4.0;
        const BOX_WIDTH: f32 = 1.2;
        let HudSymbol { point, track, up, half_size, diamond } = symbol;
        // A tracked aircraft where it's drawn this frame, not where the
        // weapons update last saw it.
        let point = track.and_then(|name| scene.content.renderizable_instances.get(&name))
            .map_or(point, |instance| instance.instance.transform.position);
        let camera = scene.cameras.active().camera.position().coords;
        let scale = app.hud_layout.draw_scale();
        let half_size = half_size * scale;
        let line_width = (if diamond { DIAMOND_WIDTH } else { BOX_WIDTH }) * scale;
        let (center, right, upward, half, width) = if let Some((map, print_scale)) = plate {
            let Some(direction) = (point - camera).try_normalize(1e-6) else { return };
            let half = half_size * map.meters_per_pixel();
            let Some(center) = map.ray_from(camera, direction).filter(|center| map.inside_frame(*center, half)) else {
                // Off the HUD's square: a lock gets an arrow at its edge,
                // pointing the way to it.
                if !diamond {
                    Self::draw_plate_arrow(app, map, direction, scale, print_scale, COLOR);
                }
                return;
            };
            (center, map.aircraft_rot * map.right, map.aircraft_rot * map.up, half, line_width * print_scale)
        } else {
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
            (point, right, screen_up, meters, line_width)
        };
        let corners = if diamond {
            [center + upward * half, center + right * half, center - upward * half, center - right * half]
        } else {
            [center + (upward + right) * half, center + (right - upward) * half, center - (upward + right) * half, center + (upward - right) * half]
        };
        for index in 0..4 {
            let (start, end) = (corners[index], corners[(index + 1) % 4]);
            app.render_sets.lines("hud_lines").dynamic.push(LineInstance { start: start.into(), end: end.into(), color: COLOR, width });
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

            // Update velocity vector marker position. On the plate, update_hud
            // draws it instead - this one stays hidden whatever the jet does
            // (standing still included), or it shows wherever it was left.
            let on_plate = self.hud_on_plate(scene);
            if on_plate {
                if let Some(marker) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "velocity_marker") {
                    marker.update_style(|s| s.set_text_color(UiColor::Rgba(0, 255, 75, 0)));
                }
            }
            if let (Some(velocity), false) = (&velocity, on_plate) {
                if velocity.magnitude() > 0.1 {
                    if let Some(player) = scene.content.renderizable_instances.get(player_id) {
                        let pos = player.instance.transform.position;
                        let vel_point = Point3::from(pos + velocity.normalize() * 100.0);
                        let screen_pos = app.camera_resources.world_to_screen(scene.cameras.active(), vel_point, app.window_manager.size.width, app.window_manager.size.height);
                        if let Some(screen_pos) = screen_pos {
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
/// of text (who, what they did, which plane, the rest), each with its color
/// and how wide it is (px).
struct KillFeedEntry {
    pieces: [(String, UiColor, f32); play_ui::KILL_FEED_PIECES],
    seconds_left: f32,
}

/// A camera key's change of view - see GameLogic::update_camera_focus.
enum CameraSwitch {
    To(CameraState),
    Next,
}

/// The seeker's HUD symbol - see GameLogic::draw_hud_symbol: where it points
/// in the world (or, `track`, the aircraft it's on - drawn where that's
/// rendered this frame), which way is up for it, how big (HUD px either way
/// from its middle), and a diamond (the seeker) or a box (a lock).
#[derive(Clone)]
struct HudSymbol {
    point: Vector3<f32>,
    track: Option<String>,
    up: Vector3<f32>,
    half_size: f32,
    diamond: bool,
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

    fn after_cameras(&mut self, scene: &mut Scene, app: &mut App) {
        // The M map draws instead of the world - its own frame plan, swapped
        // in for this frame's drawing whenever it's opened or closed.
        if app.tactical_map_active != self.showing_map_plan {
            self.showing_map_plan = app.tactical_map_active;
            app.frame_plan = if self.showing_map_plan { crate::game::frame_plans::tactical_map() } else { crate::game::frame_plans::flight() };
        }
        // While paused the HUD stays as the last flying frame left it.
        if !app.is_paused {
            self.update_hud(scene, app);
        }
        // The cockpit camera polls the tracker just before this. A frame that
        // arrives during startup or Retry rewrites the settings dot here,
        // including while the pause menu is up.
        main_menu_ui::refresh_head_tracking_if_needed(app);
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
