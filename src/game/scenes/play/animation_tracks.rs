// Keyframed parameter animation for the three kinds of thing a scene can target:
// 3D scene objects (Object3DTrack), UI nodes (UiTrack), and named cameras
// (CameraTrack). Lives alongside event_handling.rs's `EventSystem` - see
// `EventSystem::apply_tracks`, which is what actually drives these every frame -
// and is authored in the same `level_planning.ron` file as the existing
// `event_list`. Each track holds a full value at every keyframe (never a partial
// diff) and linearly interpolates between the two surrounding keyframes; before
// its first keyframe a track holds that first value, after its last it holds the
// last one - the same convention every keyframe animation tool uses.

use nalgebra::{Point3, UnitQuaternion, Vector3};
use serde::Deserialize;

use crate::app::App;
use crate::engine::rendering::ui::ui::Ui;
use crate::engine::scene_manager::scene::Scene;
use crate::engine::ui::color::{lerp_rgba, Fill, UiColor};
use crate::engine::ui::ui_node::Style;
use crate::engine::utils::lerps::lerp;

#[derive(Debug, Deserialize, Clone, Copy, Default)]
pub struct Vec3Value {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// Fixed camera orientation, degrees - `CameraTrack::Shot`'s manual
/// alternative to `look_at`. Matches `Camera`'s own yaw/pitch fields (no
/// roll - `Camera` doesn't have one).
#[derive(Debug, Deserialize, Clone, Copy)]
pub struct YawPitch {
    pub yaw: f32,
    pub pitch: f32,
}

// Cheap deterministic hash -> a pseudo-random f32 in [-1, 1] - the "random
// gradient at each integer lattice point" half of smooth_noise below. No
// relation to any particular RNG algorithm's quality guarantees; this only
// ever needs to look random to the eye; splitmix64's mixing step is a
// convenient, well-distributed way to get that from a plain integer.
fn hash_noise(n: i64) -> f32 {
    let mut x = n as u64;
    x = (x ^ (x >> 33)).wrapping_mul(0xff51afd7ed558ccd);
    x = (x ^ (x >> 33)).wrapping_mul(0xc4ceb9fe1a85ec53);
    x ^= x >> 33;
    (x as f64 / u64::MAX as f64 * 2.0 - 1.0) as f32
}

// 1D value noise: hash_noise gives an independent random value at every
// integer `t`, this smoothstep-interpolates between the two surrounding ones -
// continuous (no jump cuts, unlike raw per-frame randomness) but never
// actually repeats on any short window (unlike a sine wave, whose period is
// fixed and short enough to read as an obvious, geometric pattern - the
// "figure-8" a 2-axis sine combination traces out). `seed` decorrelates
// separate axes sampled at the same `t` from each other, so combining two of
// these doesn't trace any fixed shape either.
fn smooth_noise(t: f32, seed: i64) -> f32 {
    let i = t.floor();
    let f = t - i;
    let lattice = i as i64 + seed.wrapping_mul(104_729); // large prime - keeps different seeds' lattices from overlapping
    let a = hash_noise(lattice);
    let b = hash_noise(lattice + 1);
    let s = f * f * (3.0 - 2.0 * f); // smoothstep
    a + (b - a) * s
}

// Sums smooth_noise at increasing frequency/decreasing amplitude per octave
// (fractional Brownian motion - the standard technique for organic-looking
// noise, same idea real terrain/cloud generators use). A single smooth_noise
// channel alone reads as one uniform, deliberate-feeling sway; real hand
// tremor has a small fast jitter riding on top of a larger slow drift, not
// one smooth wave - three octaves layered together is what actually gives it
// that texture. `freq` deliberately isn't exactly doubled each octave (2.3x
// rather than 2.0x) - an exact power-of-two ratio between octaves is a known
// source of the result still reading as too regular/patterned.
fn fbm_noise(t: f32, seed: i64) -> f32 {
    let mut sum = 0.0;
    let mut amplitude = 0.5;
    let mut frequency = 1.0;
    for octave in 0..3 {
        sum += amplitude * smooth_noise(t * frequency, seed + octave * 7919);
        amplitude *= 0.5;
        frequency *= 2.3;
    }
    sum / 0.875 // normalizes back to roughly [-1, 1] (0.5 + 0.25 + 0.125 = 0.875)
}

/// A small, non-repeating per-axis wander added to `CameraTrack::Shot`'s
/// `look_at` aim point (see `Shot::apply`) - reads as a camera operator's aim
/// drifting slightly rather than the whole camera swaying (see `follow_offset`
/// for that instead). Deliberately smooth noise rather than a sine wave -
/// sine has a short, fixed period that reads as an obvious repeating pattern
/// (two sine axes combined trace a fixed Lissajous curve, e.g. a figure-8,
/// regardless of amplitude/frequency chosen) - this never exactly repeats.
/// Sampled off the shot's own local time (seconds since `start_time`, same
/// clock `fov` keyframes use), not absolute game_time, so restarting the shot
/// always restarts the wander at the same point instead of wherever the scene
/// clock happened to be.
#[derive(Debug, Deserialize, Clone, Copy)]
pub struct Wander {
    pub amplitude: Vec3Value,
    /// How fast the noise drifts through its own internal timeline - not a
    /// literal cycles/second like a sine wave's frequency (this isn't
    /// periodic), just a "how jumpy vs. how sweeping" knob. 1.0 is a
    /// reasonable starting point; try 0.3-0.5 for a slower, gentler drift.
    pub speed: f32,
}

impl Wander {
    fn sample(&self, local_time_ms: i64) -> Vector3<f32> {
        let t = local_time_ms as f32 / 1000.0 * self.speed;
        Vector3::new(
            self.amplitude.x * fbm_noise(t, 1),
            self.amplitude.y * fbm_noise(t, 2),
            self.amplitude.z * fbm_noise(t, 3),
        )
    }
}

#[derive(Debug, Deserialize, Clone, Copy)]
pub struct Vec2Value {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Deserialize, Clone, Copy)]
pub struct ColorValue {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    #[serde(default = "default_alpha")]
    pub a: u8,
}
fn default_alpha() -> u8 { 255 }

