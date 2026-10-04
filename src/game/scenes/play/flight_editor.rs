//! The flight editor - the level's map (see play::map::MapSpec), editable
//! while flying, docked around F10's map in its editor view (see
//! `draw_docked`). Laid out like an engine's editor: on the left a hierarchy
//! - every coalition, each with its aircraft beneath it; on the right an
//! inspector for whatever is selected, in collapsible sections of
//! label/value rows; along the bottom, the timeline. An aircraft's loadout
//! has a window of its own; its path is F10's Flight plan window.
//!
//! It only edits its own copy of the map (`map`) and lists what else it was
//! asked to do (`requests`); the play scene applies both every frame (see
//! play::scene::GameLogic::update_flight_editor and flight_manager::sync), so
//! a change shows up in the world right away. Drawn by the egui overlay pass
//! (see render_pass.rs), which is why it lives on `App`.

use std::collections::{HashMap, HashSet};

use egui::{Color32, RichText};
use nalgebra::Vector3;

use crate::game::scenes::play::flight_manager::LiveAircraft;
use crate::game::scenes::play::map::{AircraftPlacement, Coalition, Difficulty, MapSpec};
use crate::game::scenes::play::plane::pylon_spec::PylonSpec;
use crate::game::scenes::play::weapon::weapon_spec::WeaponCategory;
use crate::game::scenes::play::tactical_ui::TacticalUi;

/// Something the editor was asked to do beyond editing the map - for the
/// play scene to carry out (see GameLogic::update_flight_editor).
pub enum EditorRequest {
    /// Write the map to the level's map.ron.
    Save,
    /// Throw away the edits: the map as map.ron has it (applied live too).
    ReloadFromFile,
    /// Spawn every aircraft again at its place in the map.
    RespawnAll,
    /// Spawn this aircraft again at its place in the map.
    Respawn(String),
    /// Make where this aircraft is right now its place in the map.
    CaptureCurrent(String),
    /// Load this aircraft again as the map has it, where it is: every
    /// weapon back on, flares and chaff full.
    ResetLoadout(String),
}

/// What the hierarchy has selected - shown in the inspector.
#[derive(Default, Clone, PartialEq)]
enum Selection {
    #[default]
    None,
    /// A coalition, by its index in `map.coalitions` (its name can be
    /// mid-edit).
    Coalition(usize),
    /// An aircraft, by name.
    Aircraft(String),
}

#[derive(Default)]
pub struct FlightEditor {
    /// The map being edited - applied to the world every frame.
    pub map: MapSpec,
    /// Every plane folder under assets/planes/ that loads - what an
    /// aircraft's `plane` can be.
    pub planes: Vec<String>,
    /// Each plane's pylons, by plane - what an aircraft's loadout can fill.
    pub pylons: HashMap<String, Vec<PylonSpec>>,
    /// How many flares and chaff each plane carries by its data.ron, by
    /// plane - none for one without dispensers.
    pub countermeasures: HashMap<String, (u32, u32)>,
    /// Every weapon that loads, by id, with its category - what a pylon can
    /// carry (see `PylonSpec::accepts`).
    pub weapons: Vec<(String, WeaponCategory)>,
    /// Each aircraft as it is right now, by name - filled every frame.
    pub live: HashMap<String, LiveAircraft>,
    /// The last thing done, or what's wrong with the map - `true` for an
    /// error.
    pub status: Option<(String, bool)>,
    pub requests: Vec<EditorRequest>,
    /// A text field has the keyboard - flight keys are held off meanwhile
    /// (see GameLogic::update).
    pub typing: bool,
    selection: Selection,
    /// The selected aircraft's name as it's being typed - only applied once
    /// the field is left (Enter or clicking away), since a new name means
    /// spawning the aircraft again.
    name_edit: Option<String>,
    /// Coalitions folded shut in the hierarchy, by index.
    folded: HashSet<usize>,
    /// The Loadout window is up (see `loadout_window`).
    loadout_open: bool,
    /// The aircraft F10's map and this agreed was selected, last frame - so
    /// a new pick on either side carries over to the other (see
    /// `draw_docked`).
    synced_selection: Option<String>,
}

/// A change to the map's structure, made after drawing it (the map can't
/// change under the code drawing it).
enum Change {
    AddCoalition,
    RemoveCoalition(usize),
    AddAircraft(usize),
    RemoveAircraft(usize, usize),
    MoveAircraft { from: usize, index: usize, to: usize },
    MakePlayer(usize, usize),
    Rename { coalition: usize, index: usize, name: String },
}

/// Knots to m/s.
const KT_TO_MS: f32 = 0.514_444;

/// How far "+ Add point" puts a new path point past the last one (m).
const PATH_STEP: f32 = 1000.0;
/// The padding inside each part of the panels (points) - the toolbar, the
/// hierarchy's buttons, each of the inspector's blocks. The panels
/// themselves have none, so their lines run edge to edge.
const PAD: i8 = 8;
/// A panel's header bar's height (points).
const PANEL_HEADER_HEIGHT: f32 = 20.0;
/// An inspector row's label column (points).
const LABEL_WIDTH: f32 = 110.0;
/// A hierarchy row's height, and how far an aircraft sits in from its
/// coalition (points).
const ROW_HEIGHT: f32 = 22.0;
const INDENT: f32 = 18.0;

const ERROR_COLOR: Color32 = Color32::from_rgb(255, 110, 100);
const OK_COLOR: Color32 = Color32::from_rgb(140, 220, 140);
/// Gold - the player's aircraft icon.
const PLAYER_COLOR: Color32 = Color32::from_rgb(255, 210, 90);
/// Vector fields' axis colors - X red, Y green, Z blue, as in Godot.
const AXIS_COLORS: [Color32; 3] = [Color32::from_rgb(230, 85, 85), Color32::from_rgb(120, 200, 70), Color32::from_rgb(90, 140, 240)];

