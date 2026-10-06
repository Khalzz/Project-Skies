//! F10's tactical map - the battle as lines on a dark screen, live, like a
//! mission debrief map: the land as height contours (stacked rings, one per
//! CONTOUR_SPACING of height, every INDEX_CONTOUR_EVERY-th brighter), every
//! aircraft as a triangle in its coalition's color with a line straight down
//! to the sea and a ring where it meets it (its height and where it is, at a
//! glance), each trailing its last few seconds of flight, over a grid on the
//! sea - and an X wherever an aircraft was destroyed. The camera orbits whatever F1/F2 follows (see play::camera's
//! tactical view). Drawn by the renderer's thick lines (see engine::
//! rendering::thick_lines) - the world itself isn't drawn meanwhile.

use std::collections::{HashMap, VecDeque};

use nalgebra::{UnitQuaternion, Vector3};

use crate::engine::rendering::thick_lines::LineInstance;
use crate::game::scenes::play::flight_manager::LiveAircraft;
use crate::game::scenes::play::map::MapSpec;

/// Height between contours (m), and every how many contours one is brighter
/// (an index contour - 500 m).
const CONTOUR_SPACING: f32 = 100.0;
const INDEX_CONTOUR_EVERY: i32 = 5;
/// Every line's width (px).
const LINE_WIDTH: f32 = 2.0;
/// Contour colors: low, high (by height), and the coastline.
const CONTOUR_LOW: [f32; 3] = [0.05, 0.30, 0.32];
const CONTOUR_HIGH: [f32; 3] = [0.45, 0.95, 0.80];
const COASTLINE: [f32; 3] = [0.55, 1.00, 0.90];
/// How bright a plain contour is next to an index one.
const PLAIN_CONTOUR_ALPHA: f32 = 0.45;

/// The sea grid: spacing (m), how far out from the camera's target it's
/// drawn (m), its color.
const GRID_SPACING: f32 = 5000.0;
const GRID_EXTENT: f32 = 60_000.0;
const GRID_COLOR: [f32; 4] = [0.20, 0.35, 0.50, 0.22];

/// Trails: a sample every TRAIL_SAMPLE_SECONDS, the last TRAIL_SAMPLES kept.
const TRAIL_SAMPLE_SECONDS: f32 = 0.5;
const TRAIL_SAMPLES: usize = 30;

/// An aircraft's triangle and its sea ring, in pixels on screen.
const AIRCRAFT_SIZE_PX: f32 = 16.0;
const SEA_RING_RADIUS_PX: f32 = 6.0;
const SEA_RING_SEGMENTS: usize = 16;
/// The player's color, whatever its coalition.
const PLAYER_COLOR: [f32; 3] = [1.0, 0.82, 0.30];
/// The X where an aircraft was destroyed, in pixels on screen - and its
/// color once that aircraft is gone from the map (deleted).
const WRECK_MARK_SIZE_PX: f32 = 14.0;
const WRECK_MARK_FALLBACK: [f32; 3] = [1.0, 0.35, 0.30];
/// The selected aircraft: the ring round it (px), its flight plan's color,
/// its points' markers (px), and the point it's flying to's color.
const SELECTED_RING_RADIUS_PX: f32 = 15.0;
/// The ring round an aircraft under the pointer (px) - a bit bigger than the
/// selected one's, so both show when it's both.
const HOVER_RING_RADIUS_PX: f32 = 19.0;
const FLIGHT_PLAN_COLOR: [f32; 3] = [0.95, 0.95, 1.0];
const PLAN_MARKER_SIZE_PX: f32 = 9.0;
const NEXT_POINT_COLOR: [f32; 3] = [1.0, 0.80, 0.16];

/// Where an aircraft was destroyed - see `TacticalMap::record`.
struct WreckMark {
    name: String,
    position: Vector3<f32>,
}

/// What the map shows but doesn't remember between levels.
#[derive(Default)]
pub struct TacticalMap {
    pub open: bool,
    /// Each aircraft's recent positions, oldest first, by name.
    trails: HashMap<String, VecDeque<Vector3<f32>>>,
    since_sample: f32,
    /// Every place an aircraft was destroyed this level, in order - kept
    /// after it respawns or is deleted.
    wreck_marks: Vec<WreckMark>,
    /// The aircraft wrecked right now, by name - a new mark when one joins.
    wrecked: std::collections::HashSet<String>,
}

