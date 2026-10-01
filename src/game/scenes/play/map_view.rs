//! The F6 flight editor's map - the world seen from straight above, to lay
//! out paths on: the land (from the level's collision meshes, shaded by
//! height) and the runway, a grid, every aircraft and every path. Click to
//! add a point to the selected aircraft's path, drag a point to move it,
//! right-click one to delete it, click an aircraft to select it; scroll
//! zooms (about the cursor), dragging empty space pans.
//!
//! Seen from above with +Z (north) up the screen, the world's +X is to the
//! LEFT - the same as an aircraft's +X is its left wing when it faces +Z -
//! so the map isn't mirrored against the 3D view.

use egui::{Color32, Pos2, Rect, RichText, Stroke, Vec2};
use nalgebra::{UnitQuaternion, Vector3};

use crate::game::scenes::play::flight_manager::LiveAircraft;
use crate::game::scenes::play::map::{Coalition, MapSpec};

/// One triangle of the map's background, flat on the ground: (x, z) world
/// meters per corner, and its color.
#[derive(Clone)]
pub struct TerrainTriangle {
    pub points: [[f32; 2]; 3],
    pub color: Color32,
}

/// What a collision mesh is, for how the map colors it.
#[derive(Clone, Copy)]
pub enum TerrainKind {
    /// Shaded from low green to high tan by height.
    Land,
    /// Flat grey.
    Runway,
}

/// The map's background triangles for a collision mesh (its raw vertices
/// and triangles, placed at `position` and scaled by `scale` like its node)
/// - only what's above `sea_level`, the rest is under the sea.
pub fn terrain_triangles(vertices: &[Vector3<f32>], indices: &[[u32; 3]], position: Vector3<f32>, scale: Vector3<f32>, sea_level: f32, kind: TerrainKind) -> Vec<TerrainTriangle> {
    let world: Vec<Vector3<f32>> = vertices.iter().map(|v| position + v.component_mul(&scale)).collect();
    let highest = world.iter().map(|v| v.y).fold(sea_level, f32::max);
    indices.iter()
        .filter_map(|triangle| {
            let corners = triangle.map(|index| world.get(index as usize).copied());
            let [Some(a), Some(b), Some(c)] = corners else { return None };
            let top = a.y.max(b.y).max(c.y);
            if top <= sea_level {
                return None;
            }
            let color = match kind {
                TerrainKind::Runway => Color32::from_rgb(95, 95, 100),
                TerrainKind::Land => {
                    let height = ((a.y + b.y + c.y) / 3.0 - sea_level) / (highest - sea_level).max(1.0);
                    lerp_color(Color32::from_rgb(55, 95, 55), Color32::from_rgb(165, 145, 105), height.clamp(0.0, 1.0))
                }
            };
            Some(TerrainTriangle { points: [[a.x, a.z], [b.x, b.z], [c.x, c.z]], color })
        })
        .collect()
}

fn lerp_color(from: Color32, to: Color32, t: f32) -> Color32 {
    let channel = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    Color32::from_rgb(channel(from.r(), to.r()), channel(from.g(), to.g()), channel(from.b(), to.b()))
}

/// Starting zoom - pixels per meter (0.05: 20 m a pixel, ~10 km across a
/// 500 px map) - and how far it zooms either way.
const DEFAULT_SCALE: f32 = 0.05;
const MIN_SCALE: f32 = 0.0005;
const MAX_SCALE: f32 = 2.0;
/// How much one wheel notch zooms.
const ZOOM_STEP: f32 = 1.2;
/// How much one wheel notch over a path point raises/lowers it (m), and
/// with Shift held.
const HEIGHT_STEP: f32 = 50.0;
const HEIGHT_STEP_FAST: f32 = 500.0;
/// How close (pixels) the cursor has to be to a point or an aircraft to
/// grab it.
const PICK_RADIUS: f32 = 9.0;
/// Grid spacings it picks from (m) - the smallest at least MIN_GRID_PIXELS
/// apart on screen.
const GRID_SPACINGS: [f32; 8] = [100.0, 250.0, 500.0, 1000.0, 2500.0, 5000.0, 10_000.0, 25_000.0];
const MIN_GRID_PIXELS: f32 = 40.0;
/// The sea, behind everything.
const SEA_COLOR: Color32 = Color32::from_rgb(22, 42, 66);
const GRID_COLOR: Color32 = Color32::from_rgba_premultiplied(255, 255, 255, 14);
const SELECTED_PATH_COLOR: Color32 = Color32::from_rgb(255, 205, 40);
const PLAYER_COLOR: Color32 = Color32::from_rgb(255, 210, 90);

