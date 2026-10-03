//! F10's tactical map, the egui half (see play::tactical_map for the lines):
//! - a bar on top: a toggle that keeps the camera centered on the selected
//!   aircraft, following it, and who's selected;
//! - the camera free otherwise: dragging the map moves it (see
//!   play::camera's pan_tactical);
//! - pressing an aircraft's triangle on the map selects it (hovering one
//!   rings it, with a hand cursor);
//! - the Aircraft window: every aircraft, by coalition - click one to select
//!   it, double-click to follow it too;
//! - the Flight plan window: the selected aircraft's path, every point
//!   editable - moved, added (at the end, after any point, where the
//!   aircraft is), deleted, and dragged up and down the list to reorder;
//! - dragging a flight plan point on the map moves it (across, at its
//!   height; up and down with Shift).
//!
//! The play scene fills in where everything is on screen and what's in the
//! map every frame (GameLogic::update_tactical_map), and applies a changed
//! path (`path_edit`); this only draws and reports. Drawn by the egui overlay
//! pass (see render_pass.rs), which is why it lives on `App`.

use egui::{Color32, Pos2, RichText};
use nalgebra::Vector3;

use crate::game::scenes::play::flight_manager::LiveAircraft;

/// How close (pixels) a click has to be to an aircraft to select it - and
/// to a flight plan point to grab it (points come first).
const PICK_RADIUS: f32 = 16.0;
const POINT_PICK_RADIUS: f32 = 10.0;

/// A flight plan point being dragged on the map this frame - for the scene
/// to move (taken there).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointDrag {
    /// Which point of the selected aircraft's path.
    pub index: usize,
    /// Where the pointer is (logical pixels) - the point goes under it, at
    /// its own height.
    pub pointer: Pos2,
    /// How far the pointer moved this frame (pixels) - for `vertical`.
    pub delta: egui::Vec2,
    /// Shift held: up and down instead of across.
    pub vertical: bool,
}

const ERROR_COLOR: Color32 = Color32::from_rgb(255, 110, 100);
const OK_COLOR: Color32 = Color32::from_rgb(140, 220, 140);
/// The point the aircraft is flying to, in the table and by its marker.
const NEXT_POINT_COLOR: Color32 = Color32::from_rgb(255, 205, 40);
const PLAYER_COLOR: Color32 = Color32::from_rgb(255, 210, 90);

/// How an aircraft in the list is doing.
#[derive(Clone, Copy, PartialEq)]
pub enum AircraftStatus {
    Flying,
    Destroyed,
    /// In the map, but not in the scene.
    Missing,
}

/// One coalition in the Aircraft window.
pub struct RosterCoalition {
    pub name: String,
    pub color: Color32,
    pub aircraft: Vec<RosterEntry>,
}

pub struct RosterEntry {
    pub name: String,
    pub plane: String,
    pub is_player: bool,
    pub status: AircraftStatus,
}

/// The selected aircraft's flight plan, as the window shows (and edits) it.
pub struct FlightPlan {
    pub name: String,
    pub coalition: String,
    pub color: Color32,
    pub plane: String,
    pub is_player: bool,
    /// How it is right now - `None` when it isn't in the scene.
    pub live: Option<LiveAircraft>,
    /// Its path's points, world meters (x, altitude, z).
    pub points: Vec<(f32, f32, f32)>,
    /// The point its autopilot is flying to, if it's flying the path.
    pub next: Option<usize>,
    /// Where "+ Add point" puts a new last point (see flight_editor's
    /// next_path_point).
    pub next_suggestion: (f32, f32, f32),
}