/// The land's height contours, for the map's static lines - the collision
/// mesh (its raw vertices and triangles, placed at `position` and scaled by
/// `scale` like its node) sliced every CONTOUR_SPACING above `sea_level`,
/// plus the coastline just above it.
pub fn contour_lines(vertices: &[Vector3<f32>], indices: &[[u32; 3]], position: Vector3<f32>, scale: Vector3<f32>, sea_level: f32) -> Vec<LineInstance> {
    let world: Vec<Vector3<f32>> = vertices.iter().map(|v| position + v.component_mul(&scale)).collect();
    let highest = world.iter().map(|v| v.y).fold(sea_level, f32::max);
    let mut lines = Vec::new();
    for triangle in indices {
        let corners = triangle.map(|index| world.get(index as usize).copied());
        let [Some(a), Some(b), Some(c)] = corners else { continue };
        let (low, high) = (a.y.min(b.y).min(c.y), a.y.max(b.y).max(c.y));
        if high <= sea_level {
            continue;
        }
        // The coastline (just above the sea, so a flat beach still slices),
        // then each contour level through this triangle.
        let first = ((low.max(sea_level) - sea_level) / CONTOUR_SPACING).floor() as i32 + 1;
        let last = ((high - sea_level) / CONTOUR_SPACING).floor() as i32;
        let levels = std::iter::once(0).chain(first.max(1)..=last);
        for level in levels {
            let height = if level == 0 { sea_level + 1.0 } else { sea_level + level as f32 * CONTOUR_SPACING };
            let Some((from, to)) = slice(a, b, c, height) else { continue };
            let color = if level == 0 {
                [COASTLINE[0], COASTLINE[1], COASTLINE[2], 0.9]
            } else {
                let t = ((height - sea_level) / (highest - sea_level).max(1.0)).clamp(0.0, 1.0);
                let rgb = [0, 1, 2].map(|i| CONTOUR_LOW[i] + (CONTOUR_HIGH[i] - CONTOUR_LOW[i]) * t);
                let alpha = if level % INDEX_CONTOUR_EVERY == 0 { 0.95 } else { PLAIN_CONTOUR_ALPHA };
                [rgb[0], rgb[1], rgb[2], alpha]
            };
            lines.push(LineInstance { start: from.into(), end: to.into(), color, width: LINE_WIDTH });
        }
    }
    lines
}

/// Where the flat plane at `height` cuts triangle (a, b, c) - the segment
/// across it, `None` if it doesn't (or only touches a corner).
fn slice(a: Vector3<f32>, b: Vector3<f32>, c: Vector3<f32>, height: f32) -> Option<(Vector3<f32>, Vector3<f32>)> {
    let mut points = Vec::with_capacity(2);
    for (p, q) in [(a, b), (b, c), (c, a)] {
        let (dp, dq) = (p.y - height, q.y - height);
        if (dp < 0.0) != (dq < 0.0) {
            let t = dp / (dp - dq);
            points.push(p + (q - p) * t);
        }
    }
    (points.len() == 2).then(|| (points[0], points[1]))
}

/// Where the camera is and how it sees - for keeping things a set size on
/// screen.
pub struct View {
    pub camera_position: Vector3<f32>,
    /// Vertical field of view (deg) and the screen's height (px).
    pub fovy_deg: f32,
    pub screen_height: f32,
    /// What it's orbiting - the grid's drawn round it.
    pub focus: Vector3<f32>,
}

impl View {
    /// How many meters a pixel is at `point`.
    pub(crate) fn meters_per_pixel(&self, point: Vector3<f32>) -> f32 {
        let distance = (point - self.camera_position).magnitude().max(1.0);
        2.0 * distance * (self.fovy_deg.to_radians() * 0.5).tan() / self.screen_height.max(1.0)
    }
}

