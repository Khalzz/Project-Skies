//! The editor's hierarchy - every node in the scene, children under their
//! parents - and the inspector for a node that isn't an aircraft. The map's
//! own nodes (`map.nodes` - cameras, mostly) are edited here: their parent,
//! whether they're placed `Local` (an offset from the parent, moving with it)
//! or `Global` (a place in the world, disconnected from it), their transform,
//! lens and aim. Anything else the level spawned is shown as it is.

use std::collections::{HashMap, HashSet};

use egui::RichText;
use nalgebra::Vector3;

use crate::engine::scene_manager::properties::{degrees_from_rotation, rotation_from_degrees, Space, Transform3D};
use crate::game::scenes::play::map::{AnimAction, CameraSpec, DefaultFollow, NodeSpec, Pace};

use super::{inspector_header, padded, property, row_line, section, tree_row, Icon, NodeKind, RowClick, SceneNodeInfo, Selection, TreeRow, AXIS_COLORS, FlightEditor, PLAYER_COLOR};

/// A new camera child: behind and above its parent, looking where it looks
/// (m, in the parent's frame).
const NEW_CHILD_CAMERA_OFFSET: (f32, f32, f32) = (0.0, 4.0, -25.0);
const NEW_CAMERA_FOV: f32 = 60.0;

/// The order roots are listed in - the map's own first.
fn kind_rank(kind: NodeKind) -> u8 {
    match kind {
        NodeKind::Aircraft => 0,
        NodeKind::MapNode => 1,
        NodeKind::Level => 2,
        NodeKind::Runtime => 3,
    }
}

fn world_of(info: &SceneNodeInfo) -> Transform3D {
    Transform3D { position: info.world_position, rotation: rotation_from_degrees(info.world_rotation), scale: Vector3::new(1.0, 1.0, 1.0) }
}

fn tuple(vector: Vector3<f32>) -> (f32, f32, f32) {
    (vector.x, vector.y, vector.z)
}

/// `vector_field`, saying whether a value changed.
fn vector_edit(ui: &mut egui::Ui, values: [&mut f32; 3], speed: f64, suffix: &str) -> bool {
    let spacing = ui.spacing().item_spacing.x;
    let field_width = ((ui.available_width() - 30.0 - 5.0 * spacing) / 3.0).max(30.0);
    let mut changed = false;
    for ((value, letter), color) in values.into_iter().zip(["x", "y", "z"]).zip(AXIS_COLORS) {
        ui.add_sized([10.0, 20.0], egui::Label::new(RichText::new(letter).color(color).strong()));
        changed |= ui.add_sized([field_width, 20.0], egui::DragValue::new(value).speed(speed).suffix(suffix).max_decimals(2)).changed();
    }
    changed
}

impl FlightEditor {
    fn node_info(&self, id: &str) -> Option<&SceneNodeInfo> {
        self.nodes.iter().find(|node| node.id == id)
    }

    /// Whether `name` can be a new node's id - not an aircraft's, another
    /// node's (the map's or the scene's), empty, or with a `/`.
    fn id_free(&self, name: &str) -> bool {
        !name.is_empty() && !name.contains('/') && !self.map.name_taken(name) && self.node_info(name).is_none()
    }

    fn unique_id(&self, base: &str) -> String {
        (1..).map(|n| if n == 1 { base.to_owned() } else { format!("{base}_{n}") })
            .find(|name| self.id_free(name))
            .unwrap()
    }

    /// `id`'s children among the map's nodes, theirs and so on.
    fn map_descendants(&self, id: &str) -> Vec<String> {
        let mut found: Vec<String> = Vec::new();
        let mut next = vec![id.to_owned()];
        while let Some(current) = next.pop() {
            for node in &self.map.nodes {
                if node.parent.as_deref() == Some(current.as_str()) && !found.contains(&node.id) {
                    found.push(node.id.clone());
                    next.push(node.id.clone());
                }
            }
        }
        found
    }

