//! Flight plans in the editor's Properties: the selected aircraft's - a big
//! "Create path" button (a map path made from what it flies now, others can
//! fly it too), which path it flies, the path's options and its points - and
//! a path picked in the Paths window, the same way. The points are an
//! expandable list: each one's x, z and altitude, a handle to drag it up or
//! down the list, a point inserted after it or it deleted; points added at
//! the end or where the aircraft is. They're dragged on F10's map too.

use egui::RichText;
use nalgebra::Vector3;

use crate::game::scenes::play::map::PathSpec;
use crate::game::scenes::play::tactical_ui::points_table;

use super::{inspector_header, next_path_point, padded, property, section, FlightEditor, Icon, PATH_STEP};

/// Height of the big "Create path" button (points).
const CREATE_BUTTON_HEIGHT: f32 = 34.0;

/// Where a point added after `points`' last goes with no aircraft to go by:
/// on the way the last leg went - or, without one, ahead of `fallback`.
fn point_after(points: &[(f32, f32, f32)], fallback: Vector3<f32>) -> (f32, f32, f32) {
    let point = |&(x, y, z): &(f32, f32, f32)| Vector3::new(x, y, z);
    let next = match points {
        [] => fallback,
        [only] => point(only) + Vector3::z() * PATH_STEP,
        [.., before, last] => point(last) + (point(last) - point(before)).try_normalize(1e-3).unwrap_or_else(Vector3::z) * PATH_STEP,
    };
    (next.x, next.y, next.z)
}

/// A path's own options: its name (renamed once the field's left - into
/// `rename`), looped, smooth, and what uses it.
fn path_options(ui: &mut egui::Ui, path: &mut PathSpec, name_edit: &mut Option<String>, users: &[String], rename: &mut Option<(String, String)>) {
    property(ui, "Path name", |ui| {
        let buffer = name_edit.get_or_insert_with(|| path.name.clone());
        let response = ui.add_sized([ui.available_width(), 20.0], egui::TextEdit::singleline(buffer));
        if response.lost_focus() {
            let new_name = buffer.trim().to_owned();
            *name_edit = None;
            if new_name != path.name {
                *rename = Some((path.name.clone(), new_name));
            }
        } else if !response.has_focus() {
            *buffer = path.name.clone();
        }
    });
    property(ui, "Looped", |ui| {
        ui.checkbox(&mut path.looped, "").on_hover_text("Round and round, back to the first point - off: once, then on straight (flown) or held at the end (animated)");
    });
    property(ui, "Smooth", |ui| {
        ui.checkbox(&mut path.smooth, "").on_hover_text("Followed as a curve through its points (animated follows - a flying pilot makes its own curves)");
    });
    property(ui, "Used by", |ui| {
        if users.is_empty() {
            ui.label(RichText::new("nothing yet").weak());
        } else {
            ui.label(users.join(", "));
        }
    });
}

/// The points as an expandable list (see tactical_ui::points_table) with
/// "+ Add point", "+ At aircraft" (with `at_aircraft`) and "Clear" under
/// it. Returns whether anything changed.
#[allow(clippy::too_many_arguments)]
fn points_list(ui: &mut egui::Ui, id: &str, points: &mut Vec<(f32, f32, f32)>, from: Option<Vector3<f32>>, next: Option<usize>, suggestion: (f32, f32, f32), at_aircraft: Option<(f32, f32, f32)>, dragging: &mut Option<usize>) -> bool {
    let mut changed = false;
    egui::CollapsingHeader::new(RichText::new(format!("Points ({})", points.len())).strong())
        .id_salt((id, "points_list"))
        .default_open(true)
        .show_unindented(ui, |ui| {
            if points.is_empty() {
                ui.label(RichText::new("No points yet - add the first").weak());
            } else {
                changed |= points_table(ui, id, points, from, next, suggestion, dragging);
            }
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                if ui.button("+ Add point").on_hover_text("At the end - on from the last point").clicked() {
                    points.push(suggestion);
                    changed = true;
                }
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
            ui.label(RichText::new("Drag a row's handle to reorder - points can also be dragged on the map (Shift: up and down)").small().weak());
        });
    changed
}

/// The big button.
fn create_button(ui: &mut egui::Ui, hint: &str) -> bool {
    ui.add(egui::Button::new(RichText::new("Create path").strong().size(15.0)).min_size(egui::vec2(ui.available_width(), CREATE_BUTTON_HEIGHT)))
        .on_hover_text(hint)
        .clicked()
}

impl FlightEditor {
    fn unique_path_name(&self, base: &str) -> String {
        (1..).map(|n| if n == 1 { base.to_owned() } else { format!("{base}_{n}") })
            .find(|name| self.map.named_path(name).is_none())
            .unwrap()
    }

    /// A new map path for the aircraft `name` to fly - its points what it
    /// flies now (or, with nothing, one ahead of its nose).
    fn create_path_for(&mut self, name: &str) {
        let Some((_, placement)) = self.map.find(name) else { return };
        let placement = placement.clone();
        let (points, looped) = self.map.flight_plan(&placement);
        let live = self.live.get(name).copied();
        let points = if points.is_empty() { vec![next_path_point(&placement, &[], live)] } else { points };
        let path_name = self.unique_path_name(&format!("{name}_path"));
        self.map.paths.push(PathSpec { name: path_name.clone(), points, looped, smooth: false });
        if let Some(placement) = self.map.find_mut(name) {
            placement.follow = Some(path_name);
        }
        self.path_name_edit = None;
    }