#[derive(Default)]
pub struct TacticalUi {
    /// The aircraft clicked last, by name.
    pub selected: Option<String>,
    /// The aircraft under the pointer (on the map or in the list), by name -
    /// for the scene to ring it.
    pub hovered: Option<String>,
    /// The camera follows the selected aircraft (the center button on) -
    /// otherwise it's free, moved by dragging the map.
    pub following: bool,
    /// How far the map's been dragged since the scene last looked, in screen
    /// pixels (x right, y down) - for the scene to move the camera (taken
    /// there).
    pub pan_delta: (f32, f32),
    /// The map's being dragged (the press started on it, not a window).
    dragging_map: bool,
    /// Where the selected flight plan's points are on screen (logical
    /// pixels), by index - filled by the scene every frame.
    pub point_pickables: Vec<(usize, Pos2)>,
    /// A point being dragged on the map this frame - see `PointDrag`.
    pub point_drag: Option<PointDrag>,
    /// The point a drag on the map started on - held until the release.
    dragging_point: Option<usize>,
    /// The Flight plan table's row being dragged to a new place.
    dragging_row: Option<usize>,
    /// The right button went down on the map (not a window) - orbiting the
    /// camera, until it's let go.
    orbiting: bool,
    /// The selected aircraft's path, changed in the Flight plan window - for
    /// the scene to apply (taken there): its name and every point.
    pub path_edit: Option<(String, Vec<(f32, f32, f32)>)>,
    /// Every aircraft that can be clicked, and where it is on screen
    /// (logical pixels) - filled by the scene every frame.
    pub pickables: Vec<(String, Pos2)>,
    /// The selected flight plan's point numbers: where each point is on
    /// screen, its label and color - filled by the scene every frame.
    pub labels: Vec<(Pos2, String, Color32)>,
    /// Every aircraft, by coalition - filled by the scene every frame.
    pub roster: Vec<RosterCoalition>,
    /// The selected aircraft's flight plan - filled by the scene every frame.
    pub plan: Option<FlightPlan>,
}

impl TacticalUi {
    /// Whether something's being dragged on it - the map, a point, a row of
    /// the flight plan, or the camera orbited - so the mouse can be captured
    /// meanwhile and the drag doesn't stop at the screen's edge (see
    /// render_pass.rs's infinite drag).
    pub fn is_dragging(&self) -> bool {
        self.dragging_map || self.dragging_point.is_some() || self.dragging_row.is_some() || self.orbiting
    }

    /// Draws it all and takes a click on the map - call inside the egui
    /// frame while the tactical map is up.
    pub fn draw(&mut self, ctx: &egui::Context) {
        self.pick(ctx);
        self.top_bar(ctx);
        self.point_labels(ctx);
        self.roster_window(ctx);
        self.plan_window(ctx);
    }