/// `contents` with the padding each part of the window has - see PAD.
fn padded<R>(ui: &mut egui::Ui, contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new().inner_margin(egui::Margin::symmetric(PAD, 6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        contents(ui)
    }).inner
}

/// The "Properties" header: a highlighted bar the width of the column,
/// with a line under it.
fn panel_header(ui: &mut egui::Ui, title: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), PANEL_HEADER_HEIGHT), egui::Sense::hover());
    let visuals = ui.visuals();
    ui.painter().rect_filled(rect, 0.0, visuals.widgets.inactive.bg_fill);
    let font = egui::FontId::proportional(13.0);
    ui.painter().text(rect.left_center() + egui::vec2(PAD as f32, 0.0), egui::Align2::LEFT_CENTER, title, font, visuals.strong_text_color());
    ui.painter().hline(rect.x_range(), rect.bottom(), visuals.widgets.noninteractive.bg_stroke);
}

fn coalition_color(coalition: &Coalition) -> Color32 {
    Color32::from_rgb(coalition.color.0, coalition.color.1, coalition.color.2)
}

/// A faint line across the whole width - between rows.
fn row_line(ui: &mut egui::Ui) {
    let y = ui.cursor().top();
    let stroke = egui::Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color.gamma_multiply(0.6));
    ui.painter().hline(ui.max_rect().x_range(), y, stroke);
}

impl FlightEditor {
    /// The editor docked around F10's map - its editor view, laid out like
    /// an engine's editor: the hierarchy (every coalition and its aircraft,
    /// adding either) down the left, the selected thing's properties down
    /// the right, the timeline (for the animations, to come) along the
    /// bottom, each under its own header; the map in the middle. The map's
    /// and the hierarchy's selection are one (`tactical`'s Flight plan
    /// window shows the selected aircraft's path). Returns where the map
    /// goes - what the panels leave.
    pub fn draw_docked(&mut self, ctx: &egui::Context, tactical: &mut TacticalUi) -> egui::Rect {
        const HIERARCHY_PANEL_WIDTH: f32 = 260.0;
        const PROPERTIES_PANEL_WIDTH: f32 = 380.0;
        const TIMELINE_HEIGHT: f32 = 160.0;

        // Picked on the map since last frame: picked here too.
        if tactical.selected != self.synced_selection {
            self.synced_selection = tactical.selected.clone();
            let selection = match &tactical.selected {
                Some(name) => Selection::Aircraft(name.clone()),
                None => Selection::None,
            };
            if selection != self.selection {
                self.select(selection);
            }
        }
        self.drop_stale_selection();

        // Edge to edge - each part pads itself, under its header.
        let frame = egui::Frame::side_top_panel(&ctx.style()).inner_margin(egui::Margin::ZERO);
        let mut changes = Vec::new();
        egui::SidePanel::left("editor_hierarchy")
            .frame(frame)
            .resizable(true)
            .default_width(HIERARCHY_PANEL_WIDTH)
            .min_width(200.0)
            .show(ctx, |ui| {
                panel_header(ui, "Hierarchy");
                padded(ui, |ui| self.toolbar(ui));
                self.hierarchy(ui, &mut changes);
            });
        egui::SidePanel::right("editor_properties")
            .frame(frame)
            .resizable(true)
            .default_width(PROPERTIES_PANEL_WIDTH)
            .min_width(320.0)
            .show(ctx, |ui| {
                panel_header(ui, "Properties");
                self.inspector(ui, &mut changes);
            });
        egui::TopBottomPanel::bottom("editor_timeline")
            .frame(frame)
            .resizable(true)
            .default_height(TIMELINE_HEIGHT)
            .min_height(60.0)
            .show(ctx, |ui| {
                panel_header(ui, "Timeline");
                ui.centered_and_justified(|ui| {
                    ui.label(RichText::new("The animation timeline goes here").weak());
                });
            });
        for change in changes {
            self.apply(change);
        }
        let map = ctx.available_rect();
        self.loadout_window(ctx, map);

        // Picked here: picked on the map too (its Flight plan window).
        let selected = self.selected_aircraft().map(str::to_owned);
        if selected != self.synced_selection {
            self.synced_selection = selected.clone();
            tactical.selected = selected;
        }
        self.typing = ctx.wants_keyboard_input();
        map
    }

