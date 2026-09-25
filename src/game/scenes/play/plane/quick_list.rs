//! The quick menu as cascading lists - a test alternative to the D-pad
//! layout in quick_menu.rs (which this reuses the actions, plane status and
//! D-pad bindings of). The root list sits in the HUD's bottom-left corner;
//! opening a submenu shows it as a new list to the right, with the parent
//! keeping its selection marked, so the path stays visible.
//!
//! D-pad: Up/Down move the selection, Right opens a submenu or runs an
//! action, Left goes back one list.
//!
//! The HUD is built once with a fixed number of columns and rows (see
//! `MAX_COLUMNS`/`MAX_ITEMS`); `refresh_ui` fills in and shows only what the
//! open lists need.

use glyphon::cosmic_text::Align;

use crate::app::App;
use crate::engine::input::input;
use crate::engine::rendering::ui::ui::Ui;
use crate::engine::ui::color::UiColor;
use crate::engine::ui::ui_node::UiNode;
use crate::engine::ui::ui_transform::{Anchor, Orientation, PositionValue, SizeValue};
use crate::game::ui::{label, HUD_FONT};

use super::quick_menu::{GearStatus, QuickAction, QuickMenuStatus};

/// Top-level UI node id - see `build_quick_list_ui`.
pub const QUICK_LIST_UI: &str = "quick_list";

// How many lists can be open side by side, and the longest list - the HUD
// has exactly this many columns/rows built. Raise them when a deeper or
// longer menu needs it.
const MAX_COLUMNS: usize = 3;
const MAX_ITEMS: usize = 6;

const CORNER_MARGIN: f32 = 12.0;
const COLUMN_GAP: f32 = 6.0;
const ITEM_WIDTH: f32 = 240.0;
const ITEM_HEIGHT: f32 = 30.0;
const ITEM_PADDING_X: f32 = 10.0;
const FONT_SIZE: f32 = 17.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum MenuId {
    Root,
    GearAndBrakes,
    FlightControl,
}

#[derive(Clone, Copy)]
enum Entry {
    Submenu(&'static str, MenuId),
    Action(QuickAction),
}

impl Entry {
    fn label(&self, status: &QuickMenuStatus) -> String {
        match self {
            Entry::Submenu(name, _) => format!("{name}  ›"),
            Entry::Action(QuickAction::ToggleGear) => {
                let gear = match status.gear {
                    GearStatus::Up => "UP",
                    GearStatus::Down => "DOWN",
                    GearStatus::Moving => "MOVING",
                };
                format!("Landing gear ({gear})")
            }
            Entry::Action(QuickAction::ToggleParkingBrake) => {
                format!("Parking brake ({})", if status.parking_brake { "SET" } else { "OFF" })
            }
            Entry::Action(QuickAction::ToggleFlyByWire) => {
                format!("Fly-by-wire ({})", if status.fly_by_wire { "ON" } else { "OFF" })
            }
        }
    }
}

/// A menu's title and its entries, top to bottom. To add a menu: a new
/// `MenuId`, its entry here, and an `Entry::Submenu` pointing at it from its
/// parent (keep within `MAX_ITEMS` entries and `MAX_COLUMNS` deep).
fn menu(id: MenuId) -> (&'static str, &'static [Entry]) {
    match id {
        MenuId::Root => ("Aircraft", &[
            Entry::Submenu("Gear & brakes", MenuId::GearAndBrakes),
            Entry::Submenu("Flight control", MenuId::FlightControl),
        ]),
        MenuId::GearAndBrakes => ("Gear & brakes", &[
            Entry::Action(QuickAction::ToggleGear),
            Entry::Action(QuickAction::ToggleParkingBrake),
        ]),
        MenuId::FlightControl => ("Flight control", &[
            Entry::Action(QuickAction::ToggleFlyByWire),
        ]),
    }
}

/// One open list and which of its entries is selected.
#[derive(Clone, Copy)]
struct Column {
    menu: MenuId,
    selected: usize,
}

/// How one row currently looks - tracked so the UI is only touched when it
/// actually changes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RowLook {
    Hidden,
    Normal,
    /// Selected in a parent list - the path to the open submenu.
    OnPath,
    /// Selected in the list the D-pad is currently moving through.
    Selected,
}