    /// The aircraft under the pointer (nearest within PICK_RADIUS, not over a
    /// window) is hovered - a hand cursor over it - and pressing the button
    /// selects it, there and then (no waiting for a click's release). A
    /// press anywhere on the map, then moving, drags it (the camera goes
    /// free - see `pan_delta`). Every press on the map says on the console
    /// what it hit.
    fn pick(&mut self, ctx: &egui::Context) {
        let (clicked, down, delta, at, shift) = ctx.input(|input| (input.pointer.primary_pressed(), input.pointer.primary_down(), input.pointer.delta(), input.pointer.hover_pos(), input.modifiers.shift));
        let over_window = ctx.is_pointer_over_area();
        let (orbit_pressed, orbit_down) = ctx.input(|input| (input.pointer.secondary_pressed(), input.pointer.secondary_down()));
        if orbit_pressed && !over_window {
            self.orbiting = true;
        }
        if !orbit_down {
            self.orbiting = false;
        }
        // The flight plan point under the pointer, if any - grabbed before
        // anything else.
        let point_under = at.and_then(|at| {
            self.point_pickables.iter()
                .map(|(index, position)| (*index, position.distance(at)))
                .filter(|(_, distance)| *distance <= POINT_PICK_RADIUS)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(index, _)| index)
        });
        if clicked && !over_window {
            match point_under {
                Some(index) => self.dragging_point = Some(index),
                None => self.dragging_map = true,
            }
        }
        // This frame's movement counts even if the button came up in it.
        if let (Some(index), Some(pointer)) = (self.dragging_point, at) {
            if delta != egui::Vec2::ZERO {
                self.point_drag = Some(PointDrag { index, pointer, delta, vertical: shift });
            }
            ctx.set_cursor_icon(if shift { egui::CursorIcon::ResizeVertical } else { egui::CursorIcon::Grabbing });
        }
        if self.dragging_map && delta != egui::Vec2::ZERO {
            self.pan_delta.0 += delta.x;
            self.pan_delta.1 += delta.y;
            self.following = false;
            ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        if !down {
            self.dragging_map = false;
            self.dragging_point = None;
        }

        self.hovered = None;
        if over_window || self.dragging_point.is_some() {
            return;
        }
        if point_under.is_some() && !self.dragging_map {
            ctx.set_cursor_icon(egui::CursorIcon::Grab);
            return;
        }
        let Some(at) = at else { return };
        let nearest = self.pickables.iter()
            .map(|(name, position)| (name, position.distance(at)))
            .min_by(|a, b| a.1.total_cmp(&b.1));
        self.hovered = nearest.filter(|(_, distance)| *distance <= PICK_RADIUS).map(|(name, _)| name.clone());
        if self.hovered.is_some() && !self.dragging_map {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if clicked {
            match (&self.hovered, nearest) {
                (Some(name), _) => println!("F10 map: click at ({:.0}, {:.0}) - selected '{name}'", at.x, at.y),
                (None, Some((name, distance))) => println!("F10 map: click at ({:.0}, {:.0}) - nothing within {PICK_RADIUS} px (nearest '{name}', {distance:.0} px away)", at.x, at.y),
                (None, None) => println!("F10 map: click at ({:.0}, {:.0}) - no aircraft on screen", at.x, at.y),
            }
            // A press on empty map doesn't clear it - it's the start of a drag.
            if let Some(name) = &self.hovered {
                self.selected = Some(name.clone());
            }
        }
    }

    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::Area::new(egui::Id::new("tactical_top_bar"))
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 12.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let can_center = self.selected.is_some();
                        let hint = match (can_center, self.following) {
                            (false, _) => "Select an aircraft first",
                            (true, false) => "Center on the selected aircraft and follow it",
                            (true, true) => "Following - click (or drag the map) to free the camera",
                        };
                        let center = crosshair_button(ui, can_center, self.following).on_hover_text(hint);
                        if center.clicked() {
                            self.following = !self.following;
                        }
                        ui.separator();
                        match &self.plan {
                            Some(plan) => {
                                ui.label(RichText::new(&plan.name).color(plan.color).strong());
                                ui.label(RichText::new(format!("{} · {}", plan.coalition, plan.plane)).weak());
                            }
                            None => {
                                ui.label(RichText::new("Click an aircraft to select it").weak());
                            }
                        }
                        ui.separator();
                        ui.label(RichText::new("Drag: move · right-drag: orbit · scroll: zoom · F10: close").small().weak());
                    });
                });
            });
    }

    /// The flight plan's point numbers (and altitudes), beside the points.
    fn point_labels(&self, ctx: &egui::Context) {
        let painter = ctx.layer_painter(egui::LayerId::background());
        for (position, text, color) in &self.labels {
            painter.text(*position + egui::vec2(9.0, -9.0), egui::Align2::LEFT_BOTTOM, text, egui::FontId::proportional(12.0), *color);
        }
    }

    /// Every aircraft, by coalition: click to select, double-click to
    /// center on it too; hovering a row rings it on the map.
    fn roster_window(&mut self, ctx: &egui::Context) {
        let mut select = None;
        let mut center = false;
        let mut hovered = None;
        egui::Window::new("Aircraft")
            .id(egui::Id::new("tactical_roster"))
            .anchor(egui::Align2::LEFT_TOP, egui::vec2(12.0, 60.0))
            .default_width(230.0)
            .resizable(false)
            .collapsible(true)
            .show(ctx, |ui| {
                if self.roster.is_empty() {
                    ui.label(RichText::new("No aircraft in the map").weak());
                }
                for coalition in &self.roster {
                    egui::CollapsingHeader::new(RichText::new(format!("{} ({})", coalition.name, coalition.aircraft.len())).color(coalition.color).strong())
                        .id_salt(("roster", &coalition.name))
                        .default_open(true)
                        .show(ui, |ui| {
                            for aircraft in &coalition.aircraft {
                                let selected = self.selected.as_deref() == Some(aircraft.name.as_str());
                                let mut text = RichText::new(&aircraft.name);
                                if aircraft.is_player {
                                    text = text.color(PLAYER_COLOR);
                                }
                                let response = ui.horizontal(|ui| {
                                    let row = ui.selectable_label(selected, text);
                                    let (state, color) = match aircraft.status {
                                        AircraftStatus::Flying => ("", ui.visuals().weak_text_color()),
                                        AircraftStatus::Destroyed => ("  destroyed", ERROR_COLOR),
                                        AircraftStatus::Missing => ("  not in scene", ERROR_COLOR),
                                    };
                                    ui.label(RichText::new(format!("{}{state}", aircraft.plane)).small().color(color));
                                    row
                                }).inner;
                                if response.hovered() {
                                    hovered = Some(aircraft.name.clone());
                                }
                                if response.clicked() {
                                    select = Some(aircraft.name.clone());
                                }
                                if response.double_clicked() {
                                    select = Some(aircraft.name.clone());
                                    center = true;
                                }
                            }
                        });
                }
                ui.label(RichText::new("Double-click: follow it").small().weak());
            });
        if hovered.is_some() {
            self.hovered = hovered;
        }
        if let Some(name) = select {
            self.selected = Some(name);
        }
        if center {
            self.following = true;
        }
    }

    /// The selected aircraft: what and how it is, and its flight plan - each
    /// point's position editable, points inserted or deleted. A change goes
    /// to `path_edit`.
    fn plan_window(&mut self, ctx: &egui::Context) {
        let Some(plan) = &self.plan else { return };
        let mut open = true;
        let mut points = plan.points.clone();
        let mut changed = false;
        egui::Window::new("Flight plan")
            .id(egui::Id::new("tactical_flight_plan"))
            .open(&mut open)
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 60.0))
            .default_width(420.0)
            .resizable(false)
            .collapsible(true)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&plan.name).color(plan.color).strong().size(15.0));
                    ui.label(RichText::new(if plan.is_player { "player" } else { "bot" }).small().weak());
                });
                ui.label(RichText::new(format!("{} coalition · {}", plan.coalition, plan.plane)).weak());
                match plan.live {
                    Some(live) if live.wrecked => {
                        ui.label(RichText::new("Destroyed").color(ERROR_COLOR));
                    }
                    Some(live) => {
                        ui.label(RichText::new(format!("Flying · {:.0} kt · {:.0} m", live.speed_kt, live.altitude)).color(OK_COLOR));
                    }
                    None => {
                        ui.label(RichText::new("Not in the scene").color(ERROR_COLOR));
                    }
                }
                if plan.is_player {
                    ui.label(RichText::new("The player flies it - the plan isn't followed while it's the player").small().weak());
                }
                ui.separator();

                if points.is_empty() {
                    ui.label(RichText::new("No flight plan yet - add its first point").weak());
                } else {
                    changed |= points_table(ui, &mut points, plan, &mut self.dragging_row);
                }

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui.button("+ Add point").on_hover_text("At the end - on from the last point").clicked() {
                        points.push(plan.next_suggestion);
                        changed = true;
                    }
                    let at_aircraft = plan.live.filter(|live| !live.wrecked).map(|live| (live.position.x, live.position.y, live.position.z));
                    if ui.add_enabled(at_aircraft.is_some(), egui::Button::new("+ At aircraft")).on_hover_text("At the end - where the aircraft is now").clicked() {
                        if let Some(point) = at_aircraft {
                            points.push(point);
                            changed = true;
                        }
                    }
                    if ui.add_enabled(!points.is_empty(), egui::Button::new("Clear")).clicked() {
                        points.clear();
                        changed = true;
                    }
                });
            });
        if changed {
            self.path_edit = Some((plan.name.clone(), points));
        }
        if !open {
            self.selected = None;
        }
    }
}

