// Keyframed UI animation for a level's screens - the mission intro's black
// backdrop and title, the HUD fading in (UiTrack). Lives alongside
// event_handling.rs's `EventSystem` - see `EventSystem::apply_tracks`, which is
// what actually drives these every frame - and is authored in the same
// `level_planning.ron` file as the `event_list`. Anything in the world (aircraft,
// cameras, paths) is animated by the level's own animations instead (see
// engine::animation and play::level_sequences). Each track holds a full value at
// every keyframe and linearly interpolates between the two surrounding
// keyframes; before its first keyframe a track holds that first value, after its
// last it holds the last one.

use serde::Deserialize;

use crate::app::App;
use crate::engine::rendering::ui::ui::Ui;
use crate::engine::ui::color::{lerp_rgba, Fill, UiColor};
use crate::engine::ui::ui_node::Style;
use crate::engine::utils::lerps::lerp;

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
/// snapped to the first keyframe early.
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

pub(crate) fn normalize_all(ui: &mut [UiTrack]) {
    for track in ui { track.normalize(); }
}

pub(crate) fn apply_all(ui: &[UiTrack], game_time_ms: u64, app: &mut App) {
    for track in ui { track.apply(game_time_ms, app); }
}