    /// The selected aircraft's loadout in a window of its own, floating
    /// (over `area`'s top left to start with), while `loadout_open` - see
    /// `loadout_editor`.
    fn loadout_window(&mut self, ctx: &egui::Context, area: egui::Rect) {
        if !self.loadout_open {
            return;
        }
        let Some(name) = self.selected_aircraft().map(str::to_owned) else { return };
        let Some((coalition_index, index)) = self.map.coalitions.iter().enumerate()
            .find_map(|(coalition_index, coalition)| coalition.aircraft.iter().position(|aircraft| aircraft.name == name).map(|index| (coalition_index, index)))
        else { return };
        let plane = self.map.coalitions[coalition_index].aircraft[index].plane.clone();
        let pylons = self.pylons.get(&plane).cloned().unwrap_or_default();
        let plane_countermeasures = self.countermeasures.get(&plane).copied();
        let live = self.live.get(&name).copied();
        let aircraft = &mut self.map.coalitions[coalition_index].aircraft[index];
        let (weapons, requests) = (&self.weapons, &mut self.requests);

        let mut open = true;
        egui::Window::new(format!("Loadout - {name}"))
            .id(egui::Id::new("editor_loadout"))
            .open(&mut open)
            .default_pos(area.left_top() + egui::vec2(16.0, 56.0))
            .default_width(360.0)
            .resizable(false)
            .collapsible(true)
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.label(RichText::new(format!("{} · {}", aircraft.plane, if aircraft.player { "player" } else { "bot" })).weak());
                ui.add_space(4.0);
                loadout_editor(ui, &name, aircraft, &pylons, weapons, plane_countermeasures, live, requests);
            });
        if !open {
            self.loadout_open = false;
        }
    }

    /// The map can change between frames (a reload, a coalition deleted) -
    /// keeps the selection pointing at something real.
    fn drop_stale_selection(&mut self) {
        let stale = match &self.selection {
            Selection::None => false,
            Selection::Coalition(index) => *index >= self.map.coalitions.len(),
            Selection::Aircraft(name) => self.map.find(name).is_none(),
        };
        if stale {
            self.select(Selection::None);
        }
    }

    /// The selected thing's properties, scrolling under the "Properties"
    /// header.
    fn inspector(&mut self, ui: &mut egui::Ui, changes: &mut Vec<Change>) {
        egui::ScrollArea::vertical().id_salt("inspector").auto_shrink([false, false]).show(ui, |ui| {
            match self.selection.clone() {
                Selection::None => padded(ui, |ui| {
                    ui.add_space(8.0);
                    ui.label(RichText::new("Select a coalition or an aircraft in the hierarchy").weak());
                }),
                Selection::Coalition(index) => self.coalition_inspector(ui, index, changes),
                Selection::Aircraft(name) => self.aircraft_inspector(ui, &name, changes),
            }
            ui.add_space(8.0);
        });
    }

    /// The aircraft selected in the hierarchy, by name - the one the camera
    /// follows (see play::scene::GameLogic::update).
    pub fn selected_aircraft(&self) -> Option<&str> {
        match &self.selection {
            Selection::Aircraft(name) => Some(name),
            _ => None,
        }
    }

    fn select(&mut self, selection: Selection) {
        self.selection = selection;
        self.name_edit = None;
    }

    /// The coalition new aircraft go into - the selected one, or the
    /// selected aircraft's, or the first.
    fn target_coalition(&self) -> usize {
        match &self.selection {
            Selection::Coalition(index) => *index,
            Selection::Aircraft(name) => self.map.coalitions.iter()
                .position(|coalition| coalition.aircraft.iter().any(|aircraft| aircraft.name == *name))
                .unwrap_or(0),
            Selection::None => 0,
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            if ui.button("Save to map.ron").clicked() {
                self.requests.push(EditorRequest::Save);
            }
            if ui.button("Reload file").on_hover_text("Throw away the edits - back to what map.ron has").clicked() {
                self.requests.push(EditorRequest::ReloadFromFile);
            }
            if ui.button("Respawn all").on_hover_text("Every aircraft back at its place, fresh").clicked() {
                self.requests.push(EditorRequest::RespawnAll);
            }
        });
        if let Some((message, error)) = &self.status {
            ui.label(RichText::new(message).color(if *error { ERROR_COLOR } else { OK_COLOR }));
        }
    }

    /// Every coalition as a tree root, its aircraft beneath it - click a row
    /// to inspect it, a coalition's arrow to fold it.
    fn hierarchy(&mut self, ui: &mut egui::Ui, changes: &mut Vec<Change>) {
        padded(ui, |ui| ui.horizontal(|ui| {
            let target = self.target_coalition();
            let add = ui.add_enabled(!self.map.coalitions.is_empty(), egui::Button::new("+ Add aircraft"))
                .on_hover_text(self.map.coalitions.get(target).map(|coalition| format!("Into {}", coalition.name)).unwrap_or_default());
            if add.clicked() {
                changes.push(Change::AddAircraft(target));
            }
            if ui.button("+ Coalition").clicked() {
                changes.push(Change::AddCoalition);
            }
        }));

        let visuals = ui.visuals().clone();
        let mut clicked = None;
        let mut fold = None;
        // The table itself, edge to edge - square, no margin - filling the
        // rest of the column; its rows scroll inside it.
        egui::Frame::new()
            .fill(visuals.extreme_bg_color)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical().id_salt("hierarchy").auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    if self.map.coalitions.is_empty() {
                        ui.label(RichText::new("  No coalitions").weak());
                    }
                    for (coalition_index, coalition) in self.map.coalitions.iter().enumerate() {
                        if coalition_index > 0 {
                            row_line(ui);
                        }
                        let open = !self.folded.contains(&coalition_index);
                        let root = TreeRow {
                            depth: 0,
                            fold: Some(open),
                            icon: Icon::Coalition,
                            icon_color: coalition_color(coalition),
                            text: coalition.name.clone(),
                            text_color: None,
                            detail: coalition.aircraft.len().to_string(),
                        };
                        match tree_row(ui, self.selection == Selection::Coalition(coalition_index), root) {
                            RowClick::Fold => fold = Some(coalition_index),
                            RowClick::Select => clicked = Some(Selection::Coalition(coalition_index)),
                            RowClick::None => {}
                        }
                        if !open {
                            continue;
                        }
                        for aircraft in &coalition.aircraft {
                            row_line(ui);
                            let live = self.live.get(&aircraft.name);
                            let row = TreeRow {
                                depth: 1,
                                fold: None,
                                icon: Icon::Aircraft,
                                icon_color: if aircraft.player { PLAYER_COLOR } else { visuals.weak_text_color() },
                                text: aircraft.name.clone(),
                                text_color: match live {
                                    None => Some(ERROR_COLOR),
                                    Some(live) if live.wrecked => Some(visuals.weak_text_color()),
                                    Some(_) => None,
                                },
                                detail: if live.is_some_and(|live| live.wrecked) { format!("{} · wrecked", aircraft.plane) } else { aircraft.plane.clone() },
                            };
                            let selected = self.selection == Selection::Aircraft(aircraft.name.clone());
                            if tree_row(ui, selected, row) == RowClick::Select {
                                clicked = Some(Selection::Aircraft(aircraft.name.clone()));
                            }
                        }
                    }
                });
            });
        if let Some(index) = fold {
            if !self.folded.remove(&index) {
                self.folded.insert(index);
            }
        }
        if let Some(selection) = clicked {
            if selection != self.selection {
                self.select(selection);
            }
        }
    }

    /// The selected coalition: its name, color, and deleting it.
    fn coalition_inspector(&mut self, ui: &mut egui::Ui, index: usize, changes: &mut Vec<Change>) {
        let coalition = &self.map.coalitions[index];
        let has_player = coalition.aircraft.iter().any(|aircraft| aircraft.player);
        let count = coalition.aircraft.len();
        inspector_header(ui, Icon::Coalition, coalition_color(coalition), &coalition.name, |ui| {
            let mut text = if count == 1 { "1 aircraft".to_owned() } else { format!("{count} aircraft") };
            if has_player {
                text.push_str(" · the player's");
            }
            ui.label(RichText::new(text).small().weak());
        }, |_| {});

        let coalition = &mut self.map.coalitions[index];
        section(ui, "Coalition", |ui| {
            property(ui, "Name", |ui| {
                ui.add_sized([ui.available_width(), 20.0], egui::TextEdit::singleline(&mut coalition.name));
            });
            property(ui, "Color", |ui| {
                let mut color = [coalition.color.0, coalition.color.1, coalition.color.2];
                if ui.color_edit_button_srgb(&mut color).changed() {
                    coalition.color = (color[0], color[1], color[2]);
                }
                ui.label(RichText::new(format!("{}, {}, {}", color[0], color[1], color[2])).weak());
            });
            property(ui, "Aircraft", |ui| {
                ui.label(count.to_string());
            });
        });

        padded(ui, |ui| {
            let remove = ui.add_enabled(!has_player, egui::Button::new("Delete coalition"))
                .on_hover_text("Deletes its aircraft too")
                .on_disabled_hover_text("The player's aircraft is in it - make another aircraft the player first");
            if remove.clicked() {
                changes.push(Change::RemoveCoalition(index));
            }
        });
    }

    /// The selected aircraft, by section: the aircraft itself, its team,
    /// its transform, how it starts flying, and how it is right now.
    fn aircraft_inspector(&mut self, ui: &mut egui::Ui, name: &str, changes: &mut Vec<Change>) {
        let Some((coalition_index, index)) = self.map.coalitions.iter().enumerate()
            .find_map(|(coalition_index, coalition)| coalition.aircraft.iter().position(|aircraft| aircraft.name == name).map(|index| (coalition_index, index)))
        else { return };
        let planes = self.planes.clone();
        let plane_of_aircraft = &self.map.coalitions[coalition_index].aircraft[index].plane;
        let pylons = self.pylons.get(plane_of_aircraft).cloned().unwrap_or_default();
        let plane_countermeasures = self.countermeasures.get(plane_of_aircraft).copied();
        let coalitions: Vec<(String, Color32)> = self.map.coalitions.iter().map(|coalition| (coalition.name.clone(), coalition_color(coalition))).collect();
        let live = self.live.get(name).copied();
        let aircraft = &mut self.map.coalitions[coalition_index].aircraft[index];
        let requests = &mut self.requests;
        let loadout_open = &mut self.loadout_open;

        let icon_color = if aircraft.player { PLAYER_COLOR } else { ui.visuals().text_color() };
        inspector_header(ui, Icon::Aircraft, icon_color, &aircraft.name, |ui| status_line(ui, live), |ui| {
            if ui.button("Respawn").on_hover_text("Back at its place in the map, fresh").clicked() {
                requests.push(EditorRequest::Respawn(name.to_owned()));
            }
        });

        let name_edit = &mut self.name_edit;
        section(ui, "Aircraft", |ui| {
            property(ui, "Name", |ui| {
                let buffer = name_edit.get_or_insert_with(|| aircraft.name.clone());
                let response = ui.add_sized([ui.available_width(), 20.0], egui::TextEdit::singleline(buffer));
                if response.lost_focus() {
                    let new_name = buffer.trim().to_owned();
                    *name_edit = None;
                    if new_name != aircraft.name {
                        changes.push(Change::Rename { coalition: coalition_index, index, name: new_name });
                    }
                } else if !response.has_focus() {
                    // Not being typed in - follows the map.
                    *buffer = aircraft.name.clone();
                }
            });
            property(ui, "Plane", |ui| {
                egui::ComboBox::from_id_salt("plane")
                    .width(ui.available_width())
                    .selected_text(aircraft.plane.as_str())
                    .show_ui(ui, |ui| {
                        for plane in &planes {
                            ui.selectable_value(&mut aircraft.plane, plane.clone(), plane.as_str());
                        }
                    });
            });
            property(ui, "Is player", |ui| {
                // Only switched on here - there's always exactly one player,
                // so it's switched off by making another aircraft it.
                let mut is_player = aircraft.player;
                let response = ui.add_enabled(!aircraft.player, egui::Checkbox::without_text(&mut is_player))
                    .on_disabled_hover_text("Make another aircraft the player to switch this off");
                if response.changed() && is_player {
                    changes.push(Change::MakePlayer(coalition_index, index));
                }
            });
        });

        if !aircraft.player {
            section(ui, "Pilot", |ui| {
                property(ui, "Difficulty", |ui| {
                    egui::ComboBox::from_id_salt("difficulty")
                        .width(ui.available_width())
                        .selected_text(aircraft.difficulty.label())
                        .show_ui(ui, |ui| {
                            for difficulty in Difficulty::ALL {
                                ui.selectable_value(&mut aircraft.difficulty, difficulty, difficulty.label());
                            }
                        })
                        .response
                        .on_hover_text("How quickly it reacts to a missile fired at it - with flares");
                });
            });
        }

        section(ui, "Team", |ui| {
            property(ui, "Coalition", |ui| {
                let mut to = coalition_index;
                egui::ComboBox::from_id_salt("coalition")
                    .width(ui.available_width())
                    .selected_text(RichText::new(&coalitions[coalition_index].0).color(coalitions[coalition_index].1))
                    .show_ui(ui, |ui| {
                        for (other, (coalition_name, color)) in coalitions.iter().enumerate() {
                            ui.selectable_value(&mut to, other, RichText::new(coalition_name).color(*color));
                        }
                    });
                if to != coalition_index {
                    changes.push(Change::MoveAircraft { from: coalition_index, index, to });
                }
            });
        });

        section(ui, "Transform", |ui| {
            property(ui, "Position", |ui| {
                vector_field(ui, [&mut aircraft.position.0, &mut aircraft.position.1, &mut aircraft.position.2], 1.0, " m");
            });
            property(ui, "Rotation", |ui| {
                vector_field(ui, [&mut aircraft.rotation.0, &mut aircraft.rotation.1, &mut aircraft.rotation.2], 0.5, "°");
            });
            property(ui, "", |ui| {
                let capture = ui.add_enabled(live.is_some(), egui::Button::new("Use current position").min_size(egui::vec2(ui.available_width(), 0.0)))
                    .on_hover_text("Make where it is now (position and attitude) its place in the map");
                if capture.clicked() {
                    requests.push(EditorRequest::CaptureCurrent(name.to_owned()));
                }
            });
        });

        section(ui, "Flight", |ui| {
            property(ui, "Set speed", |ui| {
                let mut has_speed = aircraft.speed.is_some();
                if ui.checkbox(&mut has_speed, "").on_hover_text("Off: the plane's data.ron initial_velocity").changed() {
                    aircraft.speed = has_speed.then_some(aircraft.speed.unwrap_or(0.0));
                }
            });
            property(ui, "Speed", |ui| {
                match &mut aircraft.speed {
                    Some(speed) => {
                        // Shown and typed in knots - kept in m/s (the map
                        // file, the physics).
                        let mut knots = *speed / KT_TO_MS;
                        let response = ui.add_sized([ui.available_width(), 20.0], egui::DragValue::new(&mut knots).speed(1.0).range(0.0..=2000.0).suffix(" kt").max_decimals(0))
                            .on_hover_text(format!("{:.1} m/s", *speed));
                        if response.changed() {
                            *speed = knots * KT_TO_MS;
                        }
                    }
                    None => {
                        ui.label(RichText::new("from data.ron").weak());
                    }
                }
            });
            property(ui, "Throttle", |ui| {
                // Never narrower than a usable rail - a slider with no rail
                // length trips egui's remap assertion (and panics).
                ui.spacing_mut().slider_width = (ui.available_width() - 50.0).max(80.0);
                ui.add(egui::Slider::new(&mut aircraft.throttle, 0.0..=1.0).max_decimals(2));
            });
        });

        section(ui, "Loadout", |ui| {
            property(ui, "Weapons", |ui| {
                let text = match (aircraft.loadout.len(), pylons.len()) {
                    (_, 0) => "No pylons".to_owned(),
                    (0, pylons) => format!("Empty - {pylons} pylons"),
                    (hung, pylons) => format!("{hung} on {pylons} pylons"),
                };
                ui.label(text);
            });
            if let Some((plane_flares, plane_chaff)) = plane_countermeasures {
                property(ui, "Flares / chaff", |ui| {
                    ui.label(format!("{} / {}", aircraft.flares.unwrap_or(plane_flares), aircraft.chaff.unwrap_or(plane_chaff)));
                });
            }
            property(ui, "", |ui| {
                let text = if *loadout_open { "Hide loadout window" } else { "Edit loadout…" };
                if ui.add(egui::Button::new(text).min_size(egui::vec2(ui.available_width(), 0.0))).on_hover_text("Its weapons, flares and chaff - in a window of their own").clicked() {
                    *loadout_open = !*loadout_open;
                }
            });
        });

        section(ui, "Status", |ui| {
            match live {
                Some(live) => {
                    property(ui, "Position", |ui| {
                        ui.label(format!("{:.0}, {:.0}, {:.0}", live.position.x, live.position.y, live.position.z));
                    });
                    property(ui, "Speed", |ui| {
                        ui.label(format!("{:.0} kt", live.speed_kt));
                    });
                    property(ui, "Altitude", |ui| {
                        ui.label(format!("{:.0} m", live.altitude));
                    });
                    property(ui, "State", |ui| {
                        if live.wrecked {
                            ui.label(RichText::new("Wrecked").color(ERROR_COLOR));
                        } else {
                            ui.label(RichText::new("Flying").color(OK_COLOR));
                        }
                    });
                }
                None => property(ui, "State", |ui| {
                    ui.label(RichText::new("Not in the scene").color(ERROR_COLOR));
                }),
            }
        });

        padded(ui, |ui| {
            let remove = ui.add_enabled(!aircraft.player, egui::Button::new("Delete aircraft"))
                .on_disabled_hover_text("Make another aircraft the player first");
            if remove.clicked() {
                changes.push(Change::RemoveAircraft(coalition_index, index));
            }
        });
    }

    fn apply(&mut self, change: Change) {
        match change {
            Change::AddCoalition => {
                let name = unique_name("Coalition", |name| self.map.coalitions.iter().any(|coalition| coalition.name == name));
                self.map.coalitions.push(Coalition { name, color: (200, 200, 200), aircraft: Vec::new() });
                self.select(Selection::Coalition(self.map.coalitions.len() - 1));
            }
            Change::RemoveCoalition(coalition) => {
                self.map.coalitions.remove(coalition);
                // Folds are by index - the ones after it moved up one.
                self.folded = self.folded.iter()
                    .filter(|folded| **folded != coalition)
                    .map(|folded| if *folded > coalition { folded - 1 } else { *folded })
                    .collect();
                self.select(Selection::None);
            }
            Change::AddAircraft(coalition) => {
                let aircraft = self.new_aircraft();
                self.select(Selection::Aircraft(aircraft.name.clone()));
                self.map.coalitions[coalition].aircraft.push(aircraft);
                // Shown in the hierarchy.
                self.folded.remove(&coalition);
            }
            Change::RemoveAircraft(coalition, index) => {
                self.map.coalitions[coalition].aircraft.remove(index);
                self.select(Selection::None);
            }
            Change::MoveAircraft { from, index, to } => {
                let aircraft = self.map.coalitions[from].aircraft.remove(index);
                self.map.coalitions[to].aircraft.push(aircraft);
                self.folded.remove(&to);
            }
            Change::MakePlayer(coalition, index) => {
                for aircraft in self.map.coalitions.iter_mut().flat_map(|coalition| coalition.aircraft.iter_mut()) {
                    aircraft.player = false;
                }
                self.map.coalitions[coalition].aircraft[index].player = true;
            }
            Change::Rename { coalition, index, name } => {
                let taken = self.map.aircraft().any(|(_, aircraft)| aircraft.name == name);
                if name.is_empty() || name.contains('/') || taken {
                    self.status = Some((format!("Can't rename to '{name}' - names must be unique, not empty, without '/'"), true));
                } else {
                    self.map.coalitions[coalition].aircraft[index].name = name.clone();
                    self.select(Selection::Aircraft(name));
                }
            }
        }
    }

    /// A new bot: off the player's right wing (60 m), flying alongside it -
    /// or by the player's place in the map when the player isn't in the
    /// scene.
    fn new_aircraft(&self) -> AircraftPlacement {
        let name = unique_name("aircraft", |name| self.map.aircraft().any(|(_, aircraft)| aircraft.name == name));
        let plane = self.planes.first().cloned().unwrap_or_else(|| "f16".to_owned());
        let player = self.map.aircraft().map(|(_, aircraft)| aircraft).find(|aircraft| aircraft.player);
        let live = player.and_then(|player| self.live.get(&player.name));

        let (position, rotation, speed) = match (live, player) {
            (Some(live), _) => {
                // +X is the left wing.
                let position = live.position + live.rotation * Vector3::new(-60.0, 0.0, 0.0);
                let speed = live.speed_kt * KT_TO_MS;
                ((position.x, position.y, position.z), AircraftPlacement::rotation_degrees(&live.rotation), (speed > 1.0).then_some(speed))
            }
            (None, Some(player)) => ((player.position.0 - 60.0, player.position.1, player.position.2), player.rotation, player.speed),
            (None, None) => ((0.0, 105.0, 0.0), (0.0, 0.0, 0.0), None),
        };
        AircraftPlacement { name, plane, player: false, position, rotation, speed, throttle: if speed.is_some() { 0.8 } else { 0.0 }, path: Vec::new(), loadout: Default::default(), flares: None, chaff: None, difficulty: Default::default() }
    }
}