/// The flight plan's points as a table - a handle to drag the row up or
/// down the list (`dragging_row` - held between frames), #, x, z, altitude
/// (all draggable), the distance from the aircraft, and per point: insert a
/// point after it, delete it. Returns whether anything changed.
fn points_table(ui: &mut egui::Ui, points: &mut Vec<(f32, f32, f32)>, plan: &FlightPlan, dragging_row: &mut Option<usize>) -> bool {
    let mut changed = false;
    let mut insert_after = None;
    let mut remove = None;
    let mut dropped = false;
    // Each row's top and bottom on screen - where a dragged row would land.
    let mut rows: Vec<(f32, f32)> = Vec::with_capacity(points.len());
    let from = plan.live.map(|live| live.position);
    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
        let grid = egui::Grid::new("flight_plan_points").striped(true).num_columns(8).spacing([8.0, 4.0]).show(ui, |ui| {
            for header in ["", "#", "x", "z", "alt", "dist", "", ""] {
                ui.label(RichText::new(header).strong());
            }
            ui.end_row();
            for (index, point) in points.iter_mut().enumerate() {
                let handle = drag_handle(ui, *dragging_row == Some(index)).on_hover_text("Drag to move this point up or down the list");
                rows.push((handle.rect.top() - 2.0, handle.rect.bottom() + 2.0));
                if handle.drag_started() {
                    *dragging_row = Some(index);
                }
                if handle.drag_stopped() {
                    dropped = true;
                }
                let is_next = plan.next == Some(index);
                let number = if is_next { format!("{} >", index + 1) } else { (index + 1).to_string() };
                ui.label(if is_next { RichText::new(number).color(NEXT_POINT_COLOR).strong() } else { RichText::new(number) });
                changed |= ui.add(egui::DragValue::new(&mut point.0).speed(10.0).max_decimals(0)).changed();
                changed |= ui.add(egui::DragValue::new(&mut point.2).speed(10.0).max_decimals(0)).changed();
                changed |= ui.add(egui::DragValue::new(&mut point.1).speed(5.0).range(0.0..=20_000.0).suffix(" m").max_decimals(0)).changed();
                let distance = from.map(|from| (Vector3::new(point.0, point.1, point.2) - from).magnitude());
                ui.label(RichText::new(distance.map(format_distance).unwrap_or_else(|| "-".to_owned())).weak());
                if ui.small_button("+").on_hover_text("Insert a point after this one").clicked() {
                    insert_after = Some(index);
                }
                if ui.small_button("×").on_hover_text("Delete this point").clicked() {
                    remove = Some(index);
                }
                ui.end_row();
            }
        });

        // A row being dragged: a line where it would land; dropped there.
        if let (Some(from_row), Some(pointer)) = (*dragging_row, ui.input(|input| input.pointer.interact_pos())) {
            let landing = rows.iter().filter(|(top, bottom)| (top + bottom) * 0.5 < pointer.y).count();
            let y = match (landing.checked_sub(1).and_then(|row| rows.get(row)), rows.get(landing)) {
                (Some(above), Some(below)) => (above.1 + below.0) * 0.5,
                (Some(above), None) => above.1,
                (None, Some(below)) => below.0,
                (None, None) => pointer.y,
            };
            ui.painter().hline(grid.response.rect.x_range(), y, egui::Stroke::new(2.0, NEXT_POINT_COLOR));
            if dropped || !ui.input(|input| input.pointer.primary_down()) {
                *dragging_row = None;
                changed |= move_row(points, from_row, landing);
            }
        }
    });
    if let Some(index) = insert_after {
        // Halfway to the next point - or, after the last, where "+ Add
        // point" would put it.
        let point = match points.get(index + 1) {
            Some(next) => {
                let here = points[index];
                ((here.0 + next.0) * 0.5, (here.1 + next.1) * 0.5, (here.2 + next.2) * 0.5)
            }
            None => plan.next_suggestion,
        };
        points.insert(index + 1, point);
        changed = true;
    }
    if let Some(index) = remove {
        points.remove(index);
        changed = true;
    }
    if plan.next.is_some() {
        ui.label(RichText::new("> the point it's flying to").small().color(NEXT_POINT_COLOR));
    }
    changed
}