impl TacticalMap {
    /// Remembers where every aircraft is, every TRAIL_SAMPLE_SECONDS - call
    /// every frame, map open or not, so the trails are there when it opens.
    /// Aircraft gone from the scene lose theirs.
    /// Also marks, every frame, where any aircraft has just been destroyed.
    pub fn record(&mut self, live: &HashMap<String, LiveAircraft>, delta_time: f32) {
        for (name, aircraft) in live {
            if aircraft.wrecked {
                if self.wrecked.insert(name.clone()) {
                    self.wreck_marks.push(WreckMark { name: name.clone(), position: aircraft.position });
                }
            } else {
                // Respawned - its next loss is a new mark.
                self.wrecked.remove(name);
            }
        }
        self.wrecked.retain(|name| live.contains_key(name));
        self.trails.retain(|name, _| live.contains_key(name));
        self.since_sample += delta_time;
        if self.since_sample < TRAIL_SAMPLE_SECONDS {
            return;
        }
        self.since_sample = 0.0;
        for (name, aircraft) in live {
            let trail = self.trails.entry(name.clone()).or_default();
            trail.push_back(aircraft.position);
            while trail.len() > TRAIL_SAMPLES {
                trail.pop_front();
            }
        }
    }

    /// This frame's moving lines - the sea grid, then for every aircraft its
    /// trail, its drop line and sea ring, and its triangle.
    /// The `selected` aircraft gets a ring round it and its flight plan drawn
    /// - see `flight_plan`.
    /// The `hovered` one (under the pointer) gets a fainter, wider ring.
    pub fn lines(&self, map: &MapSpec, live: &HashMap<String, LiveAircraft>, player: &str, selected: Option<&str>, hovered: Option<&str>, view: &View, sea_level: f32) -> Vec<LineInstance> {
        let mut lines = Vec::new();
        self.grid(&mut lines, view, sea_level);
        for (coalition, placement) in map.aircraft() {
            let Some(aircraft) = live.get(&placement.name) else { continue };
            let rgb = if placement.name == player {
                PLAYER_COLOR
            } else {
                [coalition.color.0, coalition.color.1, coalition.color.2].map(|channel| channel as f32 / 255.0)
            };
            self.trail(&mut lines, &placement.name, aircraft.position, rgb);
            // A wreck is shown by its X (below), not as an aircraft.
            if !aircraft.wrecked {
                drop_line(&mut lines, aircraft.position, sea_level, rgb, view);
                triangle(&mut lines, aircraft.position, aircraft.rotation, rgb, view);
                // A line to the point its autopilot's flying to, and that
                // point marked.
                if let Some(waypoint) = aircraft.waypoint {
                    lines.push(LineInstance { start: aircraft.position.into(), end: waypoint.into(), color: [NEXT_POINT_COLOR[0], NEXT_POINT_COLOR[1], NEXT_POINT_COLOR[2], 0.55], width: LINE_WIDTH });
                    diamond(&mut lines, waypoint, PLAN_MARKER_SIZE_PX, [NEXT_POINT_COLOR[0], NEXT_POINT_COLOR[1], NEXT_POINT_COLOR[2], 0.9], view);
                }
            }
        }
        for mark in &self.wreck_marks {
            let rgb = match map.find(&mark.name) {
                Some(_) if mark.name == player => PLAYER_COLOR,
                Some((coalition, _)) => [coalition.color.0, coalition.color.1, coalition.color.2].map(|channel| channel as f32 / 255.0),
                None => WRECK_MARK_FALLBACK,
            };
            cross(&mut lines, mark.position, rgb, view);
        }
        if let Some((_, placement)) = selected.and_then(|name| map.find(name)) {
            let now = live.get(&placement.name);
            let (points, looped) = map.flight_plan(placement);
            flight_plan(&mut lines, &points, looped, now, sea_level, view);
            if let Some(aircraft) = now {
                ring(&mut lines, aircraft.position, SELECTED_RING_RADIUS_PX, [1.0, 1.0, 1.0, 0.9], view);
            }
        }
        if let Some(aircraft) = hovered.and_then(|name| live.get(name)).filter(|aircraft| !aircraft.wrecked) {
            ring(&mut lines, aircraft.position, HOVER_RING_RADIUS_PX, [1.0, 1.0, 1.0, 0.45], view);
        }
        lines
    }

