//! Two of the editor's floating windows:
//! - Paths: the map's named paths (play::map's `PathSpec`) - made, named,
//!   looped or not, smooth or not, deleted; picking one shows it on F10's
//!   map, its points edited in the Flight plan window and dragged on the
//!   map like an aircraft's.
//! - Pilot skills: how well a bot flies at each skill level (see
//!   play::pilot_skill) - changed live, saved to its file.

use egui::RichText;

use crate::game::scenes::play::map::{Difficulty, PathSpec};
use crate::game::scenes::play::pilot_skill::{self, PilotSkills, SKILLS_FILE};

use super::{FlightEditor, Selection, ERROR_COLOR, OK_COLOR};

/// A new path's first point is this high when nothing gives it a height (m).
const NEW_PATH_ALTITUDE: f32 = 1000.0;

impl FlightEditor {
    /// What follows the path `name`: aircraft (by default), map nodes (by
    /// default), and the animations that have something follow it.
    pub(super) fn path_users(&self, name: &str) -> Vec<String> {
        let mut users: Vec<String> = self.map.aircraft()
            .filter(|(_, aircraft)| aircraft.follow.as_deref() == Some(name))
            .map(|(_, aircraft)| aircraft.name.clone())
            .collect();
        users.extend(self.map.nodes.iter().filter(|node| node.follow.as_ref().is_some_and(|follow| follow.path == name)).map(|node| node.id.clone()));
        for (node, animations) in &self.map.animations {
            for animation in animations {
                if animation.paths.iter().any(|clip| clip.path == name) {
                    users.push(format!("{node} › {}", animation.name));
                }
            }
        }
        users
    }

    /// A new path, picked straight away: one point, at the selected node
    /// (or the editor's view) - add the rest in the Flight plan window.
    fn add_path(&mut self) {
        let name = (1..).map(|n| if n == 1 { "path".to_owned() } else { format!("path_{n}") })
            .find(|name| self.map.named_path(name).is_none())
            .unwrap();
        let at = self.selected_node()
            .and_then(|id| self.nodes.iter().find(|node| node.id == id))
            .map(|node| node.world_position)
            .unwrap_or_else(|| {
                let mut at = self.view_position;
                at.y = NEW_PATH_ALTITUDE;
                at
            });
        self.map.paths.push(PathSpec { name: name.clone(), points: vec![(at.x, at.y, at.z)], looped: true, smooth: false });
        self.select(Selection::Path(name));
    }

    /// Renames the path `old` - and everything that follows it by name.
    pub(super) fn rename_path(&mut self, old: &str, new: &str) {
        if new.is_empty() || self.map.named_path(new).is_some() {
            self.status = Some((format!("Can't rename the path to '{new}' - names must be unique and not empty"), true));
            return;
        }
        if let Some(path) = self.map.named_path_mut(old) {
            path.name = new.to_owned();
        }
        for aircraft in self.map.coalitions.iter_mut().flat_map(|coalition| coalition.aircraft.iter_mut()) {
            if aircraft.follow.as_deref() == Some(old) {
                aircraft.follow = Some(new.to_owned());
            }
        }
        for follow in self.map.nodes.iter_mut().filter_map(|node| node.follow.as_mut()) {
            if follow.path == old {
                follow.path = new.to_owned();
            }
        }
        for clip in self.map.animations.values_mut().flatten().flat_map(|animation| animation.paths.iter_mut()) {
            if clip.path == old {
                clip.path = new.to_owned();
            }
        }
        if self.selection == Selection::Path(old.to_owned()) {
            self.select(Selection::Path(new.to_owned()));
        }
    }