/// Moves the point at `from` to the gap `landing` (0 = before the first,
/// `points.len()` = after the last) - landing below its own place counts the
/// gap it leaves. Returns whether it moved.
fn move_row(points: &mut Vec<(f32, f32, f32)>, from: usize, landing: usize) -> bool {
    if from >= points.len() {
        return false;
    }
    let to = if landing > from { landing - 1 } else { landing };
    if to == from {
        return false;
    }
    let point = points.remove(from);
    points.insert(to.min(points.len()), point);
    true
}

/// A row's drag handle - three short lines (drawn - no font glyph
/// needed), lit while its row is being dragged.
fn drag_handle(ui: &mut egui::Ui, active: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(14.0, 18.0), egui::Sense::drag());
    let color = if active || response.hovered() { ui.visuals().strong_text_color() } else { ui.visuals().weak_text_color() };
    for offset in [-4.0, 0.0, 4.0] {
        let y = rect.center().y + offset;
        ui.painter().line_segment([egui::pos2(rect.left() + 2.0, y), egui::pos2(rect.right() - 2.0, y)], egui::Stroke::new(1.5, color));
    }
    response.on_hover_cursor(if active { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab })
}

/// "850 m", "12.4 km".
fn format_distance(meters: f32) -> String {
    if meters >= 1000.0 { format!("{:.1} km", meters / 1000.0) } else { format!("{meters:.0} m") }
}