pub struct QuickList {
    /// Always at least the root; the last one is where the D-pad acts.
    columns: Vec<Column>,
    // What the HUD last showed - see `refresh_ui`.
    shown_visible: Option<bool>,
    shown_columns: [bool; MAX_COLUMNS],
    shown_titles: [String; MAX_COLUMNS],
    shown_texts: [[String; MAX_ITEMS]; MAX_COLUMNS],
    shown_looks: [[Option<RowLook>; MAX_ITEMS]; MAX_COLUMNS],
}

impl QuickList {
    pub fn new() -> Self {
        Self {
            columns: vec![Column { menu: MenuId::Root, selected: 0 }],
            shown_visible: None,
            shown_columns: [false; MAX_COLUMNS],
            shown_titles: Default::default(),
            shown_texts: Default::default(),
            shown_looks: [[None; MAX_ITEMS]; MAX_COLUMNS],
        }
    }

    /// Reads the D-pad and navigates - returns the action to run, if Right
    /// was pressed on one. Call only while the player has control. Lists
    /// stay open until the player closes them (Left) - nothing times out.
    pub fn update(&mut self) -> Option<QuickAction> {
        let pressed = |action: &str| input::is_action_just_pressed(action);
        let depth = self.columns.len();
        let active = depth - 1; // the root list is never closed
        let entries = menu(self.columns[active].menu).1;
        let selected = self.columns[active].selected;

        if pressed("quick_menu_up") {
            self.columns[active].selected = (selected + entries.len() - 1) % entries.len();
        } else if pressed("quick_menu_down") {
            self.columns[active].selected = (selected + 1) % entries.len();
        } else if pressed("quick_menu_left") {
            if depth > 1 {
                self.columns.pop();
            }
        } else if pressed("quick_menu_right") {
            match entries[selected] {
                Entry::Submenu(_, next) => {
                    if depth < MAX_COLUMNS {
                        self.columns.push(Column { menu: next, selected: 0 });
                    }
                }
                Entry::Action(action) => return Some(action),
            }
        }
        None
    }

    /// Brings the HUD in line with the open lists - shown only when
    /// `visible`. Touches each node only when what it shows actually changed.
    pub fn refresh_ui(&mut self, app: &mut App, status: &QuickMenuStatus, visible: bool) {
        let mut changed = false;

        if self.shown_visible != Some(visible) {
            if let Some(root) = Ui::get_ui_node(&mut app.ui.renderizable_elements, QUICK_LIST_UI) {
                root.set_active(visible);
                self.shown_visible = Some(visible);
                changed = true;
            }
        }
        if !visible {
            if changed {
                app.ui.has_changed = true;
            }
            return;
        }

        let active_column = self.columns.len() - 1;
        for c in 0..MAX_COLUMNS {
            let column = self.columns.get(c).copied();

            let open = column.is_some();
            if self.shown_columns[c] != open {
                if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &column_path(c)) {
                    node.set_active(open);
                    self.shown_columns[c] = open;
                    changed = true;
                }
            }
            let Some(column) = column else { continue };

            let (title, entries) = menu(column.menu);
            if self.shown_titles[c] != title {
                if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &format!("{}/Title", column_path(c))).and_then(|node| node.as_label_mut()) {
                    node.set_text(&mut app.ui.text.font_system, title, true);
                    self.shown_titles[c] = title.to_owned();
                    changed = true;
                }
            }