/// The aircraft's loadout, editable: a weapon (or Empty) on each of its
/// plane's pylons - only the kinds each takes - weapons on pylons it doesn't
/// have (to remove), its flares and chaff, and loading it all again where it
/// is - see FlightEditor::loadout_window.
fn loadout_editor(ui: &mut egui::Ui, name: &str, aircraft: &mut AircraftPlacement, pylons: &[PylonSpec], weapons: &[(String, WeaponCategory)], plane_countermeasures: Option<(u32, u32)>, live: Option<LiveAircraft>, requests: &mut Vec<EditorRequest>) {
    let (pylons, weapons) = (pylons.to_vec(), weapons.to_vec());
    if pylons.is_empty() {
        property(ui, "", |ui| {
            ui.label(RichText::new(format!("The {} has no pylons - add them to its data.ron", aircraft.plane)).weak());
        });
    }
    // A weapon (or Empty) on each of its plane's pylons - only the
    // kinds the pylon takes.
    for pylon in &pylons {
        property(ui, &pylon.name, |ui| {
            let current = aircraft.loadout.get(&pylon.name).cloned();
            let mut choice = current.clone();
            egui::ComboBox::from_id_salt(("pylon", pylon.name.as_str()))
                .width(ui.available_width())
                .selected_text(current.as_deref().map(weapon_label).unwrap_or_else(|| "Empty".to_owned()))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut choice, None, "Empty");
                    for (id, category) in &weapons {
                        if pylon.accepts(*category) {
                            ui.selectable_value(&mut choice, Some(id.clone()), weapon_label(id));
                        }
                    }
                });
            if choice != current {
                match choice {
                    Some(id) => { aircraft.loadout.insert(pylon.name.clone(), id); }
                    None => { aircraft.loadout.remove(&pylon.name); }
                }
            }
        });
    }
    // Weapons on pylons its plane doesn't have (another plane was
    // picked) - shown so they can go.
    let stale: Vec<String> = aircraft.loadout.keys().filter(|name| !pylons.iter().any(|pylon| pylon.name == **name)).cloned().collect();
    for pylon_name in stale {
        property(ui, &pylon_name, |ui| {
            ui.label(RichText::new("no such pylon on this plane").color(ERROR_COLOR));
            if ui.small_button("×").on_hover_text("Remove it from the loadout").clicked() {
                aircraft.loadout.remove(&pylon_name);
            }
        });
    }
    if !aircraft.loadout.is_empty() {
        property(ui, "", |ui| {
            if ui.add(egui::Button::new("Clear loadout").min_size(egui::vec2(ui.available_width(), 0.0))).clicked() {
                aircraft.loadout.clear();
            }
        });
    }

    // Flares and chaff - as many as its plane carries unless set
    // here (left at that, it's left out of the map file).
    match plane_countermeasures {
        Some((plane_flares, plane_chaff)) => {
            for (label, count, plane_count) in [("Flares", &mut aircraft.flares, plane_flares), ("Chaff", &mut aircraft.chaff, plane_chaff)] {
                property(ui, label, |ui| {
                    let mut value = count.unwrap_or(plane_count);
                    let response = ui.add_sized([ui.available_width(), 20.0], egui::DragValue::new(&mut value).speed(1.0).range(0..=999))
                        .on_hover_text(format!("Its plane carries {plane_count} by its data.ron"));
                    if response.changed() {
                        *count = (value != plane_count).then_some(value);
                    }
                });
            }
        }
        None => {
            property(ui, "", |ui| {
                ui.label(RichText::new(format!("The {} has no flares or chaff - add `countermeasures` to its data.ron", aircraft.plane)).weak());
            });
        }
    }

    property(ui, "", |ui| {
        let reset = ui.add_enabled(live.is_some(), egui::Button::new("Reset loadout").min_size(egui::vec2(ui.available_width(), 0.0)))
            .on_hover_text("Load it again where it is: every weapon back on, flares and chaff full");
        if reset.clicked() {
            requests.push(EditorRequest::ResetLoadout(name.to_owned()));
        }
    });
}