/// A toggle button showing a crosshair (drawn - no font glyph needed) - lit
/// while `on`.
fn crosshair_button(ui: &mut egui::Ui, enabled: bool, on: bool) -> egui::Response {
    let size = egui::vec2(26.0, 22.0);
    let (rect, response) = ui.allocate_exact_size(size, if enabled { egui::Sense::click() } else { egui::Sense::hover() });
    let visuals = if enabled { ui.style().interact_selectable(&response, on) } else { ui.visuals().widgets.noninteractive };
    let painter = ui.painter();
    painter.rect(rect, 4.0, visuals.weak_bg_fill, visuals.bg_stroke, egui::StrokeKind::Inside);
    let color = if enabled { visuals.fg_stroke.color } else { ui.visuals().weak_text_color() };
    let stroke = egui::Stroke::new(1.5, color);
    let center = rect.center();
    painter.circle_stroke(center, 6.0, stroke);
    for (from, to) in [(egui::vec2(-9.0, 0.0), egui::vec2(-3.0, 0.0)), (egui::vec2(3.0, 0.0), egui::vec2(9.0, 0.0)), (egui::vec2(0.0, -9.0), egui::vec2(0.0, -3.0)), (egui::vec2(0.0, 3.0), egui::vec2(0.0, 9.0))] {
        painter.line_segment([center + from, center + to], stroke);
    }
    painter.circle_filled(center, 1.5, color);
    if enabled { response.on_hover_cursor(egui::CursorIcon::PointingHand) } else { response.on_hover_cursor(egui::CursorIcon::NotAllowed) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One egui frame fed the way the overlay pass feeds it (see
    /// render_pass.rs): the pointer's position every frame, a button event
    /// only when it happens.
    fn frame(ctx: &egui::Context, ui: &mut TacticalUi, at: Pos2, button: Option<bool>) {
        let mut events = vec![egui::Event::PointerMoved(at)];
        if let Some(pressed) = button {
            events.push(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
        }
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 720.0))),
            events,
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| ui.draw(ctx));
    }

    fn plan(name: &str, points: Vec<(f32, f32, f32)>) -> FlightPlan {
        FlightPlan { name: name.into(), coalition: "Blue".into(), color: Color32::WHITE, plane: "f16".into(), is_player: false, live: None, points, next: None, next_suggestion: (0.0, 1000.0, 9000.0) }
    }

    #[test]
    fn clicking_an_aircraft_selects_it() {
        let ctx = egui::Context::default();
        let mut ui = TacticalUi::default();
        let at = egui::pos2(600.0, 400.0);
        ui.pickables = vec![("bot".to_owned(), at)];
        frame(&ctx, &mut ui, at, None);
        frame(&ctx, &mut ui, at, Some(true));
        frame(&ctx, &mut ui, at, Some(false));
        frame(&ctx, &mut ui, at, None);
        assert_eq!(ui.selected.as_deref(), Some("bot"));
    }

    #[test]
    fn it_selects_on_the_press_however_long_the_button_is_held() {
        // At a few hundred frames a second a normal click spans dozens of
        // frames - it has to select as the button goes down.
        let ctx = egui::Context::default();
        let mut ui = TacticalUi::default();
        let at = egui::pos2(600.0, 400.0);
        ui.pickables = vec![("mq-9".to_owned(), at)];
        frame(&ctx, &mut ui, at, None);
        frame(&ctx, &mut ui, at, Some(true));
        assert_eq!(ui.selected.as_deref(), Some("mq-9"), "selected on the press");
        for _ in 0..200 {
            frame(&ctx, &mut ui, at, None);
        }
        frame(&ctx, &mut ui, at, Some(false));
        assert_eq!(ui.selected.as_deref(), Some("mq-9"));
    }

    #[test]
    fn dragging_the_map_pans_and_frees_the_camera_but_keeps_the_selection() {
        let ctx = egui::Context::default();
        let mut ui = TacticalUi::default();
        ui.selected = Some("mq-9".to_owned());
        ui.following = true;
        ui.pickables = vec![("mq-9".to_owned(), egui::pos2(900.0, 650.0))];
        let start = egui::pos2(600.0, 400.0);
        frame(&ctx, &mut ui, start, None);
        frame(&ctx, &mut ui, start, Some(true));
        assert_eq!(ui.selected.as_deref(), Some("mq-9"), "a press on empty map doesn't deselect");
        frame(&ctx, &mut ui, egui::pos2(650.0, 420.0), None);
        frame(&ctx, &mut ui, egui::pos2(700.0, 440.0), Some(false));
        assert_eq!(ui.pan_delta, (100.0, 40.0));
        assert!(!ui.following, "dragging frees the camera");
        assert_eq!(ui.selected.as_deref(), Some("mq-9"));
    }

    #[test]
    fn it_says_its_dragging_while_the_map_is_dragged_or_orbited() {
        let ctx = egui::Context::default();
        let mut ui = TacticalUi::default();
        let at = egui::pos2(600.0, 400.0);
        frame(&ctx, &mut ui, at, None);
        assert!(!ui.is_dragging());
        // Left: dragging the map.
        frame(&ctx, &mut ui, at, Some(true));
        assert!(ui.is_dragging());
        frame(&ctx, &mut ui, at, Some(false));
        assert!(!ui.is_dragging());
        // Right: orbiting.
        let right = |pressed: bool| egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 720.0))),
            events: vec![egui::Event::PointerMoved(at), egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() }],
            ..Default::default()
        };
        let _ = ctx.run(right(true), |ctx| ui.draw(ctx));
        assert!(ui.is_dragging(), "orbiting");
        let _ = ctx.run(right(false), |ctx| ui.draw(ctx));
        assert!(!ui.is_dragging());
    }

    #[test]
    fn rows_move_to_where_theyre_dropped() {
        let base = vec![(1.0, 0.0, 0.0), (2.0, 0.0, 0.0), (3.0, 0.0, 0.0), (4.0, 0.0, 0.0)];
        let order = |points: &Vec<(f32, f32, f32)>| points.iter().map(|point| point.0 as i32).collect::<Vec<_>>();
        // First to last.
        let mut points = base.clone();
        assert!(move_row(&mut points, 0, 4));
        assert_eq!(order(&points), [2, 3, 4, 1]);
        // Last to first.
        let mut points = base.clone();
        assert!(move_row(&mut points, 3, 0));
        assert_eq!(order(&points), [4, 1, 2, 3]);
        // Down one (into the gap after the next).
        let mut points = base.clone();
        assert!(move_row(&mut points, 1, 3));
        assert_eq!(order(&points), [1, 3, 2, 4]);
        // Dropped right where it was - either side of itself.
        let mut points = base.clone();
        assert!(!move_row(&mut points, 2, 2));
        assert!(!move_row(&mut points, 2, 3));
        assert_eq!(order(&points), [1, 2, 3, 4]);
    }

    #[test]
    fn pressing_a_path_point_drags_it_not_the_map() {
        let ctx = egui::Context::default();
        let mut ui = TacticalUi::default();
        ui.selected = Some("mq-9".to_owned());
        let point = egui::pos2(600.0, 400.0);
        ui.point_pickables = vec![(0, egui::pos2(300.0, 300.0)), (1, point)];
        frame(&ctx, &mut ui, point, None);
        frame(&ctx, &mut ui, point, Some(true));
        frame(&ctx, &mut ui, egui::pos2(630.0, 410.0), None);
        let drag = ui.point_drag.take().expect("the point is being dragged");
        assert_eq!(drag.index, 1);
        assert_eq!(drag.pointer, egui::pos2(630.0, 410.0));
        assert!(!drag.vertical);
        assert_eq!(ui.pan_delta, (0.0, 0.0), "the map doesn't move");
        assert_eq!(ui.selected.as_deref(), Some("mq-9"));
        frame(&ctx, &mut ui, egui::pos2(630.0, 410.0), Some(false));
        frame(&ctx, &mut ui, egui::pos2(660.0, 420.0), None);
        assert!(ui.point_drag.is_none(), "released - no more dragging");
    }

    #[test]
    fn a_quick_click_within_one_frame_still_selects() {
        // Down and up between two frames - both events in one.
        let ctx = egui::Context::default();
        let mut ui = TacticalUi::default();
        let at = egui::pos2(600.0, 400.0);
        ui.pickables = vec![("bot".to_owned(), at)];
        frame(&ctx, &mut ui, at, None);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 720.0))),
            events: vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() },
                egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() },
            ],
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| ui.draw(ctx));
        assert_eq!(ui.selected.as_deref(), Some("bot"));
    }

    #[test]
    fn clicking_works_with_the_labels_and_the_plan_window_up() {
        let ctx = egui::Context::default();
        let mut ui = TacticalUi::default();
        let at = egui::pos2(640.0, 400.0);
        ui.pickables = vec![("bot".to_owned(), at), ("player".to_owned(), egui::pos2(300.0, 300.0))];
        ui.labels = vec![(egui::pos2(500.0, 380.0), "1  1000 m".to_owned(), Color32::WHITE)];
        ui.selected = Some("player".to_owned());
        ui.plan = Some(plan("player", vec![(0.0, 1000.0, 0.0)]));
        for (button, position) in [(None, at), (Some(true), at), (Some(false), at), (None, at)] {
            frame(&ctx, &mut ui, position, button);
        }
        assert_eq!(ui.selected.as_deref(), Some("bot"));
    }

    #[test]
    fn an_aircraft_under_the_pointer_is_hovered() {
        let ctx = egui::Context::default();
        let mut ui = TacticalUi::default();
        ui.pickables = vec![("bot".to_owned(), egui::pos2(640.0, 400.0))];
        frame(&ctx, &mut ui, egui::pos2(645.0, 398.0), None);
        assert_eq!(ui.hovered.as_deref(), Some("bot"));
        frame(&ctx, &mut ui, egui::pos2(740.0, 400.0), None);
        assert_eq!(ui.hovered, None);
        assert_eq!(ui.selected, None, "hovering doesn't select");
    }

    #[test]
    fn the_points_table_changes_nothing_by_itself() {
        let ctx = egui::Context::default();
        let flight_plan = plan("bot", vec![(0.0, 1000.0, 0.0), (1000.0, 2000.0, 0.0)]);
        let mut points = flight_plan.points.clone();
        let input = egui::RawInput::default();
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                // Not clicking anything: nothing changes.
                assert!(!points_table(ui, &mut points, &flight_plan, &mut None));
            });
        });
        assert_eq!(points.len(), 2);
    }
}