    fn grid(&self, lines: &mut Vec<LineInstance>, view: &View, sea_level: f32) {
        let snap = |value: f32| (value / GRID_SPACING).round() * GRID_SPACING;
        let (center_x, center_z) = (snap(view.focus.x), snap(view.focus.z));
        let steps = (GRID_EXTENT / GRID_SPACING) as i32;
        for step in -steps..=steps {
            let offset = step as f32 * GRID_SPACING;
            let line = |from: Vector3<f32>, to: Vector3<f32>| LineInstance { start: from.into(), end: to.into(), color: GRID_COLOR, width: LINE_WIDTH };
            lines.push(line(Vector3::new(center_x + offset, sea_level, center_z - GRID_EXTENT), Vector3::new(center_x + offset, sea_level, center_z + GRID_EXTENT)));
            lines.push(line(Vector3::new(center_x - GRID_EXTENT, sea_level, center_z + offset), Vector3::new(center_x + GRID_EXTENT, sea_level, center_z + offset)));
        }
    }

    /// Its last positions, fading out toward the oldest, joined to where it
    /// is now.
    fn trail(&self, lines: &mut Vec<LineInstance>, name: &str, now: Vector3<f32>, rgb: [f32; 3]) {
        let Some(trail) = self.trails.get(name) else { return };
        let points: Vec<Vector3<f32>> = trail.iter().copied().chain(std::iter::once(now)).collect();
        let count = points.len().max(2) as f32;
        for (index, pair) in points.windows(2).enumerate() {
            let alpha = 0.75 * (index + 1) as f32 / (count - 1.0);
            lines.push(LineInstance { start: pair[0].into(), end: pair[1].into(), color: [rgb[0], rgb[1], rgb[2], alpha], width: LINE_WIDTH });
        }
    }
}

/// A line from the aircraft straight down to the sea, and a ring there.
fn drop_line(lines: &mut Vec<LineInstance>, position: Vector3<f32>, sea_level: f32, rgb: [f32; 3], view: &View) {
    let foot = Vector3::new(position.x, sea_level, position.z);
    lines.push(LineInstance { start: position.into(), end: foot.into(), color: [rgb[0], rgb[1], rgb[2], 0.45], width: LINE_WIDTH });
    let radius = SEA_RING_RADIUS_PX * view.meters_per_pixel(foot);
    let point = |i: usize| {
        let angle = i as f32 / SEA_RING_SEGMENTS as f32 * std::f32::consts::TAU;
        foot + Vector3::new(angle.cos(), 0.0, angle.sin()) * radius
    };
    for i in 0..SEA_RING_SEGMENTS {
        lines.push(LineInstance { start: point(i).into(), end: point(i + 1).into(), color: [rgb[0], rgb[1], rgb[2], 0.7], width: LINE_WIDTH });
    }
}

/// Two directions square to the line of sight at `position` (right, up) -
/// for drawing something flat facing the camera.
fn facing_camera(position: Vector3<f32>, view: &View) -> (Vector3<f32>, Vector3<f32>) {
    let toward_camera = (view.camera_position - position).try_normalize(1e-6).unwrap_or_else(Vector3::z);
    let right = Vector3::y().cross(&toward_camera).try_normalize(1e-6).unwrap_or_else(Vector3::x);
    (right, toward_camera.cross(&right))
}

/// A circle `radius_px` round `position`, facing the camera.
fn ring(lines: &mut Vec<LineInstance>, position: Vector3<f32>, radius_px: f32, color: [f32; 4], view: &View) {
    const SEGMENTS: usize = 24;
    let radius = radius_px * view.meters_per_pixel(position);
    let (right, up) = facing_camera(position, view);
    let point = |i: usize| {
        let angle = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
        position + (right * angle.cos() + up * angle.sin()) * radius
    };
    for i in 0..SEGMENTS {
        lines.push(LineInstance { start: point(i).into(), end: point(i + 1).into(), color, width: LINE_WIDTH });
    }
}

