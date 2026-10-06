//! The editor's timeline - one node's animation (play::map's
//! `AnimationDef`), picked or made from the node's Animations in
//! Properties: its name, length, whether it plays on start and loops;
//! played, paused, stopped and scrubbed; and laid out against time:
//! - path bars - the node follows a map path from a bar's start to its end:
//!   grab its left edge to move when it starts, its right edge when it ends
//!   (a positioned one gets there sooner or later), its middle to move it;
//! - camera keys - each a whole camera setup (where, which way, its lens,
//!   what it follows and looks at), taken from the editor camera: a diamond
//!   cuts to it, a circle moves smoothly to it from the key before;
//! - events - audio, subtitles, starting or stopping an animation.
//! Clicking one shows it in Properties (see `timeline_item_inspector`),
//! where it's edited on the spot.

use egui::{Color32, RichText};
use nalgebra::Vector3;

use crate::engine::scene_manager::properties::{degrees_from_rotation, rotation_from_degrees, Transform3D};
use crate::game::scenes::play::map::{AnimAction, AnimEvent, AnimationDef, CameraKey, Difficulty, FollowMode, PathClip, Transition, WaveOn};
use crate::game::scenes::play::path_follow::{corner_distances, SampledPath};

use super::{inspector_header, padded, property, section, EditorRequest, FlightEditor, Icon, NodeKind, Selection, TimelineRequest, PAD};

/// What's picked on the timeline, in the animation shown - edited in
/// Properties.
#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub enum TimelineSelection {
    #[default]
    None,
    /// A path bar, by index.
    Path(usize),
    /// A camera key, by index.
    Camera(usize),
    /// An event, by index.
    Event(usize),
}

/// The names column's width, a row's height and the ruler's (points).
const LABEL_COLUMN: f32 = 120.0;
const ROW_HEIGHT: f32 = 26.0;
const RULER_HEIGHT: f32 = 22.0;
/// A key's half size, how close a click has to be to grab one, and how wide
/// a bar's edge grabs are (points).
const KEY_SIZE: f32 = 6.0;
const KEY_GRAB: f32 = 8.0;
const EDGE_GRAB: f32 = 7.0;
/// Times snap to this (s) when dragged.
const SNAP: f32 = 0.01;
/// A new path bar's length (s).
const NEW_PATH_SECONDS: f32 = 5.0;
/// Knots to m/s.
const KT_TO_MS: f32 = 0.514_444;

const PLAYHEAD_COLOR: Color32 = Color32::from_rgb(255, 205, 40);
const PATH_COLOR: Color32 = Color32::from_rgb(200, 140, 255);
const CAMERA_COLOR: Color32 = Color32::from_rgb(120, 210, 140);
const EVENT_COLOR: Color32 = Color32::from_rgb(230, 150, 90);
const SELECTED_COLOR: Color32 = Color32::WHITE;

fn snap(time: f32) -> f32 {
    (time / SNAP).round() * SNAP
}

fn tuple(vector: Vector3<f32>) -> (f32, f32, f32) {
    (vector.x, vector.y, vector.z)
}

/// What dragging a path bar moves.
#[derive(Clone, Copy, PartialEq)]
enum BarPart {
    Start,
    End,
    Whole,
}

/// A drag on the timeline this frame - applied after drawing.
enum Drag {
    Bar { index: usize, part: BarPart, time: f32, delta: f32 },
    Key { index: usize, time: f32 },
    Event { index: usize, time: f32 },
}

impl FlightEditor {
    /// The animation on the timeline.
    pub(super) fn current(&self) -> Option<&AnimationDef> {
        let (node, name) = self.current_animation.as_ref()?;
        self.map.animation(node, name)
    }

    fn current_mut(&mut self) -> Option<&mut AnimationDef> {
        let (node, name) = self.current_animation.clone()?;
        self.map.animation_mut(&node, &name)
    }

    fn current_request(&self, make: impl FnOnce(String, String) -> TimelineRequest) -> Option<EditorRequest> {
        let (node, name) = self.current_animation.clone()?;
        Some(EditorRequest::Timeline(make(node, name)))
    }

    fn timeline_request(&mut self, make: impl FnOnce(String, String) -> TimelineRequest) {
        if let Some(request) = self.current_request(make) {
            self.requests.push(request);
        }
    }

    /// Picks `item` on the timeline - shown in Properties - with the
    /// playhead brought to it.
    fn pick(&mut self, item: TimelineSelection, at: f32) {
        self.timeline_selection = item;
        self.selection = Selection::TimelineItem;
        self.timeline_request(|node, name| TimelineRequest::Seek(node, name, at));
        if matches!(item, TimelineSelection::Camera(_)) {
            self.timeline_preview = true;
        }
    }

    /// A new animation for node `node`, shown on the timeline.
    pub(super) fn create_animation(&mut self, node: &str) {
        let taken: Vec<String> = self.map.animations.get(node).map(|list| list.iter().map(|animation| animation.name.clone()).collect()).unwrap_or_default();
        let name = (1..).map(|n| if n == 1 { "animation".to_owned() } else { format!("animation_{n}") })
            .find(|name| !taken.contains(name))
            .unwrap();
        self.map.animations.entry(node.to_owned()).or_default().push(AnimationDef::new(name.clone()));
        self.show_animation(node, &name);
    }

    /// Node `node`'s animation `name` on the timeline.
    pub(super) fn show_animation(&mut self, node: &str, name: &str) {
        self.current_animation = Some((node.to_owned(), name.to_owned()));
        self.timeline_selection = TimelineSelection::None;
        self.animation_name_edit = None;
    }

    /// Renames the animation shown - events that start or stop it follow.
    fn rename_current(&mut self, new: &str) {
        let Some((node, old)) = self.current_animation.clone() else { return };
        let taken = self.map.animation(&node, new).is_some();
        if new.is_empty() || taken {
            self.status = Some((format!("Can't rename the animation to '{new}' - names must be unique for the node and not empty"), true));
            return;
        }
        if let Some(animation) = self.map.animation_mut(&node, &old) {
            animation.name = new.to_owned();
        }
        for event in self.map.animations.values_mut().flatten().flat_map(|animation| animation.events.iter_mut()) {
            match &mut event.action {
                AnimAction::StartAnimation { node: of, animation } | AnimAction::StopAnimation { node: of, animation } if *of == node && *animation == old => *animation = new.to_owned(),
                _ => {}
            }
        }
        self.current_animation = Some((node, new.to_owned()));
    }