/// A row's icon - drawn as shapes where egui's fonts might lack the glyph.
#[derive(Clone, Copy)]
enum Icon {
    /// A coalition - a dot in its color.
    Coalition,
    Aircraft,
}

/// Draws `icon` centered at `center`.
fn paint_icon(painter: &egui::Painter, style: &egui::Style, icon: Icon, center: egui::Pos2, color: Color32) {
    match icon {
        Icon::Coalition => {
            painter.circle_filled(center, 5.0, color);
        }
        Icon::Aircraft => {
            painter.text(center, egui::Align2::CENTER_CENTER, "✈", egui::TextStyle::Body.resolve(style), color);
        }
    }
}

/// A fold arrow centered at `center` - pointing down when `open`, right
/// when folded.
fn paint_fold_arrow(painter: &egui::Painter, center: egui::Pos2, open: bool, color: Color32) {
    let points = if open {
        vec![center + egui::vec2(-4.0, -2.5), center + egui::vec2(4.0, -2.5), center + egui::vec2(0.0, 3.0)]
    } else {
        vec![center + egui::vec2(-2.5, -4.0), center + egui::vec2(3.0, 0.0), center + egui::vec2(-2.5, 4.0)]
    };
    painter.add(egui::Shape::convex_polygon(points, color, egui::Stroke::NONE));
}