    /// The hierarchy: adding a camera or a plain node, then every node in
    /// the scene as a tree - a row's arrow folds its children, a click
    /// selects it (an aircraft opens its own inspector).
    pub(super) fn node_hierarchy(&mut self, ui: &mut egui::Ui) {
        padded(ui, |ui| ui.horizontal(|ui| {
            let under = self.selected_node().map(|id| format!("Under '{id}'")).unwrap_or_else(|| "At the editor's view".to_owned());
            if ui.button("+ Camera").on_hover_text(format!("A camera node - {under}")).clicked() {
                self.add_map_node(true);
            }
            if ui.button("+ Node").on_hover_text(format!("An empty node, to parent things to - {under}")).clicked() {
                self.add_map_node(false);
            }
        }));

        let visible: Vec<&SceneNodeInfo> = self.nodes.iter().filter(|node| self.show_runtime || node.kind != NodeKind::Runtime).collect();
        let ids: HashSet<&str> = visible.iter().map(|node| node.id.as_str()).collect();
        let mut children: HashMap<&str, Vec<&SceneNodeInfo>> = HashMap::new();
        let mut roots = Vec::new();
        for node in &visible {
            match node.parent.as_deref().filter(|parent| ids.contains(parent)) {
                Some(parent) => children.entry(parent).or_default().push(node),
                None => roots.push(*node),
            }
        }
        roots.sort_by(|a, b| (kind_rank(a.kind), &a.id).cmp(&(kind_rank(b.kind), &b.id)));
        for list in children.values_mut() {
            list.sort_by(|a, b| a.id.cmp(&b.id));
        }

        // Flattened, depth first - (node, depth, has children).
        let mut rows: Vec<(&SceneNodeInfo, usize, bool)> = Vec::new();
        let mut stack: Vec<(&SceneNodeInfo, usize)> = roots.iter().rev().map(|node| (*node, 0)).collect();
        while let Some((node, depth)) = stack.pop() {
            let kids = children.get(node.id.as_str());
            rows.push((node, depth, kids.is_some()));
            if rows.len() > visible.len() {
                break;
            }
            if let Some(kids) = kids.filter(|_| !self.folded_nodes.contains(&node.id)) {
                for kid in kids.iter().rev() {
                    stack.push((*kid, depth + 1));
                }
            }
        }

        let player = self.map.aircraft().map(|(_, aircraft)| aircraft).find(|aircraft| aircraft.player).map(|aircraft| aircraft.name.clone());
        let visuals = ui.visuals().clone();
        let mut clicked = None;
        let mut fold = None;
        egui::Frame::new()
            .fill(visuals.extreme_bg_color)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical().id_salt("node_hierarchy").auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    if rows.is_empty() {
                        ui.label(RichText::new("  Nothing in the scene").weak());
                    }
                    for (index, (node, depth, has_children)) in rows.iter().enumerate() {
                        if index > 0 {
                            row_line(ui);
                        }
                        let (icon, icon_color) = match (node.kind, node.camera) {
                            (NodeKind::Aircraft, _) if player.as_deref() == Some(node.id.as_str()) => (Icon::Aircraft, PLAYER_COLOR),
                            (NodeKind::Aircraft, _) => (Icon::Aircraft, visuals.weak_text_color()),
                            (_, true) => (Icon::Camera, if self.preview_camera.as_deref() == Some(node.id.as_str()) { PLAYER_COLOR } else { visuals.text_color() }),
                            _ => (Icon::Node, visuals.weak_text_color()),
                        };
                        let mut detail = Vec::new();
                        if node.parent.is_some() && node.space == Space::Global {
                            detail.push("global");
                        }
                        if node.animated {
                            detail.push("animated");
                        }
                        match node.kind {
                            NodeKind::MapNode => detail.push("map"),
                            NodeKind::Runtime => detail.push("runtime"),
                            _ => {}
                        }
                        let row = TreeRow {
                            depth: *depth,
                            fold: has_children.then(|| !self.folded_nodes.contains(&node.id)),
                            icon,
                            icon_color,
                            text: node.id.clone(),
                            text_color: (node.kind == NodeKind::Runtime).then(|| visuals.weak_text_color()),
                            detail: detail.join(" · "),
                        };
                        let selected = self.selected_node() == Some(node.id.as_str());
                        match tree_row(ui, selected, row) {
                            RowClick::Fold => fold = Some(node.id.clone()),
                            RowClick::Select => clicked = Some(if node.kind == NodeKind::Aircraft { Selection::Aircraft(node.id.clone()) } else { Selection::Node(node.id.clone()) }),
                            RowClick::None => {}
                        }
                    }
                });
            });
        if let Some(id) = fold {
            if !self.folded_nodes.remove(&id) {
                self.folded_nodes.insert(id);
            }
        }
        if let Some(selection) = clicked {
            if selection != self.selection {
                self.select(selection);
            }
        }
    }

    /// A new camera (or plain node) in the map: a child of the selected
    /// node, behind and above it - or, with nothing selected, at the
    /// editor's view, in the world. Selected straight away.
    fn add_map_node(&mut self, camera: bool) {
        let id = self.unique_id(if camera { "camera" } else { "node" });
        let parent = self.selected_node().map(str::to_owned);
        let position = match &parent {
            Some(_) if camera => NEW_CHILD_CAMERA_OFFSET,
            Some(_) => (0.0, 0.0, 0.0),
            None => tuple(self.view_position),
        };
        self.map.nodes.push(NodeSpec {
            id: id.clone(),
            parent,
            space: Space::Local,
            follow_rotation: true,
            position,
            rotation: (0.0, 0.0, 0.0),
            camera: camera.then_some(CameraSpec { fov: NEW_CAMERA_FOV, near: 0.1 }),
            look_at: None,
            follow: None,
        });
        self.select(Selection::Node(id));
    }

    /// A node that isn't an aircraft: one of the map's, editable - or one
    /// the level spawned, shown as it is. Either can be animated (see the
    /// timeline).
    pub(super) fn node_inspector(&mut self, ui: &mut egui::Ui, id: &str) {
        let info = self.node_info(id).cloned();
        let is_map = self.map.node(id).is_some();
        let is_camera = info.as_ref().is_some_and(|info| info.camera) || self.map.node(id).is_some_and(|spec| spec.camera.is_some());
        let kind_text = match info.as_ref().map(|info| info.kind) {
            _ if is_map && info.is_none() => "Map node · not in the scene yet",
            Some(NodeKind::MapNode) => "Map node · saved to map.ron",
            Some(NodeKind::Runtime) => "Spawned while flying · not saved",
            Some(_) => "Spawned by the level · not saved",
            None => "Not in the scene",
        };
        let preview = &mut self.preview_camera;
        let requests = &mut self.requests;
        let icon = if is_camera { Icon::Camera } else { Icon::Node };
        inspector_header(ui, icon, ui.visuals().text_color(), id, |ui| {
            ui.label(RichText::new(kind_text).small().weak());
        }, |ui| {
            if info.is_some() && ui.button("Focus").on_hover_text("The editor camera to it").clicked() {
                requests.push(super::EditorRequest::Focus(id.to_owned()));
            }
            if is_camera {
                let mut through = preview.as_deref() == Some(id);
                if ui.toggle_value(&mut through, "View through").on_hover_text("See through this camera instead of the editor camera").changed() {
                    *preview = through.then(|| id.to_owned());
                }
            }
        });

        if is_map {
            self.map_node_sections(ui, id, info.as_ref());
            self.default_path_section(ui, id);
        } else {
            self.scene_node_sections(ui, info.as_ref());
        }
        self.animations_section(ui, id);

        if is_map {
            padded(ui, |ui| {
                let descendants = self.map_descendants(id).len();
                let hint = if descendants > 0 { format!("Its {descendants} child node(s) go too") } else { "Takes it out of the map".to_owned() };
                if ui.button("Delete node").on_hover_text(hint).clicked() {
                    self.delete_map_node(id);
                }
            });
        }
    }

    /// One of the level's own nodes (or a runtime one): where it is, read
    /// only.
    fn scene_node_sections(&mut self, ui: &mut egui::Ui, info: Option<&SceneNodeInfo>) {
        let Some(info) = info else { return };
        section(ui, "Node", |ui| {
            property(ui, "Parent", |ui| {
                ui.label(info.parent.clone().unwrap_or_else(|| "none".to_owned()));
            });
            if info.parent.is_some() {
                property(ui, "Placement", |ui| {
                    ui.label(if info.space == Space::Local { "Local (moves with its parent)" } else { "Global (disconnected)" });
                });
            }
            property(ui, "", |ui| {
                ui.label(RichText::new("Not part of the map - add your own with + Camera / + Node. It can still be animated from the timeline.").small().weak());
            });
        });
        section(ui, "Transform", |ui| {
            let (p, r) = (info.world_position, info.world_rotation);
            property(ui, "Global position", |ui| {
                ui.label(format!("{:.1}, {:.1}, {:.1} m", p.x, p.y, p.z));
            });
            property(ui, "Global rotation", |ui| {
                ui.label(format!("{:.1}°, {:.1}°, {:.1}°", r.x, r.y, r.z));
            });
        });
    }

    /// One of the map's nodes, editable: name, parent and placement,
    /// transform, lens, aim.
    fn map_node_sections(&mut self, ui: &mut egui::Ui, id: &str, info: Option<&SceneNodeInfo>) {
        let Some(spec) = self.map.node(id).cloned() else { return };
        let world = info.map(world_of);
        let parent_world = spec.parent.as_deref().and_then(|parent| self.node_info(parent)).map(world_of);
        let descendants = self.map_descendants(id);
        let mut candidates: Vec<String> = self.nodes.iter()
            .filter(|node| node.id != id && !descendants.contains(&node.id) && node.kind != NodeKind::Runtime)
            .map(|node| node.id.clone())
            .collect();
        candidates.sort();
        let mut look_targets = candidates.clone();
        look_targets.extend(descendants.iter().cloned());
        look_targets.sort();

        let mut new_spec = spec.clone();
        let mut rename = None;
        let name_edit = &mut self.node_name_edit;
        section(ui, "Node", |ui| {
            property(ui, "Name", |ui| {
                let buffer = name_edit.get_or_insert_with(|| spec.id.clone());
                let response = ui.add_sized([ui.available_width(), 20.0], egui::TextEdit::singleline(buffer));
                if response.lost_focus() {
                    let new_name = buffer.trim().to_owned();
                    *name_edit = None;
                    if new_name != spec.id {
                        rename = Some(new_name);
                    }
                } else if !response.has_focus() {
                    *buffer = spec.id.clone();
                }
            });
            property(ui, "Parent", |ui| {
                let mut parent = spec.parent.clone();
                egui::ComboBox::from_id_salt("node_parent")
                    .width(ui.available_width())
                    .selected_text(parent.clone().unwrap_or_else(|| "none (top level)".to_owned()))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut parent, None, "none (top level)");
                        for candidate in &candidates {
                            ui.selectable_value(&mut parent, Some(candidate.clone()), candidate.as_str());
                        }
                    });
                if parent != spec.parent {
                    new_spec.parent = parent;
                }
            });
            if spec.parent.is_some() {
                property(ui, "Placement", |ui| {
                    let mut space = spec.space;
                    ui.radio_value(&mut space, Space::Local, "Local").on_hover_text("An offset from the parent's origin - moves with it");
                    ui.radio_value(&mut space, Space::Global, "Global").on_hover_text("A place in the world, disconnected from the parent - stays put whatever it does");
                    new_spec.space = space;
                });
                if spec.space == Space::Local {
                    property(ui, "Follow rotation", |ui| {
                        ui.checkbox(&mut new_spec.follow_rotation, "").on_hover_text("Off: follows the parent's position only, keeping its own rotation");
                    });
                }
            }
        });

        let local = spec.is_local();
        section(ui, "Transform", |ui| {
            let (label_position, label_rotation) = if local { ("Local position", "Local rotation") } else { ("Global position", "Global rotation") };
            property(ui, label_position, |ui| {
                let (x, y, z) = &mut new_spec.position;
                vector_edit(ui, [x, y, z], 0.5, " m");
            });
            property(ui, label_rotation, |ui| {
                let (x, y, z) = &mut new_spec.rotation;
                vector_edit(ui, [x, y, z], 0.5, "°");
            });
            if local {
                if let Some(world) = world {
                    property(ui, "Global position", |ui| {
                        let p = world.position;
                        ui.label(RichText::new(format!("{:.1}, {:.1}, {:.1} m", p.x, p.y, p.z)).weak());
                    });
                }
            } else {
                property(ui, "", |ui| {
                    if ui.add(egui::Button::new("Move to the editor's view").min_size(egui::vec2(ui.available_width(), 0.0))).clicked() {
                        new_spec.position = tuple(self.view_position);
                    }
                });
            }
            if let Some(look_at) = &spec.look_at {
                property(ui, "", |ui| {
                    ui.label(RichText::new(format!("Rotation is overridden - it faces '{look_at}'")).small().weak());
                });
            }
        });

        section(ui, "Camera", |ui| {
            match &mut new_spec.camera {
                Some(camera) => {
                    property(ui, "Field of view", |ui| {
                        ui.spacing_mut().slider_width = (ui.available_width() - 60.0).max(80.0);
                        ui.add(egui::Slider::new(&mut camera.fov, 5.0..=150.0).suffix("°").max_decimals(1));
                    });
                    property(ui, "Near plane", |ui| {
                        ui.add_sized([ui.available_width(), 20.0], egui::DragValue::new(&mut camera.near).speed(0.01).range(0.01..=100.0).suffix(" m"));
                    });
                    property(ui, "", |ui| {
                        if ui.add(egui::Button::new("Not a camera").min_size(egui::vec2(ui.available_width(), 0.0))).clicked() {
                            new_spec.camera = None;
                        }
                    });
                }
                None => property(ui, "", |ui| {
                    if ui.add(egui::Button::new("Make it a camera").min_size(egui::vec2(ui.available_width(), 0.0))).clicked() {
                        new_spec.camera = Some(CameraSpec { fov: NEW_CAMERA_FOV, near: 0.1 });
                    }
                }),
            }
            property(ui, "Look at", |ui| {
                egui::ComboBox::from_id_salt("node_look_at")
                    .width(ui.available_width())
                    .selected_text(spec.look_at.clone().unwrap_or_else(|| "nothing".to_owned()))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut new_spec.look_at, None, "nothing");
                        for target in &look_targets {
                            ui.selectable_value(&mut new_spec.look_at, Some(target.clone()), target.as_str());
                        }
                    });
            });
        });

        // A new parent or placement keeps it where it is - its position and
        // rotation worked out again in the new space.
        let placement_changed = new_spec.parent != spec.parent || new_spec.space != spec.space || new_spec.follow_rotation != spec.follow_rotation;
        if placement_changed {
            if let Some(world) = world {
                let new_parent_world = new_spec.parent.as_deref().and_then(|parent| self.node_info(parent)).map(world_of);
                let placed = match (new_spec.is_local(), new_parent_world) {
                    (true, Some(parent)) => parent.relative(&world, new_spec.follow_rotation),
                    _ => world,
                };
                new_spec.position = tuple(placed.position);
                new_spec.rotation = tuple(degrees_from_rotation(&placed.rotation));
            } else if let (false, Some(parent)) = (new_spec.is_local(), parent_world) {
                let placed = parent.compose(&Transform3D { position: spec.position(), rotation: spec.rotation(), scale: Vector3::new(1.0, 1.0, 1.0) }, spec.follow_rotation);
                new_spec.position = tuple(placed.position);
            }
        }
        if new_spec != spec {
            if let Some(node) = self.map.node_mut(id) {
                *node = new_spec;
            }
        }
        if let Some(name) = rename {
            self.rename_map_node(id, &name);
        }
    }

    /// Renames the map's node `old` to `new` - and everything that names
    /// it: children's parents, aims, camera cuts, its tracks, and the
    /// "parent"/"look_at" keys that point at it.
    fn rename_map_node(&mut self, old: &str, new: &str) {
        if !self.id_free(new) {
            self.status = Some((format!("Can't rename to '{new}' - names must be unique (aircraft too), not empty, without '/'"), true));
            return;
        }
        for node in self.map.nodes.iter_mut() {
            if node.id == old {
                node.id = new.to_owned();
            }
            if node.parent.as_deref() == Some(old) {
                node.parent = Some(new.to_owned());
            }
            if node.look_at.as_deref() == Some(old) {
                node.look_at = Some(new.to_owned());
            }
        }
        // Its animations move with it; camera keys and events that name it
        // follow.
        if let Some(animations) = self.map.animations.remove(old) {
            self.map.animations.insert(new.to_owned(), animations);
        }
        for animation in self.map.animations.values_mut().flatten() {
            for key in animation.camera.iter_mut() {
                if key.follow.as_deref() == Some(old) {
                    key.follow = Some(new.to_owned());
                }
                if key.look_at.as_deref() == Some(old) {
                    key.look_at = Some(new.to_owned());
                }
            }
            for event in animation.events.iter_mut() {
                match &mut event.action {
                    AnimAction::StartAnimation { node, .. } | AnimAction::StopAnimation { node, .. } if node == old => *node = new.to_owned(),
                    _ => {}
                }
            }
        }
        if let Some((node, _)) = &mut self.current_animation {
            if node == old {
                *node = new.to_owned();
            }
        }
        if self.preview_camera.as_deref() == Some(old) {
            self.preview_camera = Some(new.to_owned());
        }
        self.timeline_selection = Default::default();
        self.select(Selection::Node(new.to_owned()));
    }

    /// Takes the map's node `id` out - its child nodes, their tracks and the
    /// camera cuts to any of them go too; aims at them are dropped.
    fn delete_map_node(&mut self, id: &str) {
        let mut gone = self.map_descendants(id);
        gone.push(id.to_owned());
        self.map.nodes.retain(|node| !gone.contains(&node.id));
        for node in self.map.nodes.iter_mut() {
            if node.look_at.as_ref().is_some_and(|target| gone.contains(target)) {
                node.look_at = None;
            }
        }
        // Their animations go; camera keys stop following or looking at them,
        // events that start or stop theirs go.
        self.map.animations.retain(|node, _| !gone.contains(node));
        for animation in self.map.animations.values_mut().flatten() {
            for key in animation.camera.iter_mut() {
                if key.follow.as_ref().is_some_and(|follow| gone.contains(follow)) {
                    key.follow = None;
                }
                if key.look_at.as_ref().is_some_and(|target| gone.contains(target)) {
                    key.look_at = None;
                }
            }
            animation.events.retain(|event| !matches!(&event.action, AnimAction::StartAnimation { node, .. } | AnimAction::StopAnimation { node, .. } if gone.contains(node)));
        }
        if self.current_animation.as_ref().is_some_and(|(node, _)| gone.contains(node)) {
            self.current_animation = None;
        }
        if self.preview_camera.as_ref().is_some_and(|camera| gone.contains(camera)) {
            self.preview_camera = None;
        }
        self.timeline_selection = Default::default();
        self.select(Selection::None);
    }

    /// The node's animations: a dropdown of them - the one picked shows on
    /// the timeline below, where it's edited - and "Create Animation".
    pub(super) fn animations_section(&mut self, ui: &mut egui::Ui, id: &str) {
        let names: Vec<String> = self.map.animations.get(id).map(|list| list.iter().map(|animation| animation.name.clone()).collect()).unwrap_or_default();
        let shown = self.current_animation.as_ref().filter(|(node, _)| node == id).map(|(_, name)| name.clone());
        let mut show = None;
        let mut create = false;
        section(ui, "Animations", |ui| {
            property(ui, "Animation", |ui| {
                egui::ComboBox::from_id_salt("node_animations")
                    .width(ui.available_width())
                    .selected_text(shown.clone().unwrap_or_else(|| if names.is_empty() { "none yet".to_owned() } else { "pick one".to_owned() }))
                    .show_ui(ui, |ui| {
                        for name in &names {
                            if ui.selectable_label(shown.as_deref() == Some(name.as_str()), name).clicked() {
                                show = Some(name.clone());
                            }
                        }
                    })
                    .response
                    .on_hover_text("The one picked shows on the timeline below");
            });
            if ui.add(egui::Button::new(RichText::new("Create Animation").strong().size(14.0)).min_size(egui::vec2(ui.available_width(), 30.0)))
                .on_hover_text("A new animation for it - made on the timeline below").clicked() {
                create = true;
            }
        });
        if let Some(name) = show {
            self.show_animation(id, &name);
        }
        if create {
            self.create_animation(id);
        }
    }

    /// A map node's own path - followed (animated) from the level's start
    /// whenever no animation has it following another.
    fn default_path_section(&mut self, ui: &mut egui::Ui, id: &str) {
        let paths: Vec<String> = self.map.paths.iter().map(|path| path.name.clone()).collect();
        let Some(spec) = self.map.node_mut(id) else { return };
        section(ui, "Path", |ui| {
            property(ui, "Follows", |ui| {
                let mut path = spec.follow.as_ref().map(|follow| follow.path.clone());
                egui::ComboBox::from_id_salt("node_default_path")
                    .width(ui.available_width())
                    .selected_text(path.clone().unwrap_or_else(|| "nothing".to_owned()))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut path, None, "nothing");
                        for name in &paths {
                            ui.selectable_value(&mut path, Some(name.clone()), name.as_str());
                        }
                    })
                    .response
                    .on_hover_text("A path it follows from the level's start (see the Paths window) - an animation's follow takes over while it runs");
                match (path, &mut spec.follow) {
                    (None, follow) => *follow = None,
                    (Some(path), Some(follow)) => follow.path = path,
                    (Some(path), follow) => *follow = Some(DefaultFollow { path, pace: Pace::Speed(60.0), orient: true }),
                }
            });
            if let Some(follow) = &mut spec.follow {
                property(ui, "Pace", |ui| {
                    let mut kind = follow.pace.label();
                    egui::ComboBox::from_id_salt("node_default_pace")
                        .selected_text(kind)
                        .show_ui(ui, |ui| {
                            for option in ["Speed", "Duration", "By skill"] {
                                ui.selectable_value(&mut kind, option, option);
                            }
                        });
                    if kind != follow.pace.label() {
                        follow.pace = match kind {
                            "Speed" => Pace::Speed(60.0),
                            "Duration" => Pace::Duration(10.0),
                            _ => Pace::BySkill,
                        };
                    }
                    match &mut follow.pace {
                        Pace::Speed(knots) => { ui.add(egui::DragValue::new(knots).speed(1.0).range(0.0..=2000.0).suffix(" kt")); }
                        Pace::Duration(seconds) => { ui.add(egui::DragValue::new(seconds).speed(0.1).range(0.1..=3600.0).suffix(" s")); }
                        Pace::BySkill => {}
                    }
                });
                property(ui, "Nose along path", |ui| {
                    ui.checkbox(&mut follow.orient, "");
                });
            }
        });
    }
}