    /// Deletes the animation shown - events that start or stop it go.
    fn delete_current(&mut self) {
        let Some((node, name)) = self.current_animation.clone() else { return };
        self.timeline_request(|node, name| TimelineRequest::Stop(node, name));
        if let Some(list) = self.map.animations.get_mut(&node) {
            list.retain(|animation| animation.name != name);
            if list.is_empty() {
                self.map.animations.remove(&node);
            }
        }
        for animation in self.map.animations.values_mut().flatten() {
            animation.events.retain(|event| !matches!(&event.action, AnimAction::StartAnimation { node: of, animation } | AnimAction::StopAnimation { node: of, animation } if *of == node && *animation == name));
        }
        self.current_animation = None;
        self.timeline_selection = TimelineSelection::None;
        if self.selection == Selection::TimelineItem {
            self.select(Selection::None);
        }
    }

    /// The editor camera's pose as a camera key's: an offset from `follow`
    /// (in its frame when `follow_rotation`), the world's otherwise.
    fn view_as_key_pose(&self, follow: Option<&str>, follow_rotation: bool) -> ((f32, f32, f32), (f32, f32, f32)) {
        let world = Transform3D { position: self.view_position, rotation: self.view_rotation.unwrap_or_default(), scale: Vector3::new(1.0, 1.0, 1.0) };
        let placed = match follow.and_then(|id| self.node_world(id)) {
            Some(parent) => parent.relative(&world, follow_rotation),
            None => world,
        };
        (tuple(placed.position), tuple(degrees_from_rotation(&placed.rotation)))
    }

    fn node_world(&self, id: &str) -> Option<Transform3D> {
        let info = self.nodes.iter().find(|node| node.id == id)?;
        Some(Transform3D { position: info.world_position, rotation: rotation_from_degrees(info.world_rotation), scale: Vector3::new(1.0, 1.0, 1.0) })
    }

    /// A camera key at the playhead, from the editor camera - picked.
    fn add_camera_key(&mut self) {
        let time = snap(self.playback.time);
        let (position, rotation) = self.view_as_key_pose(None, true);
        let fov = if self.view_fov > 1.0 { self.view_fov } else { 60.0 };
        let Some(animation) = self.current_mut() else { return };
        animation.camera.retain(|key| (key.time - time).abs() >= SNAP * 0.5);
        animation.camera.push(CameraKey { time, transition: Transition::Cut, position, rotation, fov, follow: None, follow_rotation: true, look_at: None, wave: false, wave_amount: 5.0, wave_speed: 1.0, wave_on: WaveOn::Position });
        animation.sort();
        let index = animation.camera.iter().position(|key| key.time == time).unwrap_or(0);
        self.pick(TimelineSelection::Camera(index), time);
    }

