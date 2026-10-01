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
    fn meters_per_pixel(&self, point: Vector3<f32>) -> f32 {
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
    pub fn lines(&self, map: &MapSpec, live: &HashMap<String, LiveAircraft>, player: &str, view: &View, sea_level: f32) -> Vec<LineInstance> {
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

/// An X at `position`, facing the camera - where an aircraft was destroyed.
fn cross(lines: &mut Vec<LineInstance>, position: Vector3<f32>, rgb: [f32; 3], view: &View) {
    let half = WRECK_MARK_SIZE_PX * 0.5 * view.meters_per_pixel(position);
    let toward_camera = (view.camera_position - position).try_normalize(1e-6).unwrap_or_else(Vector3::z);
    let right = Vector3::y().cross(&toward_camera).try_normalize(1e-6).unwrap_or_else(Vector3::x);
    let up = toward_camera.cross(&right);
    let color = [rgb[0], rgb[1], rgb[2], 1.0];
    for diagonal in [right + up, right - up] {
        let arm = diagonal * (half / std::f32::consts::SQRT_2);
        lines.push(LineInstance { start: (position - arm).into(), end: (position + arm).into(), color, width: LINE_WIDTH });
    }
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
        LiveAircraft { position, rotation: UnitQuaternion::identity(), speed_kt: 0.0, altitude: position.y, wrecked, next_path_point: None }
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