/// One row of the hierarchy - see `tree_row`.
struct TreeRow {
    /// How many levels in - 0 for a coalition, 1 for its aircraft.
    depth: usize,
    /// A foldable row's fold arrow - `Some(open)`.
    fold: Option<bool>,
    icon: Icon,
    icon_color: Color32,
    text: String,
    /// `None`: the normal text color.
    text_color: Option<Color32>,
    /// Dimmed, on the right (the plane, an aircraft count).
    detail: String,
}

/// What a click on a hierarchy row did.
#[derive(PartialEq)]
enum RowClick {
    None,
    /// On the fold arrow.
    Fold,
    /// Anywhere else on the row.
    Select,
}

/// Draws one hierarchy row, full width: its fold arrow, indented by its
/// depth, icon, name, detail on the right; highlighted when selected or
/// hovered.
fn tree_row(ui: &mut egui::Ui, selected: bool, row: TreeRow) -> RowClick {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_HEIGHT), egui::Sense::click());
    let visuals = ui.visuals();
    let background = if selected {
        visuals.selection.bg_fill
    } else if response.hovered() {
        visuals.widgets.hovered.weak_bg_fill
    } else {
        Color32::TRANSPARENT
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 0.0, background);

    let font = egui::TextStyle::Body.resolve(ui.style());
    let small = egui::TextStyle::Small.resolve(ui.style());
    // Room for the fold arrow on the left, then each level indents.
    let arrow_center = egui::pos2(rect.left() + 10.0, rect.center().y);
    let left = rect.left() + 20.0 + row.depth as f32 * INDENT;
    if let Some(open) = row.fold {
        paint_fold_arrow(painter, arrow_center, open, visuals.text_color());
    }
    // A tree line from the parent down to each child.
    if row.depth > 0 {
        let x = left - INDENT * 0.5;
        let stroke = egui::Stroke::new(1.0, visuals.weak_text_color().gamma_multiply(0.5));
        painter.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.center().y)], stroke);
        painter.line_segment([egui::pos2(x, rect.center().y), egui::pos2(left - 2.0, rect.center().y)], stroke);
    }
    paint_icon(painter, ui.style(), row.icon, egui::pos2(left + 6.0, rect.center().y), row.icon_color);
    let text_color = row.text_color.unwrap_or(if selected { visuals.selection.stroke.color } else { visuals.text_color() });
    painter.text(egui::pos2(left + 18.0, rect.center().y), egui::Align2::LEFT_CENTER, row.text, font, text_color);
    painter.text(rect.right_center() - egui::vec2(6.0, 0.0), egui::Align2::RIGHT_CENTER, row.detail, small, visuals.weak_text_color());

    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    if !response.clicked() {
        return RowClick::None;
    }
    let on_arrow = row.fold.is_some() && response.interact_pointer_pos().is_some_and(|pos| pos.x < rect.left() + 20.0);
    if on_arrow { RowClick::Fold } else { RowClick::Select }
}