    /// When along path bar `clip` (s into the animation) it reaches each of
    /// its path's points - and, for a flown path that doesn't loop, the end
    /// of the path (`Some`, if that's within the bar). A positioned bar
    /// spreads the path over itself; a flown one goes at its flying speed
    /// (its set speed, or its pilot's), round again for a looped path.
    fn bar_marks(&self, node: &str, clip: &PathClip) -> (Vec<f32>, Option<f32>) {
        let Some(path) = self.map.named_path(&clip.path) else { return (Vec::new(), None) };
        let (distances, total) = corner_distances(&path.points, path.looped);
        if total <= 0.0 {
            return (vec![clip.start], None);
        }
        match clip.mode {
            FollowMode::Positioned => {
                let span = clip.end - clip.start;
                (distances.iter().map(|along| clip.start + along / total * span).collect(), None)
            }
            FollowMode::Flown => {
                let skill = clip.skill.or_else(|| self.map.find(node).map(|(_, placement)| placement.difficulty)).unwrap_or_default().skill();
                let speed = clip.speed_kt.unwrap_or(skill.cruise_kt) * KT_TO_MS;
                if speed <= 0.01 {
                    return (vec![clip.start], None);
                }
                let mut marks = Vec::new();
                let mut lap = 0.0;
                'laps: loop {
                    for along in &distances {
                        let at = clip.start + (lap + along) / speed;
                        if at > clip.end || marks.len() > 500 {
                            break 'laps;
                        }
                        marks.push(at);
                    }
                    if !path.looped {
                        break;
                    }
                    lap += total;
                }
                let end = (!path.looped).then(|| clip.start + total / speed).filter(|end| *end <= clip.end);
                (marks, end)
            }
        }
    }

    /// A path bar at the playhead following `path` - flown for an aircraft,
    /// positioned for anything else - picked.
    fn add_path_bar(&mut self, path: &str) {
        let start = snap(self.playback.time);
        let aircraft = self.current_animation.as_ref().is_some_and(|(node, _)| self.map.find(node).is_some());
        let Some(animation) = self.current_mut() else { return };
        let end = start + NEW_PATH_SECONDS;
        animation.length = animation.length.max(end);
        animation.paths.push(PathClip { start, end, path: path.to_owned(), mode: if aircraft { FollowMode::Flown } else { FollowMode::Positioned }, speed_kt: None, skill: None, orient: true });
        animation.sort();
        let index = animation.paths.iter().position(|clip| clip.start == start && clip.path == path).unwrap_or(0);
        self.pick(TimelineSelection::Path(index), start);
    }

    fn add_event(&mut self, action: AnimAction) {
        let time = snap(self.playback.time);
        let Some(animation) = self.current_mut() else { return };
        animation.events.push(AnimEvent { time, action: action.clone() });
        animation.sort();
        let index = animation.events.iter().position(|event| event.time == time && event.action == action).unwrap_or(0);
        self.pick(TimelineSelection::Event(index), time);
    }

    /// The timeline: the animation's settings, its transport, then the
    /// ruler and its rows.
    pub(super) fn timeline(&mut self, ui: &mut egui::Ui) {
        if self.current().is_none() {
            self.current_animation = None;
            let selected = self.selected_node().map(|id| id.to_owned());
            let mut create = false;
            padded(ui, |ui| {
                match &selected {
                    Some(id) => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("{id} has no animation")).weak());
                            if ui.button("Create Animation").clicked() {
                                create = true;
                            }
                        });
                    }
                    None => {
                        ui.label(RichText::new("Select a node or an aircraft to see its animations here.").weak());
                    }
                }
            });
            if create {
                if let Some(id) = selected {
                    self.create_animation(&id);
                }
            }
            return;
        }
        // Long enough for what's in it.
        if let Some(animation) = self.current_mut() {
            let end = animation.paths.iter().map(|clip| clip.end)
                .chain(animation.camera.iter().map(|key| key.time))
                .chain(animation.events.iter().map(|event| event.time))
                .fold(0.0_f32, f32::max);
            if animation.length < end {
                animation.length = end;
            }
        }
        if let TimelineSelection::Path(index) | TimelineSelection::Camera(index) | TimelineSelection::Event(index) = self.timeline_selection {
            let gone = self.current().is_none_or(|animation| match self.timeline_selection {
                TimelineSelection::Path(_) => index >= animation.paths.len(),
                TimelineSelection::Camera(_) => index >= animation.camera.len(),
                _ => index >= animation.events.len(),
            });
            if gone {
                self.timeline_selection = TimelineSelection::None;
            }
        }
        padded(ui, |ui| self.animation_bar(ui));
        padded(ui, |ui| self.transport(ui));
        ui.separator();
        self.rows(ui);
    }

    /// The animation's own settings.
    fn animation_bar(&mut self, ui: &mut egui::Ui) {
        let Some((node, name)) = self.current_animation.clone() else { return };
        let names: Vec<String> = self.map.animations.get(&node).map(|list| list.iter().map(|animation| animation.name.clone()).collect()).unwrap_or_default();
        let mut rename = None;
        let mut delete = false;
        let mut picked = None;
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(format!("{node} ›")).strong());
            if names.len() > 1 {
                egui::ComboBox::from_id_salt("timeline_animations")
                    .selected_text(&name)
                    .show_ui(ui, |ui| {
                        for candidate in &names {
                            if ui.selectable_label(candidate == &name, candidate).clicked() {
                                picked = Some(candidate.clone());
                            }
                        }
                    })
                    .response
                    .on_hover_text("This node's animations");
            }
            let buffer = self.animation_name_edit.get_or_insert_with(|| name.clone());
            let response = ui.add_sized([140.0, 20.0], egui::TextEdit::singleline(buffer)).on_hover_text("Its name - Enter to rename");
            if response.lost_focus() {
                let new_name = buffer.trim().to_owned();
                self.animation_name_edit = None;
                if new_name != name {
                    rename = Some(new_name);
                }
            } else if !response.has_focus() {
                *buffer = name.clone();
            }
            let Some(animation) = self.map.animation_mut(&node, &name) else { return };
            ui.label("Length");
            ui.add(egui::DragValue::new(&mut animation.length).speed(0.1).range(0.5..=3600.0).suffix(" s").max_decimals(2));
            ui.checkbox(&mut animation.on_start, "On start").on_hover_text("Plays as soon as the level starts");
            ui.checkbox(&mut animation.looped, "Loop").on_hover_text("Starts over when it ends");
            ui.checkbox(&mut animation.lock_input, "Lock input").on_hover_text("The player has no controls while it plays (and feels no G) - back the moment it ends");
            ui.checkbox(&mut animation.hide_hud, "Hide HUD").on_hover_text("The HUD's hidden while it plays");
            ui.separator();
            ui.checkbox(&mut self.timeline_preview, "Camera in view").on_hover_text("See through the animation's camera while it runs or is held - off: the editor camera stays");
            if ui.button("Delete").on_hover_text("This animation").clicked() {
                delete = true;
            }
        });
        if let Some(picked) = picked {
            self.show_animation(&node, &picked);
            return;
        }
        if let Some(new_name) = rename {
            self.rename_current(&new_name);
        }
        if delete {
            self.delete_current();
        }
    }

    fn transport(&mut self, ui: &mut egui::Ui) {
        let playback = self.playback;
        let length = self.current().map(|animation| animation.length).unwrap_or(0.0);
        let paths: Vec<String> = self.map.paths.iter().map(|path| path.name.clone()).collect();
        let targets: Vec<(String, Vec<String>)> = self.map.animations.iter()
            .map(|(node, list)| (node.clone(), list.iter().map(|animation| animation.name.clone()).collect()))
            .collect();
        ui.horizontal_wrapped(|ui| {
            if playback.playing {
                if ui.button("Pause").on_hover_text("Holds it here").clicked() {
                    self.timeline_request(|node, name| TimelineRequest::Pause(node, name));
                }
            } else if playback.engaged {
                if ui.button("Start").on_hover_text("On from where it's held").clicked() {
                    self.timeline_request(|node, name| TimelineRequest::Play(node, name));
                }
            } else if ui.button("Start").on_hover_text("From the start").clicked() {
                self.timeline_request(|node, name| TimelineRequest::Start(node, name));
            }
            if ui.button("Reset").on_hover_text("Stops it at the start. A plane it moved is put back where it spawns, flying on its own").clicked() {
                self.timeline_request(|node, name| TimelineRequest::Reset(node, name));
            }
            let state = match (playback.playing, playback.engaged) {
                (true, _) => "playing",
                (false, true) => "held",
                (false, false) => "stopped",
            };
            ui.label(RichText::new(format!("{:.2} / {:.2} s", playback.time, length)).monospace());
            ui.label(RichText::new(state).small().weak());
            ui.separator();
            ui.menu_button("+ Path", |ui| {
                ui.label(RichText::new(format!("From {:.2} s, it follows", playback.time)).small().weak());
                if paths.is_empty() {
                    ui.label(RichText::new("No paths - make one in the Paths window, or with Create path on an aircraft").weak());
                }
                for path in &paths {
                    if ui.button(path).clicked() {
                        self.add_path_bar(path);
                        ui.close();
                    }
                }
            });
            if ui.button("+ Camera key").on_hover_text(format!("At {:.2} s: the camera as the editor camera is now", playback.time)).clicked() {
                self.add_camera_key();
            }
            ui.menu_button("+ Event", |ui| {
                let mut action = None;
                if ui.button("Play audio").clicked() {
                    action = Some(AnimAction::PlayAudio("./assets/audio/".to_owned()));
                }
                if ui.button("Subtitle").clicked() {
                    action = Some(AnimAction::Subtitle { text: "...".to_owned(), seconds: 3.0 });
                }
                ui.separator();
                for (label, start) in [("Start animation", true), ("Stop animation", false)] {
                    ui.menu_button(label, |ui| {
                        for (node, names) in &targets {
                            for name in names {
                                if ui.button(format!("{node} › {name}")).clicked() {
                                    action = Some(if start {
                                        AnimAction::StartAnimation { node: node.clone(), animation: name.clone() }
                                    } else {
                                        AnimAction::StopAnimation { node: node.clone(), animation: name.clone() }
                                    });
                                }
                            }
                        }
                    });
                }
                if let Some(action) = action {
                    self.add_event(action);
                    ui.close();
                }
            });
        });
    }

    /// The ruler (click or drag to scrub) and the rows: path bars, camera
    /// keys, events.
    fn rows(&mut self, ui: &mut egui::Ui) {
        let Some(animation) = self.current().cloned() else { return };
        let owner = self.current_animation.as_ref().map(|(node, _)| node.clone()).unwrap_or_default();
        let marks: Vec<(Vec<f32>, Option<f32>)> = animation.paths.iter().map(|clip| self.bar_marks(&owner, clip)).collect();
        let length = animation.length.max(0.1);
        let playhead = self.playback.time;
        let picked = if self.selection == Selection::TimelineItem { self.timeline_selection } else { TimelineSelection::None };

        let width = ui.available_width();
        let (ruler, ruler_response) = ui.allocate_exact_size(egui::vec2(width, RULER_HEIGHT), egui::Sense::click_and_drag());
        let x0 = ruler.left() + LABEL_COLUMN;
        let x1 = (ruler.right() - PAD as f32 - 8.0).max(x0 + 10.0);
        let per_second = (x1 - x0) / length;
        let x_of = |time: f32| x0 + time.clamp(0.0, length) * per_second;
        let time_at = |x: f32| ((x - x0) / per_second).clamp(0.0, length);

        let visuals = ui.visuals().clone();
        let painter = ui.painter_at(ruler);
        painter.rect_filled(ruler, 0.0, visuals.widgets.inactive.bg_fill);
        let step = [0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0].into_iter().find(|step| step * per_second >= 55.0).unwrap_or(600.0);
        let font = egui::FontId::proportional(11.0);
        let mut tick = 0.0;
        while tick <= length + 1e-4 {
            let x = x_of(tick);
            painter.line_segment([egui::pos2(x, ruler.bottom() - 6.0), egui::pos2(x, ruler.bottom())], egui::Stroke::new(1.0, visuals.weak_text_color()));
            let text = if step < 1.0 { format!("{tick:.2}") } else { format!("{tick:.0}s") };
            painter.text(egui::pos2(x + 2.0, ruler.top() + 2.0), egui::Align2::LEFT_TOP, text, font.clone(), visuals.weak_text_color());
            tick += step;
        }
        painter.text(egui::pos2(ruler.left() + PAD as f32, ruler.center().y), egui::Align2::LEFT_CENTER, "Time", font.clone(), visuals.text_color());
        let playhead_x = x_of(playhead);
        painter.add(egui::Shape::convex_polygon(
            vec![egui::pos2(playhead_x - 5.0, ruler.top()), egui::pos2(playhead_x + 5.0, ruler.top()), egui::pos2(playhead_x, ruler.top() + 8.0)],
            PLAYHEAD_COLOR,
            egui::Stroke::NONE,
        ));
        if ruler_response.clicked() || ruler_response.dragged() {
            if let Some(pointer) = ruler_response.interact_pointer_pos() {
                if pointer.x >= x0 - 4.0 {
                    let at = snap(time_at(pointer.x));
                    self.timeline_request(|node, name| TimelineRequest::Seek(node, name, at));
                }
            }
        }
        ruler_response.on_hover_text("Click or drag to scrub");

        let mut clicked: Option<(TimelineSelection, f32)> = None;
        let mut drag: Option<Drag> = None;
        let area_top = ui.cursor().top();
        let label_font = egui::FontId::proportional(12.0);
        let small = egui::FontId::proportional(11.0);

        // Path bars.
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), egui::Sense::hover());
        {
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 0.0, visuals.faint_bg_color);
            painter.text(egui::pos2(rect.left() + PAD as f32, rect.center().y), egui::Align2::LEFT_CENTER, "Paths", label_font.clone(), PATH_COLOR);
            for (index, clip) in animation.paths.iter().enumerate() {
                let bar = egui::Rect::from_min_max(egui::pos2(x_of(clip.start), rect.top() + 3.0), egui::pos2(x_of(clip.end).max(x_of(clip.start) + 4.0), rect.bottom() - 3.0));
                let lit = picked == TimelineSelection::Path(index);
                painter.rect_filled(bar, 4.0, PATH_COLOR.gamma_multiply(if lit { 0.6 } else { 0.3 }));
                painter.rect_stroke(bar, 4.0, egui::Stroke::new(1.0, if lit { SELECTED_COLOR } else { PATH_COLOR }), egui::StrokeKind::Inside);
                // Where along it each of the path's points is reached - and
                // a flown path's end.
                let (points, path_end) = &marks[index];
                for (number, &at) in points.iter().enumerate() {
                    let x = x_of(at);
                    painter.line_segment([egui::pos2(x, bar.bottom() - 6.0), egui::pos2(x, bar.bottom() - 1.0)], egui::Stroke::new(1.0, SELECTED_COLOR.gamma_multiply(0.7)));
                    if per_second * (points.get(number + 1).copied().unwrap_or(clip.end) - at) > 14.0 {
                        painter.text(egui::pos2(x + 2.0, bar.bottom() - 2.0), egui::Align2::LEFT_BOTTOM, format!("{}", number % points.len().max(1) + 1), egui::FontId::proportional(9.0), SELECTED_COLOR.gamma_multiply(0.7));
                    }
                }
                if let Some(end) = path_end {
                    let x = x_of(*end);
                    painter.line_segment([egui::pos2(x, bar.top() + 1.0), egui::pos2(x, bar.bottom() - 1.0)], egui::Stroke::new(2.0, PLAYHEAD_COLOR.gamma_multiply(0.8)));
                }
                let how = match clip.mode {
                    FollowMode::Positioned => "positioned",
                    FollowMode::Flown => "flown · path-based",
                };
                painter.text(bar.left_center() + egui::vec2(EDGE_GRAB + 2.0, -3.0), egui::Align2::LEFT_CENTER, format!("{} · {how}", clip.path), small.clone(), SELECTED_COLOR);
                // The two edge grabs.
                for edge in [bar.left(), bar.right()] {
                    let grip = egui::Rect::from_center_size(egui::pos2(edge, bar.center().y), egui::vec2(3.0, bar.height() - 6.0));
                    painter.rect_filled(grip, 1.0, if lit { SELECTED_COLOR } else { PATH_COLOR });
                }
                let parts = [
                    (BarPart::Start, egui::Rect::from_min_max(bar.left_top(), egui::pos2(bar.left() + EDGE_GRAB, bar.bottom()))),
                    (BarPart::End, egui::Rect::from_min_max(egui::pos2(bar.right() - EDGE_GRAB, bar.top()), bar.right_bottom())),
                    (BarPart::Whole, bar.shrink2(egui::vec2(EDGE_GRAB, 0.0))),
                ];
                for (part, hit) in parts {
                    if hit.width() <= 0.0 {
                        continue;
                    }
                    let response = ui.interact(hit, egui::Id::new(("timeline_bar", index, part as u8)), egui::Sense::click_and_drag());
                    let cursor = if part == BarPart::Whole { egui::CursorIcon::Grab } else { egui::CursorIcon::ResizeHorizontal };
                    let response = response.on_hover_cursor(cursor);
                    if response.clicked() || response.drag_started() {
                        clicked = Some((TimelineSelection::Path(index), clip.start));
                    }
                    if response.dragged() {
                        if let Some(pointer) = response.interact_pointer_pos() {
                            drag = Some(Drag::Bar { index, part, time: snap(time_at(pointer.x)), delta: response.drag_delta().x / per_second });
                        }
                    }
                }
            }
        }

        // Camera keys.
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), egui::Sense::hover());
        {
            let painter = ui.painter_at(rect);
            painter.text(egui::pos2(rect.left() + PAD as f32, rect.center().y), egui::Align2::LEFT_CENTER, "Camera", label_font.clone(), CAMERA_COLOR);
            let y = rect.center().y;
            // Between keys: a solid line into a smooth one (the move), a
            // faint one before a cut (held, then the cut).
            for pair in animation.camera.windows(2) {
                let (from, to) = (x_of(pair[0].time), x_of(pair[1].time));
                let stroke = if pair[1].transition == Transition::Smooth { egui::Stroke::new(3.0, CAMERA_COLOR.gamma_multiply(0.8)) } else { egui::Stroke::new(1.0, CAMERA_COLOR.gamma_multiply(0.35)) };
                painter.line_segment([egui::pos2(from, y), egui::pos2(to, y)], stroke);
            }
            if let Some(last) = animation.camera.last() {
                painter.line_segment([egui::pos2(x_of(last.time), y), egui::pos2(x_of(length), y)], egui::Stroke::new(1.0, CAMERA_COLOR.gamma_multiply(0.35)));
            }
            for (index, key) in animation.camera.iter().enumerate() {
                let center = egui::pos2(x_of(key.time), y);
                let hit = egui::Rect::from_center_size(center, egui::vec2(KEY_GRAB * 2.0, ROW_HEIGHT));
                let response = ui.interact(hit, egui::Id::new(("timeline_camera_key", index)), egui::Sense::click_and_drag()).on_hover_cursor(egui::CursorIcon::Grab);
                let lit = picked == TimelineSelection::Camera(index) || response.hovered();
                let color = if lit { SELECTED_COLOR } else { CAMERA_COLOR };
                match key.transition {
                    Transition::Cut => {
                        let points = vec![center + egui::vec2(0.0, -KEY_SIZE), center + egui::vec2(KEY_SIZE, 0.0), center + egui::vec2(0.0, KEY_SIZE), center + egui::vec2(-KEY_SIZE, 0.0)];
                        painter.add(egui::Shape::convex_polygon(points, color, egui::Stroke::new(1.0, Color32::BLACK)));
                    }
                    Transition::Smooth => {
                        painter.circle(center, KEY_SIZE, color, egui::Stroke::new(1.0, Color32::BLACK));
                    }
                }
                if response.clicked() || response.drag_started() {
                    clicked = Some((TimelineSelection::Camera(index), key.time));
                }
                if response.dragged() {
                    if let Some(pointer) = response.interact_pointer_pos() {
                        drag = Some(Drag::Key { index, time: snap(time_at(pointer.x)) });
                    }
                }
            }
            if animation.camera.is_empty() {
                painter.text(egui::pos2(x0 + 8.0, y), egui::Align2::LEFT_CENTER, "the player's view - + Camera key takes the editor camera's", small.clone(), visuals.weak_text_color());
            }
        }

        // Events.
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), egui::Sense::hover());
        {
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, 0.0, visuals.faint_bg_color);
            painter.text(egui::pos2(rect.left() + PAD as f32, rect.center().y), egui::Align2::LEFT_CENTER, "Events", label_font.clone(), EVENT_COLOR);
            for (index, event) in animation.events.iter().enumerate() {
                let center = egui::pos2(x_of(event.time), rect.center().y);
                painter.text(center + egui::vec2(KEY_SIZE + 4.0, 0.0), egui::Align2::LEFT_CENTER, event.action.describe(), small.clone(), EVENT_COLOR.gamma_multiply(0.8));
                let hit = egui::Rect::from_center_size(center, egui::vec2(KEY_GRAB * 2.0, ROW_HEIGHT));
                let response = ui.interact(hit, egui::Id::new(("timeline_event", index)), egui::Sense::click_and_drag()).on_hover_cursor(egui::CursorIcon::Grab);
                let lit = picked == TimelineSelection::Event(index) || response.hovered();
                painter.circle_filled(center, KEY_SIZE - 1.0, if lit { SELECTED_COLOR } else { EVENT_COLOR });
                if response.clicked() || response.drag_started() {
                    clicked = Some((TimelineSelection::Event(index), event.time));
                }
                if response.dragged() {
                    if let Some(pointer) = response.interact_pointer_pos() {
                        drag = Some(Drag::Event { index, time: snap(time_at(pointer.x)) });
                    }
                }
            }
        }

        // The playhead, down through every row.
        let bottom = ui.cursor().top();
        ui.painter().line_segment([egui::pos2(playhead_x, area_top), egui::pos2(playhead_x, bottom)], egui::Stroke::new(1.5, PLAYHEAD_COLOR));
        ui.painter().line_segment([egui::pos2(x0, area_top), egui::pos2(x0, bottom)], egui::Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color));

        if let Some((item, at)) = clicked {
            self.pick(item, at);
        }
        if let Some(drag) = drag {
            let Some(animation) = self.current_mut() else { return };
            let picked = match drag {
                Drag::Bar { index, part, time, delta } => {
                    let clip = &mut animation.paths[index];
                    match part {
                        BarPart::Start => clip.start = time.min(clip.end - 0.1).max(0.0),
                        BarPart::End => clip.end = time.max(clip.start + 0.1),
                        BarPart::Whole => {
                            let span = clip.end - clip.start;
                            clip.start = snap((clip.start + delta).max(0.0));
                            clip.end = clip.start + span;
                        }
                    }
                    let moved = clip.clone();
                    animation.sort();
                    TimelineSelection::Path(animation.paths.iter().position(|clip| *clip == moved).unwrap_or(0))
                }
                Drag::Key { index, time } => {
                    animation.camera[index].time = time;
                    let moved = animation.camera[index].clone();
                    animation.sort();
                    TimelineSelection::Camera(animation.camera.iter().position(|key| *key == moved).unwrap_or(0))
                }
                Drag::Event { index, time } => {
                    animation.events[index].time = time;
                    let moved = animation.events[index].clone();
                    animation.sort();
                    TimelineSelection::Event(animation.events.iter().position(|event| *event == moved).unwrap_or(0))
                }
            };
            self.timeline_selection = picked;
            self.selection = Selection::TimelineItem;
        }

        // Delete takes the picked thing out (not while typing).
        if ui.input(|input| input.key_pressed(egui::Key::Delete)) && !ui.ctx().wants_keyboard_input() && self.selection == Selection::TimelineItem {
            self.delete_picked();
        }
    }

    fn delete_picked(&mut self) {
        let picked = self.timeline_selection;
        if let Some(animation) = self.current_mut() {
            match picked {
                TimelineSelection::Path(index) if index < animation.paths.len() => { animation.paths.remove(index); }
                TimelineSelection::Camera(index) if index < animation.camera.len() => { animation.camera.remove(index); }
                TimelineSelection::Event(index) if index < animation.events.len() => { animation.events.remove(index); }
                _ => {}
            }
        }
        self.timeline_selection = TimelineSelection::None;
        self.select(Selection::None);
    }

    /// Properties for what's picked on the timeline - a path bar, a camera
    /// key, an event - edited on the spot.
    pub(super) fn timeline_item_inspector(&mut self, ui: &mut egui::Ui) {
        let Some((node, name)) = self.current_animation.clone() else { return };
        let mut delete = false;
        match self.timeline_selection {
            TimelineSelection::None => {}
            TimelineSelection::Path(index) => delete = self.path_bar_inspector(ui, &node, &name, index),
            TimelineSelection::Camera(index) => delete = self.camera_key_inspector(ui, &node, &name, index),
            TimelineSelection::Event(index) => delete = self.event_inspector(ui, &node, &name, index),
        }
        if delete {
            self.delete_picked();
        }
    }

    fn path_bar_inspector(&mut self, ui: &mut egui::Ui, node: &str, name: &str, index: usize) -> bool {
        let paths: Vec<String> = self.map.paths.iter().map(|path| path.name.clone()).collect();
        let is_aircraft = self.nodes.iter().any(|info| info.id == node && info.kind == NodeKind::Aircraft);
        let own_skill = self.map.find(node).map(|(_, placement)| placement.difficulty);
        let Some(clip) = self.map.animation(node, name).and_then(|animation| animation.paths.get(index)).cloned() else { return false };
        let path_length = self.map.named_path(&clip.path).and_then(SampledPath::new).map(|path| path.length()).unwrap_or(0.0);
        inspector_header(ui, Icon::Path, ui.visuals().text_color(), &format!("Path bar · {}", clip.path), |ui| {
            ui.label(RichText::new(format!("{node} › {name}")).small().weak());
        }, |_| {});

        let mut edited = clip.clone();
        let mut delete = false;
        section(ui, "Path", |ui| {
            property(ui, "Follows", |ui| {
                egui::ComboBox::from_id_salt("bar_path")
                    .width(ui.available_width())
                    .selected_text(edited.path.as_str())
                    .show_ui(ui, |ui| {
                        for path in &paths {
                            ui.selectable_value(&mut edited.path, path.clone(), path.as_str());
                        }
                    });
            });
            property(ui, "How", |ui| {
                ui.radio_value(&mut edited.mode, FollowMode::Positioned, "Positioned").on_hover_text("Placed along the path by time - start of the bar at its start, end of the bar at its end, whatever the physics");
                ui.add_enabled_ui(is_aircraft, |ui| {
                    ui.radio_value(&mut edited.mode, FollowMode::Flown, "Flown").on_hover_text("Its flight system flies it for real, from the path's start, until the bar ends");
                }).response.on_disabled_hover_text("Only an aircraft can fly a path");
            });
            property(ui, "Starts at", |ui| {
                ui.add(egui::DragValue::new(&mut edited.start).speed(0.05).range(0.0..=3600.0).suffix(" s").max_decimals(2));
            });
            property(ui, "Ends at", |ui| {
                ui.add(egui::DragValue::new(&mut edited.end).speed(0.05).range((edited.start + 0.1)..=3600.0).suffix(" s").max_decimals(2));
            });
            match edited.mode {
                FollowMode::Positioned => {
                    property(ui, "Speed", |ui| {
                        let seconds = (edited.end - edited.start).max(0.01);
                        ui.label(RichText::new(format!("{:.0} kt - {:.0} m in {seconds:.2} s (drag the bar's right edge to change it)", path_length / seconds / KT_TO_MS, path_length)).weak());
                    });
                    property(ui, "Nose along path", |ui| {
                        ui.checkbox(&mut edited.orient, "");
                    });
                }
                FollowMode::Flown => {
                    property(ui, "", |ui| {
                        ui.label(RichText::new("Path-based: scrub into the bar and the plane's shown where along the path it'd be at its flying speed (the ticks are its points) - play from there and it flies on from that point.").small().weak());
                    });
                    property(ui, "Set speed", |ui| {
                        let mut set = edited.speed_kt.is_some();
                        if ui.checkbox(&mut set, "").on_hover_text("Off: as fast as its pilot's skill flies - an Ace at full power").changed() {
                            edited.speed_kt = set.then_some(edited.speed_kt.unwrap_or(350.0));
                        }
                        if let Some(knots) = &mut edited.speed_kt {
                            ui.add(egui::DragValue::new(knots).speed(1.0).range(50.0..=2000.0).suffix(" kt"));
                        }
                    });
                    property(ui, "Pilot skill", |ui| {
                        let own = own_skill.map(|skill| format!("its own ({})", skill.label())).unwrap_or_else(|| "Medium".to_owned());
                        egui::ComboBox::from_id_salt("bar_skill")
                            .width(ui.available_width())
                            .selected_text(edited.skill.map(|skill| skill.label().to_owned()).unwrap_or(own.clone()))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut edited.skill, None, own.as_str());
                                for skill in Difficulty::ALL {
                                    ui.selectable_value(&mut edited.skill, Some(skill), skill.label());
                                }
                            });
                    });
                }
            }
        });
        padded(ui, |ui| {
            if ui.button("Delete path bar").clicked() {
                delete = true;
            }
        });
        if edited != clip {
            if let Some(animation) = self.map.animation_mut(node, name) {
                if let Some(slot) = animation.paths.get_mut(index) {
                    *slot = edited;
                }
            }
        }
        delete
    }

    fn camera_key_inspector(&mut self, ui: &mut egui::Ui, node: &str, name: &str, index: usize) -> bool {
        let mut targets: Vec<String> = self.nodes.iter().filter(|info| info.kind != NodeKind::Runtime).map(|info| info.id.clone()).collect();
        targets.sort();
        let Some(key) = self.map.animation(node, name).and_then(|animation| animation.camera.get(index)).cloned() else { return false };
        inspector_header(ui, Icon::Camera, ui.visuals().text_color(), &format!("Camera key · {:.2} s", key.time), |ui| {
            ui.label(RichText::new(format!("{node} › {name}")).small().weak());
        }, |_| {});

        let mut edited = key.clone();
        let mut delete = false;
        let mut take_view = false;
        let mut follow_changed = None;
        section(ui, "Camera key", |ui| {
            property(ui, "Time", |ui| {
                ui.add(egui::DragValue::new(&mut edited.time).speed(0.05).range(0.0..=3600.0).suffix(" s").max_decimals(2));
            });
            property(ui, "Transition", |ui| {
                ui.radio_value(&mut edited.transition, Transition::Cut, "Cut").on_hover_text("A hard cut to it at its time (a diamond)");
                ui.radio_value(&mut edited.transition, Transition::Smooth, "Smooth").on_hover_text("A smooth move from the key before, all the way to its time (a circle)");
            });
            property(ui, "", |ui| {
                if ui.add(egui::Button::new("Set to the editor camera").min_size(egui::vec2(ui.available_width(), 0.0))).on_hover_text("Where the editor camera is now and the way it looks, its field of view too").clicked() {
                    take_view = true;
                }
            });
            let space = if edited.follow.is_some() { "Offset" } else { "Position" };
            property(ui, space, |ui| {
                let (x, y, z) = &mut edited.position;
                for (value, letter) in [(x, "x"), (y, "y"), (z, "z")] {
                    ui.label(RichText::new(letter).weak());
                    ui.add(egui::DragValue::new(value).speed(0.5).max_decimals(1));
                }
            });
            property(ui, "Rotation", |ui| {
                ui.add_enabled_ui(edited.look_at.is_none(), |ui| {
                    let (x, y, z) = &mut edited.rotation;
                    for (value, letter) in [(x, "x"), (y, "y"), (z, "z")] {
                        ui.label(RichText::new(letter).weak());
                        ui.add(egui::DragValue::new(value).speed(0.5).suffix("°").max_decimals(1));
                    }
                }).response.on_disabled_hover_text("It looks at what it looks at");
            });
            property(ui, "Field of view", |ui| {
                ui.spacing_mut().slider_width = (ui.available_width() - 60.0).max(80.0);
                ui.add(egui::Slider::new(&mut edited.fov, 5.0..=150.0).suffix("°").max_decimals(1));
            });
            property(ui, "Wave", |ui| {
                ui.checkbox(&mut edited.wave, "").on_hover_text("Drifts while this key holds, instead of sitting locked. Keeps moving while the animation is held, so you can see it as you set it");
            });
            if edited.wave {
                property(ui, "Amount", |ui| {
                    ui.add(egui::DragValue::new(&mut edited.wave_amount).speed(0.1).range(0.0..=500.0).suffix(" m").max_decimals(1)).on_hover_text("How far it travels");
                });
                property(ui, "Speed", |ui| {
                    ui.add(egui::DragValue::new(&mut edited.wave_speed).speed(0.05).range(0.0..=20.0).max_decimals(2)).on_hover_text("How fast it travels. 1 is a slow swing, higher is quicker");
                });
                property(ui, "On", |ui| {
                    ui.radio_value(&mut edited.wave_on, WaveOn::Position, "Position").on_hover_text("The camera itself drifts");
                    ui.radio_value(&mut edited.wave_on, WaveOn::Objective, "Objective").on_hover_text("The point it looks at drifts, and the camera turns to follow. With nothing to look at, that's a point ahead of the camera");
                });
            }
            property(ui, "Follows", |ui| {
                let mut follow = edited.follow.clone();
                egui::ComboBox::from_id_salt("key_follow")
                    .width(ui.available_width())
                    .selected_text(follow.clone().unwrap_or_else(|| "nothing (stays put)".to_owned()))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut follow, None, "nothing (stays put)");
                        for target in &targets {
                            ui.selectable_value(&mut follow, Some(target.clone()), target.as_str());
                        }
                    })
                    .response
                    .on_hover_text("A node it moves with. With Follow rotation on, the offset is in that node's frame");
                if follow != edited.follow {
                    follow_changed = Some(follow);
                }
            });
            if follow_changed.as_ref().map(|follow| follow.is_some()).unwrap_or(edited.follow.is_some()) {
                property(ui, "Follow rotation", |ui| {
                    ui.checkbox(&mut edited.follow_rotation, "").on_hover_text("On: the offset turns with it, so a camera set on top stays there when it rolls. Off: it follows the position only and keeps its own rotation");
                });
            }
            property(ui, "Looks at", |ui| {
                egui::ComboBox::from_id_salt("key_look_at")
                    .width(ui.available_width())
                    .selected_text(edited.look_at.clone().unwrap_or_else(|| "nothing".to_owned()))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut edited.look_at, None, "nothing");
                        for target in &targets {
                            ui.selectable_value(&mut edited.look_at, Some(target.clone()), target.as_str());
                        }
                    });
            });
        });
        padded(ui, |ui| {
            if ui.button("Delete camera key").clicked() {
                delete = true;
            }
        });

        if take_view {
            let (position, rotation) = self.view_as_key_pose(edited.follow.as_deref(), edited.follow_rotation);
            edited.position = position;
            edited.rotation = rotation;
            if self.view_fov > 1.0 {
                edited.fov = self.view_fov;
            }
        }
        // A new follow keeps it where it is in the world - its offset
        // worked out again.
        if let Some(follow) = follow_changed {
            let local = Transform3D {
                position: Vector3::new(edited.position.0, edited.position.1, edited.position.2),
                rotation: rotation_from_degrees(Vector3::new(edited.rotation.0, edited.rotation.1, edited.rotation.2)),
                scale: Vector3::new(1.0, 1.0, 1.0),
            };
            let world = match edited.follow.as_deref().and_then(|id| self.node_world(id)) {
                Some(parent) => parent.compose(&local, key.follow_rotation),
                None => local,
            };
            let placed = match follow.as_deref().and_then(|id| self.node_world(id)) {
                Some(parent) => parent.relative(&world, edited.follow_rotation),
                None => world,
            };
            edited.position = tuple(placed.position);
            edited.rotation = tuple(degrees_from_rotation(&placed.rotation));
            edited.follow = follow;
        } else if !take_view && edited.follow_rotation != key.follow_rotation {
            // Flipping the toggle keeps the camera where it is. The offset
            // is worked out again in the frame the toggle now means.
            if let Some(parent) = edited.follow.as_deref().and_then(|id| self.node_world(id)) {
                let local = Transform3D {
                    position: Vector3::new(edited.position.0, edited.position.1, edited.position.2),
                    rotation: rotation_from_degrees(Vector3::new(edited.rotation.0, edited.rotation.1, edited.rotation.2)),
                    scale: Vector3::new(1.0, 1.0, 1.0),
                };
                let world = parent.compose(&local, key.follow_rotation);
                let placed = parent.relative(&world, edited.follow_rotation);
                edited.position = tuple(placed.position);
                edited.rotation = tuple(degrees_from_rotation(&placed.rotation));
            }
        }
        if edited != key {
            let time = edited.time;
            if let Some(animation) = self.map.animation_mut(node, name) {
                if let Some(slot) = animation.camera.get_mut(index) {
                    *slot = edited.clone();
                }
                animation.sort();
                if let Some(new_index) = animation.camera.iter().position(|key| *key == edited) {
                    self.timeline_selection = TimelineSelection::Camera(new_index);
                }
            }
            if time != key.time {
                self.timeline_request(|node, name| TimelineRequest::Seek(node, name, time));
            }
        }
        delete
    }

    fn event_inspector(&mut self, ui: &mut egui::Ui, node: &str, name: &str, index: usize) -> bool {
        let targets: Vec<(String, Vec<String>)> = self.map.animations.iter()
            .map(|(node, list)| (node.clone(), list.iter().map(|animation| animation.name.clone()).collect()))
            .collect();
        let Some(event) = self.map.animation(node, name).and_then(|animation| animation.events.get(index)).cloned() else { return false };
        inspector_header(ui, Icon::Node, ui.visuals().text_color(), &format!("Event · {:.2} s", event.time), |ui| {
            ui.label(RichText::new(format!("{node} › {name}")).small().weak());
        }, |_| {});
        let mut edited = event.clone();
        let mut delete = false;
        section(ui, "Event", |ui| {
            property(ui, "Time", |ui| {
                ui.add(egui::DragValue::new(&mut edited.time).speed(0.05).range(0.0..=3600.0).suffix(" s").max_decimals(2));
            });
            match &mut edited.action {
                AnimAction::PlayAudio(folder) => property(ui, "Audio folder", |ui| {
                    ui.add_sized([ui.available_width(), 20.0], egui::TextEdit::singleline(folder)).on_hover_text("A folder with an audio.ogg in it");
                }),
                AnimAction::Subtitle { text, seconds } => {
                    property(ui, "Subtitle", |ui| {
                        ui.add_sized([ui.available_width(), 20.0], egui::TextEdit::singleline(text));
                    });
                    property(ui, "For", |ui| {
                        ui.add(egui::DragValue::new(seconds).speed(0.1).range(0.1..=60.0).suffix(" s"));
                    });
                }
                AnimAction::StartAnimation { node: of, animation } | AnimAction::StopAnimation { node: of, animation } => property(ui, "Animation", |ui| {
                    egui::ComboBox::from_id_salt("event_animation")
                        .width(ui.available_width())
                        .selected_text(format!("{of} › {animation}"))
                        .show_ui(ui, |ui| {
                            for (target, names) in &targets {
                                for target_name in names {
                                    let current = *of == *target && *animation == *target_name;
                                    if ui.selectable_label(current, format!("{target} › {target_name}")).clicked() {
                                        *of = target.clone();
                                        *animation = target_name.clone();
                                    }
                                }
                            }
                        });
                }),
            }
        });
        padded(ui, |ui| {
            if ui.button("Delete event").clicked() {
                delete = true;
            }
        });
        if edited != event {
            if let Some(animation) = self.map.animation_mut(node, name) {
                if let Some(slot) = animation.events.get_mut(index) {
                    *slot = edited.clone();
                }
                animation.sort();
                if let Some(new_index) = animation.events.iter().position(|event| *event == edited) {
                    self.timeline_selection = TimelineSelection::Event(new_index);
                }
            }
        }
        delete
    }
}