            for i in 0..MAX_ITEMS {
                let entry = entries.get(i);
                let look = match entry {
                    None => RowLook::Hidden,
                    Some(_) if i != column.selected => RowLook::Normal,
                    Some(_) if c == active_column => RowLook::Selected,
                    Some(_) => RowLook::OnPath,
                };
                let text = entry.map(|entry| entry.label(status)).unwrap_or_default();

                if self.shown_texts[c][i] != text || self.shown_looks[c][i] != Some(look) {
                    if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &row_path(c, i)) {
                        style_row(node, look);
                        if let Some(label) = node.as_label_mut() {
                            label.set_text(&mut app.ui.text.font_system, &text, true);
                        }
                        self.shown_texts[c][i] = text;
                        self.shown_looks[c][i] = Some(look);
                        changed = true;
                    }
                }
            }
        }

        if changed {
            app.ui.has_changed = true;
        }
    }
}

fn column_path(column: usize) -> String {
    format!("{QUICK_LIST_UI}/Col{column}")
}

fn row_path(column: usize, row: usize) -> String {
    format!("{}/Item{row}", column_path(column))
}

fn style_row(node: &mut UiNode, look: RowLook) {
    node.set_active(look != RowLook::Hidden);
    let (background, text) = match look {
        RowLook::Hidden | RowLook::Normal => (UiColor::TRANSPARENT, UiColor::Rgb(240, 240, 240)),
        RowLook::OnPath => (UiColor::Rgba(255, 255, 255, 45), UiColor::WHITE),
        // Same "lit keycap" inversion the settings chips use on hover.
        RowLook::Selected => (UiColor::Rgba(255, 255, 255, 225), UiColor::Rgb(20, 20, 20)),
    };
    node.update_style(|s| s.set_background_color(background).set_text_color(text));
}

/// A fixed-size, left-aligned text row. Its horizontal padding is added on
/// top of the requested width - a node's padding comes out of whatever size
/// `set_size` gives it.
fn row_label(app: &mut App, text: &str, font_size: f32) -> UiNode {
    label(app, text)
        .set_size(SizeValue::Pixels(ITEM_WIDTH + ITEM_PADDING_X * 2.0), SizeValue::Pixels(ITEM_HEIGHT))
        .set_padding([0.0, 0.0, ITEM_PADDING_X, ITEM_PADDING_X])
        .set_font(&mut app.ui.text.font_system, HUD_FONT)
        .set_font_size(&mut app.ui.text.font_system, font_size)
        .set_corner_radius(4.0)
        .set_align(Align::Left)
}

fn build_column(app: &mut App, index: usize) -> UiNode {
    let mut column = UiNode::container()
        .set_orientation(Orientation::Vertical)
        .set_size(SizeValue::Fit, SizeValue::Fit)
        .set_padding(8.0)
        .set_gap(3.0)
        .set_corner_radius(10.0)
        .set_background_color(UiColor::Rgba(0, 0, 0, 170))
        .set_child("Title", row_label(app, "", FONT_SIZE).set_text_color(UiColor::Rgb(170, 170, 170)));
    for row in 0..MAX_ITEMS {
        column = column.set_child(format!("Item{row}"), row_label(app, "", FONT_SIZE).active(false));
    }
    column.active(index == 0)
}

/// The HUD: one list per open menu, side by side from the bottom-left corner
/// and bottom-aligned (so the root doesn't move as taller submenus open).
/// Built inactive and blank -
/// `QuickList::refresh_ui` fills it in and shows it.
pub fn build_quick_list_ui(app: &mut App) -> UiNode {
    let mut root = UiNode::container()
        .set_orientation(Orientation::Horizontal)
        .set_size(SizeValue::Fit, SizeValue::Fit)
        .set_position(PositionValue::Start(CORNER_MARGIN), PositionValue::End(-CORNER_MARGIN))
        .set_background_color(UiColor::TRANSPARENT)
        .set_child_anchor(Anchor::Start, Anchor::End)
        .set_gap(COLUMN_GAP);
    for index in 0..MAX_COLUMNS {
        root = root.set_child(format!("Col{index}"), build_column(app, index));
    }
    root.active(false)
}