/// The inspector's top: what's selected - its icon and name, its own
/// actions on the right (`actions`), and its status under them (`status`) -
/// with a line under it.
fn inspector_header(ui: &mut egui::Ui, icon: Icon, icon_color: Color32, name: &str, status: impl FnOnce(&mut egui::Ui), actions: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 10)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(20.0, 22.0), egui::Sense::hover());
            paint_icon(ui.painter(), ui.style(), icon, rect.center(), icon_color);
            ui.label(RichText::new(name).size(17.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), actions);
        });
        status(ui);
    });
    ui.separator();
}

/// An aircraft's state for its inspector header: a dot and flying, wrecked
/// or not in the scene - the details are in its Status section.
fn status_line(ui: &mut egui::Ui, live: Option<LiveAircraft>) {
    let (state, color) = match live {
        None => ("Not in the scene", ERROR_COLOR),
        Some(live) if live.wrecked => ("Wrecked", ERROR_COLOR),
        Some(_) => ("Flying", OK_COLOR),
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(16.0, 14.0), egui::Sense::hover());
        ui.painter().circle_filled(rect.center(), 3.5, color);
        ui.label(RichText::new(state).small().color(color));
    });
}

/// A collapsible inspector section (egui's own collapsing header, open to
/// start with) - a card of its own: a faint rounded box, its title on top.
fn section(ui: &mut egui::Ui, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    let visuals = ui.visuals().clone();
    egui::Frame::new()
        .fill(visuals.faint_bg_color)
        .stroke(egui::Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(10, 6))
        .outer_margin(egui::Margin { left: PAD, right: PAD, top: 3, bottom: 3 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::CollapsingHeader::new(RichText::new(title).strong())
                .id_salt(title)
                .default_open(true)
                .show_unindented(ui, |ui| {
                    ui.add_space(2.0);
                    ui.spacing_mut().item_spacing.y = 4.0;
                    body(ui);
                });
        });
}