    /// The selected aircraft's flight plan, in its inspector - see the
    /// module's doc comment.
    pub(super) fn flight_plan_section(&mut self, ui: &mut egui::Ui, name: &str) {
        let Some((_, placement)) = self.map.find(name) else { return };
        let placement = placement.clone();
        let live = self.live.get(name).copied();
        let follow = placement.follow.clone().filter(|path| self.map.named_path(path).is_some());
        let paths: Vec<(String, usize)> = self.map.paths.iter().map(|path| (path.name.clone(), path.points.len())).collect();
        let (mut points, _) = self.map.flight_plan(&placement);
        let before = points.clone();
        let mut options = follow.as_ref().and_then(|path| self.map.named_path(path)).cloned();
        let options_before = options.clone();
        let users = follow.as_ref().map(|path| self.path_users(path)).unwrap_or_default();
        let from = live.map(|live| live.position);
        let next = live.and_then(|live| live.next_path_point);
        let suggestion = next_path_point(&placement, &points, live);
        let at_aircraft = live.filter(|live| !live.wrecked).map(|live| (live.position.x, live.position.y, live.position.z));

        let mut create = false;
        let mut flies = placement.follow.clone();
        let mut rename = None;
        let dragging = &mut self.points_dragging;
        let name_edit = &mut self.path_name_edit;
        section(ui, "Flight plan", |ui| {
            if create_button(ui, "A new map path for it to fly, from what it flies now - other aircraft and animations can use it too") {
                create = true;
            }
            ui.add_space(4.0);
            property(ui, "Flies", |ui| {
                let own = format!("its own path ({} points)", placement.path.len());
                egui::ComboBox::from_id_salt("flight_plan_flies")
                    .width(ui.available_width())
                    .selected_text(follow.clone().unwrap_or_else(|| own.clone()))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut flies, None, own.as_str());
                        for (path, count) in &paths {
                            ui.selectable_value(&mut flies, Some(path.clone()), format!("{path} ({count} points)"));
                        }
                    })
                    .response
                    .on_hover_text("What it flies by default - a bot flies it; animations can have it follow others");
            });
            match &mut options {
                Some(path) => path_options(ui, path, name_edit, &users, &mut rename),
                None => property(ui, "", |ui| {
                    ui.label(RichText::new("A path of its own - only it flies it. Create path makes a map path of it.").small().weak());
                }),
            }
            ui.add_space(2.0);
            points_list(ui, "aircraft_flight_plan", &mut points, from, next, suggestion, at_aircraft, dragging);
        });

        if create {
            self.create_path_for(name);
            return;
        }
        if flies != placement.follow {
            if let Some(placement) = self.map.find_mut(name) {
                placement.follow = flies;
            }
            self.path_name_edit = None;
            return;
        }
        if points != before {
            self.map.set_flight_plan(name, points);
        }
        if let (Some(options), Some(path)) = (options, follow.as_ref()) {
            if Some(&options) != options_before.as_ref() {
                if let Some(spec) = self.map.named_path_mut(path) {
                    spec.looped = options.looped;
                    spec.smooth = options.smooth;
                }
            }
        }
        if let Some((old, new)) = rename {
            self.rename_path(&old, &new);
        }
    }

    /// A path picked in the Paths window: its options, its points, and
    /// deleting it.
    pub(super) fn path_inspector(&mut self, ui: &mut egui::Ui, name: &str) {
        let Some(spec) = self.map.named_path(name).cloned() else { return };
        let users = self.path_users(name);
        inspector_header(ui, Icon::Path, ui.visuals().text_color(), name, |ui| {
            let text = format!("{} points · {}{}", spec.points.len(), if spec.looped { "looped" } else { "once" }, if spec.smooth { " · smooth" } else { "" });
            ui.label(RichText::new(text).small().weak());
        }, |_| {});

        let mut edited = spec.clone();
        let mut rename = None;
        let suggestion = point_after(&spec.points, Vector3::new(self.view_position.x, 1000.0, self.view_position.z));
        let dragging = &mut self.points_dragging;
        let name_edit = &mut self.path_name_edit;
        section(ui, "Path", |ui| {
            path_options(ui, &mut edited, name_edit, &users, &mut rename);
            ui.add_space(2.0);
            points_list(ui, "picked_path", &mut edited.points, None, None, suggestion, None, dragging);
        });
        let mut delete = false;
        padded(ui, |ui| {
            if ui.button("Delete path").on_hover_text("Aircraft on it go back to their own path; follows of it in animations go").clicked() {
                delete = true;
            }
        });

        if delete {
            self.delete_path(name);
            return;
        }
        if edited != spec {
            if let Some(path) = self.map.named_path_mut(name) {
                path.points = edited.points;
                path.looped = edited.looped;
                path.smooth = edited.smooth;
            }
        }
        if let Some((old, new)) = rename {
            self.rename_path(&old, &new);
        }
    }
}