/// A diamond `size_px` across at `position`, facing the camera.
fn diamond(lines: &mut Vec<LineInstance>, position: Vector3<f32>, size_px: f32, color: [f32; 4], view: &View) {
    let half = size_px * 0.5 * view.meters_per_pixel(position);
    let (right, up) = facing_camera(position, view);
    let corners = [position + up * half, position + right * half, position - up * half, position - right * half];
    for i in 0..4 {
        lines.push(LineInstance { start: corners[i].into(), end: corners[(i + 1) % 4].into(), color, width: LINE_WIDTH });
    }
}

/// The selected aircraft's flight plan: its path's points joined in order
/// (and the last back to the first when it loops), a diamond on each with a
/// faint line down to the sea (their heights), and a line from the aircraft
/// to the point it's flying to - that point's diamond in gold.
pub(crate) fn flight_plan(lines: &mut Vec<LineInstance>, path: &[(f32, f32, f32)], looped: bool, aircraft: Option<&LiveAircraft>, sea_level: f32, view: &View) {
    if path.is_empty() {
        return;
    }
    let points: Vec<Vector3<f32>> = path.iter().map(|&(x, y, z)| Vector3::new(x, y, z)).collect();
    let plan = |alpha: f32| [FLIGHT_PLAN_COLOR[0], FLIGHT_PLAN_COLOR[1], FLIGHT_PLAN_COLOR[2], alpha];
    for pair in points.windows(2) {
        lines.push(LineInstance { start: pair[0].into(), end: pair[1].into(), color: plan(0.9), width: LINE_WIDTH });
    }
    if looped && points.len() > 2 {
        // Back round to the start - dimmer, it's the loop.
        lines.push(LineInstance { start: points[points.len() - 1].into(), end: points[0].into(), color: plan(0.35), width: LINE_WIDTH });
    }
    let next = aircraft.and_then(|aircraft| aircraft.next_path_point).filter(|next| *next < points.len());
    for (index, point) in points.iter().enumerate() {
        let foot = Vector3::new(point.x, sea_level, point.z);
        lines.push(LineInstance { start: (*point).into(), end: foot.into(), color: plan(0.2), width: LINE_WIDTH });
        let color = if Some(index) == next { [NEXT_POINT_COLOR[0], NEXT_POINT_COLOR[1], NEXT_POINT_COLOR[2], 1.0] } else { plan(1.0) };
        diamond(lines, *point, PLAN_MARKER_SIZE_PX, color, view);
    }
    if let (Some(aircraft), Some(next)) = (aircraft, next) {
        if !aircraft.wrecked {
            lines.push(LineInstance { start: aircraft.position.into(), end: points[next].into(), color: [NEXT_POINT_COLOR[0], NEXT_POINT_COLOR[1], NEXT_POINT_COLOR[2], 0.6], width: LINE_WIDTH });
        }
    }
}

/// An X at `position`, facing the camera - where an aircraft was destroyed.
fn cross(lines: &mut Vec<LineInstance>, position: Vector3<f32>, rgb: [f32; 3], view: &View) {
    let half = WRECK_MARK_SIZE_PX * 0.5 * view.meters_per_pixel(position);
    let (right, up) = facing_camera(position, view);
    let color = [rgb[0], rgb[1], rgb[2], 1.0];
    for diagonal in [right + up, right - up] {
        let arm = diagonal * (half / std::f32::consts::SQRT_2);
        lines.push(LineInstance { start: (position - arm).into(), end: (position + arm).into(), color, width: LINE_WIDTH });
    }
}

/// A missile in flight: a streak back along where it came from, in its
/// shooter's color, and a bright little head where it is - a set size on
/// screen, like the aircraft.
pub fn missile_marker(lines: &mut Vec<LineInstance>, position: Vector3<f32>, velocity: Vector3<f32>, rgb: [f32; 3], view: &View) {
    const STREAK_PX: f32 = 18.0;
    const HEAD_PX: f32 = 5.0;
    let meters_per_pixel = view.meters_per_pixel(position);
    let back = -velocity.try_normalize(1e-3).unwrap_or_else(Vector3::z);
    let tail = position + back * STREAK_PX * meters_per_pixel;
    lines.push(LineInstance { start: tail.into(), end: position.into(), color: [rgb[0], rgb[1], rgb[2], 0.9], width: LINE_WIDTH });
    diamond(lines, position, HEAD_PX, [1.0, 0.95, 0.85, 1.0], view);
}