impl ColorValue {
    fn to_ui_color(self) -> UiColor {
        UiColor::Rgba(self.r, self.g, self.b, self.a)
    }

    fn from_rgba_f32(rgba: [f32; 4]) -> ColorValue {
        ColorValue {
            r: (rgba[0].clamp(0.0, 1.0) * 255.0).round() as u8,
            g: (rgba[1].clamp(0.0, 1.0) * 255.0).round() as u8,
            b: (rgba[2].clamp(0.0, 1.0) * 255.0).round() as u8,
            a: (rgba[3].clamp(0.0, 1.0) * 255.0).round() as u8,
        }
    }
}

fn lerp_vec3(a: Vec3Value, b: Vec3Value, t: f32) -> Vec3Value {
    Vec3Value { x: lerp(a.x, b.x, t), y: lerp(a.y, b.y, t), z: lerp(a.z, b.z, t) }
}

fn lerp_vec2(a: Vec2Value, b: Vec2Value, t: f32) -> Vec2Value {
    Vec2Value { x: lerp(a.x, b.x, t), y: lerp(a.y, b.y, t) }
}

fn lerp_color(a: ColorValue, b: ColorValue, t: f32) -> ColorValue {
    let from: [f32; 4] = a.to_ui_color().into();
    let to: [f32; 4] = b.to_ui_color().into();
    ColorValue::from_rgba_f32(lerp_rgba(from, to, t))
}

/// For non-lerpable (discrete) values like `bool` - holds `a` right up until the
/// next keyframe's own timestamp, then switches to `b` exactly there (`sample`
/// only ever reaches `t >= 1.0` at that instant), i.e. a plain step change rather
/// than a blend.
fn step<T: Copy>(a: T, b: T, t: f32) -> T {
    if t >= 1.0 { b } else { a }
}

/// One point on a track's timeline. `time` is in milliseconds, relative to the
/// owning track's own `start_time` - not scene start (see `local_time`).
#[derive(Debug, Deserialize, Clone, Copy)]
pub struct Keyframe<T: Copy> {
    pub time: u64,
    pub value: T,
}

/// Local time (ms since the track's own `start_time`) for `game_time_ms`. `None`
/// means the track hasn't started yet - its target is left untouched rather than
/// snapped to the first keyframe early, so a track that starts at 3000ms doesn't
/// stomp the object's authored/physics-driven state for the 3 seconds before that.
fn local_time(game_time_ms: u64, start_time: u64) -> Option<i64> {
    let local = game_time_ms as i64 - start_time as i64;
    (local >= 0).then_some(local)
}