/// What the map is showing - where it's looking, how zoomed - and what's
/// being dragged.
#[derive(Default)]
pub struct MapView {
    /// The world (x, z) at the map's center - `None` until first shown,
    /// when it centers on the selected aircraft.
    center: Option<[f32; 2]>,
    /// Pixels per meter - 0 until first shown.
    scale: f32,
    /// The selected aircraft's path point being dragged - its index.
    dragging_point: Option<usize>,
}

/// What clicking the map asked for, beyond editing the path in place.
pub enum MapAction {
    None,
    /// An aircraft was clicked - select it.
    Select(String),
}

/// Where world (x, z) lands on the map, and back.
struct Projection {
    rect: Rect,
    center: [f32; 2],
    scale: f32,
}

impl Projection {
    fn to_screen(&self, x: f32, z: f32) -> Pos2 {
        // +X to the left, +Z up - see the module docs.
        self.rect.center() + Vec2::new(-(x - self.center[0]) * self.scale, -(z - self.center[1]) * self.scale)
    }

    fn to_world(&self, point: Pos2) -> [f32; 2] {
        let offset = point - self.rect.center();
        [self.center[0] - offset.x / self.scale, self.center[1] - offset.y / self.scale]
    }
}

impl MapView {
    /// Centers the map on world (x, z).
    pub fn center_on(&mut self, x: f32, z: f32) {
        self.center = Some([x, z]);
    }