/// One inspector row: `label` dimmed in the fixed-width label column, the
/// value's widgets (`value`) filling the rest.
fn property(ui: &mut egui::Ui, label: &str, value: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(LABEL_WIDTH, 20.0), egui::Sense::hover());
        let font = egui::TextStyle::Body.resolve(ui.style());
        ui.painter().text(rect.left_center(), egui::Align2::LEFT_CENTER, label, font, ui.visuals().weak_text_color());
        value(ui);
    });
}

/// Three numbers side by side, each with its axis letter in the axis color
/// (x red, y green, z blue) - for a position or rotation.
fn vector_field(ui: &mut egui::Ui, values: [&mut f32; 3], speed: f64, suffix: &str) {
    vector_field_leaving(ui, values, speed, suffix, 0.0);
}

/// `vector_field`, leaving `leave` points free on its right (for a button).
fn vector_field_leaving(ui: &mut egui::Ui, values: [&mut f32; 3], speed: f64, suffix: &str, leave: f32) {
    vector_field_labelled(ui, values, ["x", "y", "z"], speed, suffix, leave);
}

/// `vector_field_leaving` with its own axis labels (e.g. "alt" for a path
/// point's height).
fn vector_field_labelled(ui: &mut egui::Ui, values: [&mut f32; 3], letters: [&str; 3], speed: f64, suffix: &str, leave: f32) {
    let spacing = ui.spacing().item_spacing.x;
    let letter_widths = letters.map(|letter| if letter.len() > 1 { 22.0 } else { 10.0 });
    let field_width = ((ui.available_width() - leave - letter_widths.iter().sum::<f32>() - 5.0 * spacing) / 3.0).max(30.0);
    for (((value, letter), color), letter_width) in values.into_iter().zip(letters).zip(AXIS_COLORS).zip(letter_widths) {
        ui.add_sized([letter_width, 20.0], egui::Label::new(RichText::new(letter).color(color).strong()));
        ui.add_sized([field_width, 20.0], egui::DragValue::new(value).speed(speed).suffix(suffix).max_decimals(1));
    }
}

/// A weapon's name to show, from its id: "missiles/aim-9x" -> "AIM-9X".
fn weapon_label(id: &str) -> String {
    id.rsplit('/').next().unwrap_or(id).to_uppercase()
}

/// Where "+ Add point" puts the next point of `aircraft`'s path: PATH_STEP
/// on from the last point, carrying on the way the path was going - or, for
/// the first point, PATH_STEP ahead of the aircraft's nose (where it is now,
/// or its place in the map when it isn't in the scene).
pub(crate) fn next_path_point(aircraft: &AircraftPlacement, live: Option<LiveAircraft>) -> (f32, f32, f32) {
    let (position, rotation) = live.map(|live| (live.position, live.rotation)).unwrap_or_else(|| (aircraft.position(), aircraft.rotation()));
    let nose = rotation * Vector3::z();
    let points: Vec<Vector3<f32>> = aircraft.path.iter().map(|&(x, y, z)| Vector3::new(x, y, z)).collect();
    let next = match points.as_slice() {
        [] => position + nose * PATH_STEP,
        [only] => only + (only - position).try_normalize(1e-3).unwrap_or(nose) * PATH_STEP,
        [.., before, last] => last + (last - before).try_normalize(1e-3).unwrap_or(nose) * PATH_STEP,
    };
    (next.x, next.y, next.z)
}

/// `base`, or `base 2`, `base 3`... - the first one `taken` says is free.
fn unique_name(base: &str, taken: impl Fn(&str) -> bool) -> String {
    (1..).map(|n| if n == 1 { base.to_owned() } else { format!("{base} {n}") })
        .find(|name| !taken(name))
        .unwrap()
}