/// Samples `keyframes` (must already be sorted by `time` - see `normalize`) at
/// `local_time_ms`: holds the first value before it starts, holds the last value
/// once it's past the end, linearly interpolates (via `lerp_fn`) between the two
/// surrounding keyframes otherwise. `None` only if `keyframes` is empty.
fn sample<T: Copy>(keyframes: &[Keyframe<T>], local_time_ms: i64, lerp_fn: impl Fn(T, T, f32) -> T) -> Option<T> {
    let first = keyframes.first()?;
    if local_time_ms <= first.time as i64 {
        return Some(first.value);
    }
    let last = keyframes.last().expect("checked non-empty above");
    if local_time_ms >= last.time as i64 {
        return Some(last.value);
    }
    for pair in keyframes.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if local_time_ms <= b.time as i64 {
            let span = (b.time as i64 - a.time as i64).max(1) as f32;
            let t = (local_time_ms - a.time as i64) as f32 / span;
            return Some(lerp_fn(a.value, b.value, t));
        }
    }
    Some(last.value) // unreachable given the clamps above, but keeps this total
}

/// Animates a 3D `GameObject`'s transform (see `assets/scenes/*/data.ron`),
/// looked up by its scene `id` in `scene.content.renderizable_instances` - the same map
/// physics writes into every frame (see `App::run`), so a track targeting a
/// physics-driven object will fight it every frame rather than composing with
/// it - unless physics is paused for that object for the track's duration (see
/// `PhysicsCommand::TogglePause`/`SetTransform`, and `GameLogic::update`'s
/// cinematic handling), in which case the track *is* the object's position for
/// as long as it runs.
///
/// `end_time`, if set, makes the track release control back once `game_time`
/// reaches it - a plain no-op past that point, same as before `start_time` -
/// instead of the default "hold the last keyframe's value forever." Needed for
/// exactly that physics hand-back case: once a paused rigidbody is un-paused and
/// resumes writing this object's transform every frame, a `Position` track that
/// kept re-asserting its last keyframe would permanently fight it right back.
#[derive(Debug, Deserialize)]
pub enum Object3DTrack {
    Position { target: String, #[serde(default)] start_time: u64, #[serde(default)] end_time: Option<u64>, keyframes: Vec<Keyframe<Vec3Value>> },
    /// Euler angles in degrees, converted to a quaternion each frame the same way
    /// `data.ron`'s own `rotation:` field is (see `RawTransform`/`Transform::from_raw`).
    Rotation { target: String, #[serde(default)] start_time: u64, #[serde(default)] end_time: Option<u64>, keyframes: Vec<Keyframe<Vec3Value>> },
    Scale { target: String, #[serde(default)] start_time: u64, #[serde(default)] end_time: Option<u64>, keyframes: Vec<Keyframe<Vec3Value>> },
}

impl Object3DTrack {
    fn normalize(&mut self) {
        match self {
            Object3DTrack::Position { keyframes, .. }
            | Object3DTrack::Rotation { keyframes, .. }
            | Object3DTrack::Scale { keyframes, .. } => keyframes.sort_by_key(|k| k.time),
        }
    }

    fn apply(&self, game_time_ms: u64, scene: &mut Scene, _app: &mut App) {
        match self {
            Object3DTrack::Position { target, start_time, end_time, keyframes } => {
                if end_time.is_some_and(|end| game_time_ms >= end) {
                    return;
                }
                let Some(local) = local_time(game_time_ms, *start_time) else { return };
                let Some(value) = sample(keyframes, local, lerp_vec3) else { return };
                if let Some(instance) = scene.content.renderizable_instances.get_mut(target) {
                    instance.instance.transform.position = Vector3::new(value.x, value.y, value.z);
                }
            }
            Object3DTrack::Rotation { target, start_time, end_time, keyframes } => {
                if end_time.is_some_and(|end| game_time_ms >= end) {
                    return;
                }
                let Some(local) = local_time(game_time_ms, *start_time) else { return };
                let Some(value) = sample(keyframes, local, lerp_vec3) else { return };
                if let Some(instance) = scene.content.renderizable_instances.get_mut(target) {
                    instance.instance.transform.rotation =
                        UnitQuaternion::from_euler_angles(value.x.to_radians(), value.y.to_radians(), value.z.to_radians());
                }
            }
            Object3DTrack::Scale { target, start_time, end_time, keyframes } => {
                if end_time.is_some_and(|end| game_time_ms >= end) {
                    return;
                }
                let Some(local) = local_time(game_time_ms, *start_time) else { return };
                let Some(value) = sample(keyframes, local, lerp_vec3) else { return };
                if let Some(instance) = scene.content.renderizable_instances.get_mut(target) {
                    instance.instance.transform.scale = Vector3::new(value.x, value.y, value.z);
                }
            }
        }
    }
}

/// Animates a UI node (see `assets/ui/*.ron`), looked up by the same
/// `"parent/child"` path `Ui::get_ui_node` uses everywhere else.
#[derive(Debug, Deserialize)]
pub enum UiTrack {
    Position { target: String, #[serde(default)] start_time: u64, keyframes: Vec<Keyframe<Vec2Value>> },
    Alpha { target: String, #[serde(default)] start_time: u64, keyframes: Vec<Keyframe<f32>> },
    Color { target: String, #[serde(default)] start_time: u64, keyframes: Vec<Keyframe<ColorValue>> },
    /// Shows/hides a node (`UiNode::set_active`, recursive over its children) -
    /// a discrete switch rather than a lerp (see `step`). A single keyframe (e.g.
    /// `keyframes: [(time: 0, value: true)]`) is the common case: nothing happens
    /// until `start_time`, then it flips once and holds.
    Active { target: String, #[serde(default)] start_time: u64, keyframes: Vec<Keyframe<bool>> },
}

impl UiTrack {
    fn normalize(&mut self) {
        match self {
            UiTrack::Position { keyframes, .. } => keyframes.sort_by_key(|k| k.time),
            UiTrack::Alpha { keyframes, .. } => keyframes.sort_by_key(|k| k.time),
            UiTrack::Color { keyframes, .. } => keyframes.sort_by_key(|k| k.time),
            UiTrack::Active { keyframes, .. } => keyframes.sort_by_key(|k| k.time),
        }
    }

    fn apply(&self, game_time_ms: u64, app: &mut App) {
        match self {
            UiTrack::Position { target, start_time, keyframes } => {
                let Some(local) = local_time(game_time_ms, *start_time) else { return };
                let Some(value) = sample(keyframes, local, lerp_vec2) else { return };
                if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, target) {
                    node.move_to(value.x, value.y);
                    app.ui.has_changed = true;
                }
            }
            UiTrack::Alpha { target, start_time, keyframes } => {
                let Some(local) = local_time(game_time_ms, *start_time) else { return };
                let Some(value) = sample(keyframes, local, lerp) else { return };
                if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, target) {
                    node.set_alpha(value);
                    app.ui.has_changed = true;
                }
            }
            UiTrack::Color { target, start_time, keyframes } => {
                let Some(local) = local_time(game_time_ms, *start_time) else { return };
                let Some(value) = sample(keyframes, local, lerp_color) else { return };
                if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, target) {
                    let color = value.to_ui_color();
                    node.update_style(|style| Style { background_color: Some(Fill::Solid(color)), ..style });
                    app.ui.has_changed = true;
                }
            }
            UiTrack::Active { target, start_time, keyframes } => {
                let Some(local) = local_time(game_time_ms, *start_time) else { return };
                let Some(value) = sample(keyframes, local, step) else { return };
                if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, target) {
                    node.set_active(value);
                    app.ui.has_changed = true;
                }
            }
        }
    }
}

/// Animates a named `CameraHandler` camera (see `camera.rs`) by name - `target`
/// must already be registered there (by default that's only `"main"`, the one
/// `GameLogic::camera_control` drives every frame from the plane's transform).
/// Tracks apply after `camera_control` each frame (see `GameLogic::update`), so a
/// track targeting `"main"` visibly wins over the flight follow-cam for as long
/// as it's running.
///
/// `Position`/`Fov` are keyframed like every other track - they hold their last
/// keyframe's value forever once reached, so a `main`-targeting one of these
/// should end on a value the follow-cam would produce anyway (or target a
/// separate, dedicated camera name via `CameraHandler::create_camera` instead) -
/// there's no "hand control back" step for these two. `Shot` is the exception:
/// it's bounded by its own `[start_time, end_time)` window and does nothing
/// outside it, so control reverts to `camera_control` automatically once it
/// ends - see its own doc comment.
#[derive(Debug, Deserialize)]
pub enum CameraTrack {
    Position { target: String, #[serde(default)] start_time: u64, keyframes: Vec<Keyframe<Vec3Value>> },
    Fov { target: String, #[serde(default)] start_time: u64, keyframes: Vec<Keyframe<f32>> },
    /// A bounded cinematic shot - one variant covering every position/
    /// orientation combination instead of a separate variant per combination
    /// (a fixed-position "look at the plane" shot and a "chase the plane"
    /// shot used to be entirely different track types, `LookAt`/`FollowAt`,
    /// even though they only ever differed in *where* position/orientation
    /// came from). Pick one position source and one orientation source per
    /// shot:
    ///
    /// - Position: `position` (fixed) or `follow_at` (a `GameObject` id -
    ///   position recomputed every frame as that object's current position
    ///   plus `follow_offset` rotated by its current orientation, the same
    ///   "chase cam" convention `camera_control`'s own flight follow-cam uses,
    ///   see `CameraState::Normal`'s `(0, 8, -50)`-style offset). A no-op frame
    ///   (nothing happens, same as before `start_time`) if neither is set.
    /// - Orientation: `look_at` (a `GameObject` id - aimed at its current
    ///   position every frame via `Camera::look_at`) or `rotation` (fixed
    ///   yaw/pitch degrees). Neither set leaves whatever orientation the
    ///   position step (or a previous frame) already left the camera at.
    ///
    /// Two shots back to back in the same `camera_tracks` list (one's
    /// `end_time` equal to the next one's `start_time`) reads as a hard cut -
    /// e.g. a static `look_at`-only shot handing off to a `follow_at` chase,
    /// both driven off the identical `game_time` so there's no gap frame.
    ///
    /// Unlike `Position`/`Fov` this isn't keyframed at all (besides `fov`) -
    /// it's a plain `[start_time, end_time)` window: outside it, this does
    /// nothing, so `target` just reverts to whatever else drives it (e.g.
    /// `camera_control`'s per-frame flight follow-cam) with no separate "hand
    /// back" step. `EventSystem::is_cinematic_camera_active` uses this same
    /// window to gate player input off for as long as the shot is running.
    Shot {
        target: String,
        #[serde(default)]
        position: Option<Vec3Value>,
        #[serde(default)]
        follow_at: Option<String>,
        // Only meaningful when follow_at is set - zero (dead center on the
        // followed object) if omitted.
        #[serde(default)]
        follow_offset: Vec3Value,
        #[serde(default)]
        look_at: Option<String>,
        #[serde(default)]
        rotation: Option<YawPitch>,
        /// A small, non-repeating drift on the camera's already-resolved
        /// position itself (see `Wander`'s own doc comment) - rotated by
        /// `follow_at`'s current orientation when that's set (same space as
        /// `follow_offset`), added directly in world space with a fixed
        /// `position` instead. This plus `look_at_wander` together is what
        /// actually reads as handheld - position alone (camera swaying, aim
        /// perfectly locked) reads as an operator standing still panning to
        /// track; aim alone (camera perfectly rigid, only the look direction
        /// drifting) reads as a mounted turret scanning. A real handheld
        /// shot has both: the camera itself never sits quite still, and the
        /// aim wanders on top of that.
        #[serde(default)]
        position_shake: Option<Wander>,
        /// Only meaningful alongside `look_at` - adds a small, non-repeating
        /// per-axis drift (see `Wander`'s own doc comment) to the aim point,
        /// in world space, before `Camera::look_at` runs. Reads as the aim
        /// wandering slightly rather than the whole camera swaying.
        #[serde(default)]
        look_at_wander: Option<Wander>,
        /// Keyframed the same way as every other track (see `Keyframe`/`sample`) -
        /// times are relative to this shot's own `start_time`, so e.g.
        /// `(time: 0, value: 65.0), (time: 2000, value: 20.0)` push-ins two
        /// seconds in. Leave empty for a fixed FOV (whatever `camera_control`'s
        /// current state already set). Nothing needs to hand FOV back manually
        /// once `end_time` passes either way - every `CameraState` (see
        /// `camera_control`) asserts its own fovy unconditionally each frame, so
        /// the very next frame after this track goes silent just reads as
        /// whatever that state's base FOV is.
        #[serde(default)]
        fov: Vec<Keyframe<f32>>,
        #[serde(default)]
        start_time: u64,
        end_time: u64,
    },
}

impl CameraTrack {
    fn normalize(&mut self) {
        match self {
            CameraTrack::Position { keyframes, .. } => keyframes.sort_by_key(|k| k.time),
            CameraTrack::Fov { keyframes, .. } => keyframes.sort_by_key(|k| k.time),
            CameraTrack::Shot { fov, .. } => fov.sort_by_key(|k| k.time),
        }
    }

    fn apply(&self, game_time_ms: u64, scene: &mut Scene, _app: &mut App) {
        match self {
            CameraTrack::Position { target, start_time, keyframes } => {
                let Some(local) = local_time(game_time_ms, *start_time) else { return };
                let Some(value) = sample(keyframes, local, lerp_vec3) else { return };
                if let Some(instance) = scene.cameras.get_mut(target) {
                    instance.camera.set_position(Point3::new(value.x, value.y, value.z));
                }
            }
            CameraTrack::Fov { target, start_time, keyframes } => {
                let Some(local) = local_time(game_time_ms, *start_time) else { return };
                let Some(value) = sample(keyframes, local, lerp) else { return };
                if let Some(instance) = scene.cameras.get_mut(target) {
                    instance.projection.fovy = value;
                }
            }
            CameraTrack::Shot { target, position, follow_at, follow_offset, look_at, rotation, position_shake, look_at_wander, fov, start_time, end_time } => {
                if game_time_ms < *start_time || game_time_ms >= *end_time {
                    return;
                }
                // Guaranteed Some - the gate above already confirmed game_time_ms
                // >= start_time.
                let local = local_time(game_time_ms, *start_time).unwrap_or(0);

                let resolved_position = if let Some(p) = position {
                    let shake = position_shake.map_or(Vector3::zeros(), |s| s.sample(local));
                    Point3::from(Vector3::new(p.x, p.y, p.z) + shake)
                } else if let Some(follow_id) = follow_at {
                    let Some(instance) = scene.content.renderizable_instances.get(follow_id) else { return };
                    let follow_rotation = instance.instance.transform.rotation;
                    let offset_world = follow_rotation * Vector3::new(follow_offset.x, follow_offset.y, follow_offset.z);
                    let shake = position_shake.map_or(Vector3::zeros(), |s| follow_rotation * s.sample(local));
                    Point3::from(instance.instance.transform.position + offset_world + shake)
                } else {
                    // Neither position source configured - nothing to do.
                    return;
                };

                // Some(None) would mean "look_at was set but the target wasn't
                // found" - bail the whole shot rather than render at a
                // half-resolved position/orientation, same as the old LookAt's
                // own early-return on a missing look_at target.
                let resolved_look_at = match look_at {
                    Some(look_at_id) => match scene.content.renderizable_instances.get(look_at_id) {
                        Some(instance) => {
                            let wander = look_at_wander.map_or(Vector3::zeros(), |w| w.sample(local));
                            Some(instance.instance.transform.position + wander)
                        }
                        None => return,
                    },
                    None => None,
                };

                if let Some(instance) = scene.cameras.get_mut(target) {
                    instance.camera.set_position(resolved_position);
                    if let Some(look_pos) = resolved_look_at {
                        instance.camera.look_at(Point3::from(look_pos));
                    } else if let Some(YawPitch { yaw, pitch }) = rotation {
                        instance.camera.set_yaw_pitch(yaw.to_radians(), pitch.to_radians());
                    }
                    if !fov.is_empty() {
                        if let Some(value) = sample(fov, local, lerp) {
                            instance.projection.fovy = value;
                        }
                    }
                }
            }
        }
    }

    /// Whether this is a `Shot` currently inside its own `[start_time,
    /// end_time)` window - see `EventSystem::is_cinematic_camera_active`.
    fn is_cinematic_active(&self, game_time_ms: u64) -> bool {
        match self {
            CameraTrack::Shot { start_time, end_time, .. } => {
                game_time_ms >= *start_time && game_time_ms < *end_time
            }
            _ => false,
        }
    }
}

pub(crate) fn normalize_all(object_3d: &mut [Object3DTrack], ui: &mut [UiTrack], camera: &mut [CameraTrack]) {
    for track in object_3d { track.normalize(); }
    for track in ui { track.normalize(); }
    for track in camera { track.normalize(); }
}

pub(crate) fn any_cinematic_active(camera: &[CameraTrack], game_time_ms: u64) -> bool {
    camera.iter().any(|track| track.is_cinematic_active(game_time_ms))
}

pub(crate) fn apply_all(object_3d: &[Object3DTrack], ui: &[UiTrack], camera: &[CameraTrack], game_time_ms: u64, scene: &mut Scene, app: &mut App) {
    for track in object_3d { track.apply(game_time_ms, scene, app); }
    for track in ui { track.apply(game_time_ms, app); }
    for track in camera { track.apply(game_time_ms, scene, app); }
}