    /// Zooms and centers so all of `points` (world x, z) fit in a map
    /// `size` big, with a margin.
    fn fit(&mut self, points: &[[f32; 2]], size: Vec2) {
        let Some(first) = points.first() else { return };
        let (mut min, mut max) = (*first, *first);
        for point in points {
            min = [min[0].min(point[0]), min[1].min(point[1])];
            max = [max[0].max(point[0]), max[1].max(point[1])];
        }
        self.center = Some([(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5]);
        let span = (max[0] - min[0]).max(max[1] - min[1]).max(500.0);
        self.scale = (size.x.min(size.y) * 0.8 / span).clamp(MIN_SCALE, MAX_SCALE);
    }

    /// Draws the map filling `ui`, and handles clicks on it - editing the
    /// path of `selected` in `map` directly (adding, moving, deleting its
    /// points). `focus` is where to center it the first time it's shown.
    pub fn show(&mut self, ui: &mut egui::Ui, terrain: &[TerrainTriangle], map: &mut MapSpec, live: &std::collections::HashMap<String, LiveAircraft>, selected: Option<&str>, focus: Option<[f32; 2]>) -> MapAction {
        if self.scale <= 0.0 {
            self.scale = DEFAULT_SCALE;
        }
        if self.center.is_none() {
            self.center = Some(focus.unwrap_or([0.0, 0.0]));
        }

        // Its own controls, above the map.
        let mut fit = false;
        let mut center_selected = false;
        ui.horizontal(|ui| {
            fit = ui.button("Fit paths").on_hover_text("Zoom to show every path and aircraft").clicked();
            center_selected = ui.add_enabled(focus.is_some(), egui::Button::new("Center on selected")).clicked();
            ui.label(RichText::new("Click: add point · drag: move point / pan · right-click: delete · scroll: zoom · scroll on a point: its height (Shift: faster)").small().weak());
        });

        let (response, painter) = ui.allocate_painter(ui.available_size().max(Vec2::splat(100.0)), egui::Sense::click_and_drag());
        let rect = response.rect;
        if fit {
            let mut points: Vec<[f32; 2]> = map.aircraft().flat_map(|(_, aircraft)| aircraft.path.iter().map(|&(x, _, z)| [x, z])).collect();
            points.extend(map.aircraft().map(|(_, aircraft)| live.get(&aircraft.name).map(|live| [live.position.x, live.position.z]).unwrap_or([aircraft.position.0, aircraft.position.2])));
            self.fit(&points, rect.size());
        }
        if center_selected {
            if let Some([x, z]) = focus {
                self.center_on(x, z);
            }
        }

        // Scroll over one of the selected path's points: raise/lower it.
        // Anywhere else: zoom about the cursor - the point under it stays
        // put.
        if let Some(hover) = response.hover_pos() {
            let (scroll, shift) = ui.input(|input| (input.raw_scroll_delta.y, input.modifiers.shift));
            let hovered_point = selected.and_then(|name| {
                let projection = self.projection(rect);
                let index = map.find(name)?.1.path.iter().position(|&(x, _, z)| projection.to_screen(x, z).distance(hover) <= PICK_RADIUS)?;
                Some((name, index))
            });
            if let (true, Some((name, index))) = (scroll != 0.0, hovered_point) {
                let step = if shift { HEIGHT_STEP_FAST } else { HEIGHT_STEP };
                if let Some(point) = map.find_mut(name).and_then(|aircraft| aircraft.path.get_mut(index)) {
                    point.1 = (point.1 + step * scroll.signum()).max(0.0);
                }
            } else if scroll != 0.0 {
                let projection = self.projection(rect);
                let anchor = projection.to_world(hover);
                self.scale = (self.scale * ZOOM_STEP.powf(scroll.signum())).clamp(MIN_SCALE, MAX_SCALE);
                let offset = hover - rect.center();
                self.center = Some([anchor[0] + offset.x / self.scale, anchor[1] + offset.y / self.scale]);
            }
        }

        let action = self.interact(&response, rect, map, live, selected);
        self.paint(&painter, rect, terrain, map, live, selected);
        action
    }

    fn projection(&self, rect: Rect) -> Projection {
        Projection { rect, center: self.center.unwrap_or([0.0, 0.0]), scale: self.scale }
    }

    /// Clicks and drags on the map.
    fn interact(&mut self, response: &egui::Response, rect: Rect, map: &mut MapSpec, live: &std::collections::HashMap<String, LiveAircraft>, selected: Option<&str>) -> MapAction {
        let projection = self.projection(rect);
        let pointer = response.interact_pointer_pos().or(response.hover_pos());
        // The selected aircraft's path point under the cursor.
        let point_under = |map: &MapSpec, at: Pos2| -> Option<usize> {
            let (_, aircraft) = map.find(selected?)?;
            aircraft.path.iter().position(|&(x, _, z)| projection.to_screen(x, z).distance(at) <= PICK_RADIUS)
        };

        if response.drag_started_by(egui::PointerButton::Primary) {
            self.dragging_point = response.interact_pointer_pos().and_then(|at| point_under(map, at));
        }
        if response.dragged_by(egui::PointerButton::Primary) {
            match (self.dragging_point, selected, pointer) {
                (Some(index), Some(name), Some(at)) => {
                    let [x, z] = projection.to_world(at);
                    if let Some(point) = map.find_mut(name).and_then(|aircraft| aircraft.path.get_mut(index)) {
                        point.0 = x;
                        point.2 = z;
                    }
                }
                // Not on a point - pan.
                _ => {
                    let delta = response.drag_delta();
                    if let Some(center) = &mut self.center {
                        center[0] += delta.x / self.scale;
                        center[1] += delta.y / self.scale;
                    }
                }
            }
        }
        if response.drag_stopped() {
            self.dragging_point = None;
        }
        if response.dragged_by(egui::PointerButton::Middle) {
            let delta = response.drag_delta();
            if let Some(center) = &mut self.center {
                center[0] += delta.x / self.scale;
                center[1] += delta.y / self.scale;
            }
        }

        let Some(at) = response.interact_pointer_pos() else { return MapAction::None };
        if response.secondary_clicked() {
            if let (Some(index), Some(name)) = (point_under(map, at), selected) {
                if let Some(aircraft) = map.find_mut(name) {
                    aircraft.path.remove(index);
                }
            }
            return MapAction::None;
        }
        if !response.clicked() {
            return MapAction::None;
        }
        // An aircraft under it - select that.
        let aircraft_under = map.aircraft().find(|(_, aircraft)| {
            let [x, z] = live.get(&aircraft.name).map(|live| [live.position.x, live.position.z]).unwrap_or([aircraft.position.0, aircraft.position.2]);
            projection.to_screen(x, z).distance(at) <= PICK_RADIUS
        });
        if let Some((_, aircraft)) = aircraft_under {
            if Some(aircraft.name.as_str()) != selected {
                return MapAction::Select(aircraft.name.clone());
            }
        }
        // Otherwise a new point on the selected aircraft's path - at the
        // height of its last point, or where the aircraft flies now.
        if point_under(map, at).is_none() {
            if let Some(name) = selected {
                let altitude = live.get(name).map(|live| live.position.y);
                if let Some(aircraft) = map.find_mut(name) {
                    let height = aircraft.path.last().map(|point| point.1).or(altitude).unwrap_or(aircraft.position.1);
                    let [x, z] = projection.to_world(at);
                    aircraft.path.push((x, height, z));
                }
            }
        }
        MapAction::None
    }

    fn paint(&self, painter: &egui::Painter, rect: Rect, terrain: &[TerrainTriangle], map: &MapSpec, live: &std::collections::HashMap<String, LiveAircraft>, selected: Option<&str>) {
        let projection = self.projection(rect);
        let painter = painter.with_clip_rect(rect);
        painter.rect_filled(rect, 0.0, SEA_COLOR);

        // The land and the runway.
        let mut mesh = egui::Mesh::default();
        for triangle in terrain {
            let corners = triangle.points.map(|[x, z]| projection.to_screen(x, z));
            let bounds = Rect::from_points(&corners);
            if !bounds.intersects(rect) {
                continue;
            }
            let base = mesh.vertices.len() as u32;
            for corner in corners {
                mesh.colored_vertex(corner, triangle.color);
            }
            mesh.add_triangle(base, base + 1, base + 2);
        }
        painter.add(egui::Shape::mesh(mesh));

        self.paint_grid(&painter, &projection);

        // Paths - the selected aircraft's on top, numbered.
        let mut aircraft: Vec<(&Coalition, &crate::game::scenes::play::map::AircraftPlacement)> = map.aircraft().collect();
        aircraft.sort_by_key(|(_, aircraft)| Some(aircraft.name.as_str()) == selected);
        for (coalition, placement) in &aircraft {
            let is_selected = Some(placement.name.as_str()) == selected;
            let now = live.get(&placement.name);
            let (x, z) = now.map(|live| (live.position.x, live.position.z)).unwrap_or((placement.position.0, placement.position.2));
            let at = projection.to_screen(x, z);
            if !placement.path.is_empty() {
                let color = if is_selected { SELECTED_PATH_COLOR } else { coalition_color(coalition).gamma_multiply(0.6) };
                let points: Vec<Pos2> = placement.path.iter().map(|&(x, _, z)| projection.to_screen(x, z)).collect();
                let next = now.and_then(|live| live.next_path_point).filter(|next| *next < points.len()).unwrap_or(0);
                painter.add(egui::Shape::dashed_line(&[at, points[next]], Stroke::new(1.0, color.gamma_multiply(0.7)), 6.0, 4.0));
                painter.add(egui::Shape::line(points.clone(), Stroke::new(if is_selected { 2.0 } else { 1.5 }, color)));
                for (index, point) in points.iter().enumerate() {
                    painter.circle(*point, if is_selected { 5.0 } else { 3.0 }, color, Stroke::new(1.0, Color32::BLACK));
                    if is_selected {
                        let height = placement.path[index].1;
                        painter.text(*point + Vec2::new(8.0, -8.0), egui::Align2::LEFT_BOTTOM, format!("{}  {:.0} m", index + 1, height), egui::FontId::proportional(11.0), Color32::WHITE);
                    }
                }
            }
        }

        // Aircraft: an arrow along its heading, its name beside it.
        for (coalition, placement) in &aircraft {
            let is_selected = Some(placement.name.as_str()) == selected;
            let now = live.get(&placement.name);
            let (position, rotation) = now.map(|live| (live.position, live.rotation)).unwrap_or_else(|| (placement.position(), placement.rotation()));
            let at = projection.to_screen(position.x, position.z);
            let fill = if placement.player { PLAYER_COLOR } else { coalition_color(coalition) };
            paint_aircraft(&painter, at, heading_on_screen(&rotation), fill, is_selected);
            let label = if now.is_some_and(|live| live.wrecked) { format!("{} (wrecked)", placement.name) } else { placement.name.clone() };
            painter.text(at + Vec2::new(10.0, 10.0), egui::Align2::LEFT_TOP, label, egui::FontId::proportional(11.0), if is_selected { Color32::WHITE } else { Color32::from_gray(200) });
        }

        self.paint_compass_and_scale(&painter, rect);
    }

    /// Grid lines every so many meters - as dense as stays readable.
    fn paint_grid(&self, painter: &egui::Painter, projection: &Projection) {
        let spacing = GRID_SPACINGS.iter().copied().find(|spacing| spacing * self.scale >= MIN_GRID_PIXELS).unwrap_or(*GRID_SPACINGS.last().unwrap());
        let rect = projection.rect;
        let [left_x, top_z] = projection.to_world(rect.left_top());
        let [right_x, bottom_z] = projection.to_world(rect.right_bottom());
        let stroke = Stroke::new(1.0, GRID_COLOR);
        let mut x = (right_x.min(left_x) / spacing).floor() * spacing;
        while x <= right_x.max(left_x) {
            let screen_x = projection.to_screen(x, 0.0).x;
            painter.vline(screen_x, rect.y_range(), stroke);
            x += spacing;
        }
        let mut z = (bottom_z.min(top_z) / spacing).floor() * spacing;
        while z <= bottom_z.max(top_z) {
            let screen_y = projection.to_screen(0.0, z).y;
            painter.hline(rect.x_range(), screen_y, stroke);
            z += spacing;
        }
    }

    /// North (+Z) arrow top-right, a scale bar bottom-left.
    fn paint_compass_and_scale(&self, painter: &egui::Painter, rect: Rect) {
        let north = rect.right_top() + Vec2::new(-22.0, 30.0);
        painter.arrow(north + Vec2::new(0.0, 12.0), Vec2::new(0.0, -22.0), Stroke::new(2.0, Color32::WHITE));
        painter.text(north + Vec2::new(0.0, -14.0), egui::Align2::CENTER_BOTTOM, "N (+Z)", egui::FontId::proportional(11.0), Color32::WHITE);

        let meters = GRID_SPACINGS.iter().copied().rev().find(|meters| meters * self.scale <= 140.0).unwrap_or(GRID_SPACINGS[0]);
        let length = meters * self.scale;
        let start = rect.left_bottom() + Vec2::new(14.0, -16.0);
        let stroke = Stroke::new(2.0, Color32::WHITE);
        painter.line_segment([start, start + Vec2::new(length, 0.0)], stroke);
        painter.line_segment([start + Vec2::new(0.0, -4.0), start + Vec2::new(0.0, 4.0)], stroke);
        painter.line_segment([start + Vec2::new(length, -4.0), start + Vec2::new(length, 4.0)], stroke);
        let text = if meters >= 1000.0 { format!("{:.1} km", meters / 1000.0) } else { format!("{meters:.0} m") };
        painter.text(start + Vec2::new(length * 0.5, -6.0), egui::Align2::CENTER_BOTTOM, text, egui::FontId::proportional(11.0), Color32::WHITE);
    }
}

fn coalition_color(coalition: &Coalition) -> Color32 {
    Color32::from_rgb(coalition.color.0, coalition.color.1, coalition.color.2)
}

/// Which way the nose points on the map (a screen direction).
fn heading_on_screen(rotation: &UnitQuaternion<f32>) -> Vec2 {
    let nose = rotation * Vector3::z();
    // +X to the left, +Z up.
    Vec2::new(-nose.x, -nose.z).normalized()
}

/// An aircraft on the map: an arrowhead pointing `heading`, ringed when
/// selected.
fn paint_aircraft(painter: &egui::Painter, at: Pos2, heading: Vec2, fill: Color32, selected: bool) {
    let heading = if heading.is_finite() && heading.length() > 0.0 { heading } else { Vec2::new(0.0, -1.0) };
    let side = heading.rot90();
    let points = vec![at + heading * 9.0, at - heading * 6.0 + side * 6.0, at - heading * 6.0 - side * 6.0];
    painter.add(egui::Shape::convex_polygon(points, fill, Stroke::new(1.0, Color32::BLACK)));
    if selected {
        painter.circle_stroke(at, 13.0, Stroke::new(1.5, Color32::WHITE));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projection() -> Projection {
        Projection { rect: Rect::from_min_size(Pos2::ZERO, Vec2::splat(200.0)), center: [0.0, 0.0], scale: 0.1 }
    }

    #[test]
    fn plus_x_is_left_and_plus_z_is_up() {
        let projection = projection();
        let center = projection.rect.center();
        assert!(projection.to_screen(100.0, 0.0).x < center.x, "+X should be left of center");
        assert!(projection.to_screen(0.0, 100.0).y < center.y, "+Z should be above center");
    }

    #[test]
    fn screen_and_world_round_trip() {
        let projection = projection();
        let [x, z] = projection.to_world(projection.to_screen(123.0, -456.0));
        assert!((x - 123.0).abs() < 1e-3 && (z + 456.0).abs() < 1e-3, "{x}, {z}");
    }

    #[test]
    fn a_nose_along_plus_z_points_up_the_map() {
        let heading = heading_on_screen(&UnitQuaternion::identity());
        assert!(heading.y < -0.99, "{heading:?}");
    }

    #[test]
    fn only_land_above_the_sea_is_kept() {
        let vertices = [Vector3::new(0.0, -1.0, 0.0), Vector3::new(1.0, -1.0, 0.0), Vector3::new(0.0, -1.0, 1.0), Vector3::new(0.0, 5.0, 0.0)];
        let triangles = terrain_triangles(&vertices, &[[0, 1, 2], [0, 1, 3]], Vector3::zeros(), Vector3::repeat(1.0), 0.0, TerrainKind::Land);
        assert_eq!(triangles.len(), 1);
    }
}