/// The aircraft as a triangle in its own wings' plane - nose ahead, a wing
/// tip either side - so it shows its heading, climb and bank.
fn triangle(lines: &mut Vec<LineInstance>, position: Vector3<f32>, rotation: UnitQuaternion<f32>, rgb: [f32; 3], view: &View) {
    let size = AIRCRAFT_SIZE_PX * view.meters_per_pixel(position);
    // +Z the nose, +X the left wing.
    let nose = rotation * Vector3::z();
    let left = rotation * Vector3::x();
    let corners = [
        position + nose * size * 0.6,
        position - nose * size * 0.4 + left * size * 0.45,
        position - nose * size * 0.4 - left * size * 0.45,
    ];
    let color = [rgb[0], rgb[1], rgb[2], 1.0];
    for i in 0..3 {
        lines.push(LineInstance { start: corners[i].into(), end: corners[(i + 1) % 3].into(), color, width: LINE_WIDTH });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plane_through_a_triangle_cuts_one_segment() {
        let (a, b, c) = (Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 200.0, 0.0), Vector3::new(0.0, 200.0, 10.0));
        let (from, to) = slice(a, b, c, 100.0).expect("100 m cuts it");
        assert!((from.y - 100.0).abs() < 1e-3 && (to.y - 100.0).abs() < 1e-3);
        assert!(slice(a, b, c, 300.0).is_none());
    }

    fn live(position: Vector3<f32>, wrecked: bool) -> LiveAircraft {
        LiveAircraft { position, rotation: UnitQuaternion::identity(), speed_kt: 0.0, altitude: position.y, wrecked, next_path_point: None, waypoint: None, g: 1.0, g_felt: true, stick_pull: 0.0, g_command: 1.0, autopilot: false, flown_by: "", stick_target_pull: 0.0 }
    }

    #[test]
    fn each_loss_is_marked_once_where_it_happened() {
        let mut map = TacticalMap::default();
        let mut aircraft = HashMap::new();
        aircraft.insert("bot".to_owned(), live(Vector3::new(0.0, 500.0, 0.0), false));
        map.record(&aircraft, 0.1);
        // Destroyed at (100, 20, 0) - marked there, once, however long it sinks.
        aircraft.insert("bot".to_owned(), live(Vector3::new(100.0, 20.0, 0.0), true));
        map.record(&aircraft, 0.1);
        aircraft.insert("bot".to_owned(), live(Vector3::new(100.0, -5.0, 0.0), true));
        map.record(&aircraft, 0.1);
        assert_eq!(map.wreck_marks.len(), 1);
        assert_eq!(map.wreck_marks[0].position, Vector3::new(100.0, 20.0, 0.0));
        // Respawned, then lost again - a second mark; both stay.
        aircraft.insert("bot".to_owned(), live(Vector3::new(0.0, 500.0, 0.0), false));
        map.record(&aircraft, 0.1);
        aircraft.insert("bot".to_owned(), live(Vector3::new(900.0, 30.0, 0.0), true));
        map.record(&aircraft, 0.1);
        assert_eq!(map.wreck_marks.len(), 2);
    }

    #[test]
    fn a_hill_gets_a_coastline_and_its_contours() {
        // A 350 m pyramid: coastline + 100, 200, 300 m - per face.
        let vertices = [Vector3::new(-1.0, -0.1, -1.0), Vector3::new(1.0, -0.1, -1.0), Vector3::new(1.0, -0.1, 1.0), Vector3::new(-1.0, -0.1, 1.0), Vector3::new(0.0, 0.35, 0.0)];
        let faces = [[0, 1, 4], [1, 2, 4], [2, 3, 4], [3, 0, 4]];
        let lines = contour_lines(&vertices, &faces, Vector3::zeros(), Vector3::repeat(1000.0), 0.0);
        assert_eq!(lines.len(), 4 * 4);
    }
}