    /// Deletes the path `name` - aircraft on it go back to their own path,
    /// map nodes stop following it, animations' follows of it go.
    pub(super) fn delete_path(&mut self, name: &str) {
        self.map.paths.retain(|path| path.name != name);
        for aircraft in self.map.coalitions.iter_mut().flat_map(|coalition| coalition.aircraft.iter_mut()) {
            if aircraft.follow.as_deref() == Some(name) {
                aircraft.follow = None;
            }
        }
        for node in self.map.nodes.iter_mut() {
            if node.follow.as_ref().is_some_and(|follow| follow.path == name) {
                node.follow = None;
            }
        }
        for animation in self.map.animations.values_mut().flatten() {
            animation.paths.retain(|clip| clip.path != name);
        }
        if self.selection == Selection::Path(name.to_owned()) {
            self.select(Selection::None);
        }
        self.timeline_selection = Default::default();
    }

    /// The Paths window (over `area`'s top left to start with), while it's
    /// open: every path - picking one opens it in Properties (its options and
    /// points) and shows it on the map.
    pub(super) fn paths_window(&mut self, ctx: &egui::Context, area: egui::Rect) {
        if !self.paths_open {
            return;
        }
        let mut open = true;
        let mut pick = None;
        egui::Window::new("Paths")
            .id(egui::Id::new("editor_paths"))
            .open(&mut open)
            .default_pos(area.left_top() + egui::vec2(290.0, 56.0))
            .default_size(egui::vec2(240.0, 260.0))
            .resizable(true)
            .collapsible(true)
            .show(ctx, |ui| {
                if ui.add(egui::Button::new(RichText::new("+ Path").strong()).min_size(egui::vec2(ui.available_width(), 26.0)))
                    .on_hover_text("A new path - its first point at the selected node, or the editor's view").clicked() {
                    self.add_path();
                }
                ui.separator();
                if self.map.paths.is_empty() {
                    ui.label(RichText::new("No paths yet").weak());
                }
                egui::ScrollArea::vertical().id_salt("paths_list").show(ui, |ui| {
                    for path in &self.map.paths {
                        let picked = self.selected_path.as_deref() == Some(path.name.as_str());
                        let users = self.path_users(&path.name).len();
                        let text = format!("{}   {} pts{}", path.name, path.points.len(), if users > 0 { format!(" · {users} using it") } else { String::new() });
                        if ui.selectable_label(picked, text).clicked() {
                            pick = Some(path.name.clone());
                        }
                    }
                });
                ui.label(RichText::new("Pick one to edit it in Properties and see it on the map").small().weak());
            });
        if let Some(name) = pick {
            self.select(Selection::Path(name));
        }
        if !open {
            self.paths_open = false;
        }
    }

    /// The Pilot skills window, while it's open: every skill level's
    /// numbers - changed live (bots fly differently straight away), saved
    /// to their file with Save.
    pub(super) fn skills_window(&mut self, ctx: &egui::Context, area: egui::Rect) {
        if !self.skills_open {
            return;
        }
        let mut skills = pilot_skill::skills();
        let before = skills;
        let mut open = true;
        egui::Window::new("Pilot skills")
            .id(egui::Id::new("editor_pilot_skills"))
            .open(&mut open)
            .default_pos(area.left_top() + egui::vec2(16.0, 380.0))
            .default_width(420.0)
            .resizable(false)
            .collapsible(true)
            .show(ctx, |ui| {
                ui.label(RichText::new("How well a bot flies at each level - everything a pilot does reads these. Changes apply straight away.").small().weak());
                egui::Grid::new("pilot_skills_grid").striped(true).num_columns(4).spacing([10.0, 4.0]).show(ui, |ui| {
                    ui.label("");
                    for difficulty in Difficulty::ALL {
                        ui.label(RichText::new(difficulty.label()).strong());
                    }
                    ui.end_row();
                    let row = |ui: &mut egui::Ui, skills: &mut PilotSkills, label: &str, hint: &str, edit: &dyn Fn(&mut egui::Ui, &mut pilot_skill::PilotSkill)| {
                        ui.label(label).on_hover_text(hint);
                        for difficulty in Difficulty::ALL {
                            edit(ui, skills.get_mut(difficulty));
                        }
                        ui.end_row();
                    };
                    row(ui, &mut skills, "Max G", "The most G it pulls", &|ui, skill| { ui.add(egui::DragValue::new(&mut skill.max_g).speed(0.1).range(1.0..=12.0).suffix(" g")); });
                    row(ui, &mut skills, "Max AoA", "The most angle of attack it flies", &|ui, skill| { ui.add(egui::DragValue::new(&mut skill.max_aoa_deg).speed(0.2).range(2.0..=40.0).suffix("°")); });
                    row(ui, &mut skills, "Max bank", "Its steepest bank in a turn", &|ui, skill| { ui.add(egui::DragValue::new(&mut skill.max_bank_deg).speed(0.5).range(10.0..=180.0).suffix("°")); });
                    row(ui, &mut skills, "Turn eagerness", "Degrees of bank per degree it's off its heading", &|ui, skill| { ui.add(egui::DragValue::new(&mut skill.turn_eagerness).speed(0.05).range(0.2..=6.0)); });
                    row(ui, &mut skills, "Corner lead", "How far from a path's point it turns for the next - small: swings wide; big: cuts in", &|ui, skill| { ui.add(egui::DragValue::new(&mut skill.corner_lead_m).speed(5.0).range(50.0..=3000.0).suffix(" m")); });
                    row(ui, &mut skills, "Precision", "0..1 - how exactly it holds heading, height and speed", &|ui, skill| { ui.add(egui::DragValue::new(&mut skill.precision).speed(0.01).range(0.0..=1.0)); });
                    row(ui, &mut skills, "Hands", "How fast it moves the stick and throttle (per second)", &|ui, skill| { ui.add(egui::DragValue::new(&mut skill.control_rate).speed(0.1).range(0.5..=20.0)); });
                    row(ui, &mut skills, "Cruise speed", "Its speed when the speed's left to it (By skill)", &|ui, skill| { ui.add(egui::DragValue::new(&mut skill.cruise_kt).speed(1.0).range(50.0..=1500.0).suffix(" kt")); });
                    row(ui, &mut skills, "Max performance", "With its speed left to it: full throttle", &|ui, skill| { ui.checkbox(&mut skill.max_performance, ""); });
                    row(ui, &mut skills, "Reaction", "How long until it notices a missile", &|ui, skill| { ui.add(egui::DragValue::new(&mut skill.reaction_seconds).speed(0.05).range(0.0..=10.0).suffix(" s")); });
                    row(ui, &mut skills, "Flares again", "How often it bursts flares again while a missile's coming - off: once", &|ui, skill| {
                        let mut again = skill.flare_burst_every.is_some();
                        if ui.checkbox(&mut again, "").changed() {
                            skill.flare_burst_every = again.then_some(skill.flare_burst_every.unwrap_or(3.0));
                        }
                        if let Some(every) = &mut skill.flare_burst_every {
                            ui.add(egui::DragValue::new(every).speed(0.1).range(0.2..=30.0).suffix(" s"));
                        }
                    });
                });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button(format!("Save to {SKILLS_FILE}")).clicked() {
                        self.skills_status = Some(match skills.save() {
                            Ok(()) => (format!("Saved to {SKILLS_FILE}"), false),
                            Err(error) => (error, true),
                        });
                    }
                    if ui.button("Reload file").clicked() {
                        skills = PilotSkills::load();
                    }
                    if ui.button("Built-in values").clicked() {
                        skills = PilotSkills::default();
                    }
                });
                if let Some((message, error)) = &self.skills_status {
                    ui.label(RichText::new(message).color(if *error { ERROR_COLOR } else { OK_COLOR }));
                }
            });
        if skills != before {
            pilot_skill::set_skills(skills);
        }
        if !open {
            self.skills_open = false;
        }
    }
}
