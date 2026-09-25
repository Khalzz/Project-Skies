//! The D-pad quick menu - a compact block tucked into the HUD's bottom-left
//! corner for setting up the plane in flight. Each menu has up to four slots
//! (one per D-pad direction); a slot runs an action or opens a submenu.
//! Inside a submenu, the direction that opened it becomes Back - press the
//! same button again to return.
//!
//! Split in two:
//! - the menu itself (`QuickMenu`, `MenuId`, `Slot`) - plain data plus
//!   navigation, owned by `Plane`, which applies whatever `QuickAction`
//!   comes back (same code paths as the keyboard keys for those actions);
//! - its HUD (`build_quick_menu_ui` + `QuickMenu::refresh_ui`) - a D-pad
//!   drawn from one arm sprite (pressed/unpressed), rotated into each
//!   direction, arms meeting in the middle like a controller's, with a
//!   legend beside it listing what each direction does.

use glyphon::cosmic_text::Align;

use crate::app::App;
use crate::engine::input::input;
use crate::engine::rendering::ui::ui::Ui;
use crate::engine::ui::color::UiColor;
use crate::engine::ui::ui_node::UiNode;
use crate::engine::ui::ui_transform::{Anchor, Orientation, PositionValue, SizeValue};
use crate::game::ui::label;

/// Top-level UI node id - see `build_quick_menu_ui`.
pub const QUICK_MENU_UI: &str = "quick_menu";

/// Any submenu with no D-pad input for this long closes back to the root.
const IDLE_RETURN_SECONDS: f32 = 5.0;
/// How long a pressed key stays lit.
const FLASH_SECONDS: f32 = 0.15;

// The D-pad arm art - the UP arm (outer edge at the top, point facing the
// centre); the other three directions are the same art rotated (see
// `Direction::quarter_turns`), which keeps the outer-edge highlight on the
// outside for every arm. Paths are relative to assets/.
const ARM_UNPRESSED: &str = "sprites/controller/dpad-up-unpressed.png";
const ARM_PRESSED: &str = "sprites/controller/dpad-up-pressed.png";
/// Every texture the HUD swaps between - preload these (`Ui::preload_images`)
/// when building it, since the pressed one isn't on any node until a press.
pub const ARM_SPRITES: [&str; 2] = [ARM_UNPRESSED, ARM_PRESSED];
// Arm size on screen - the art is 201x299, so this keeps its aspect.
const ARM_WIDTH: f32 = 34.0;
const ARM_LENGTH: f32 = 51.0;
// Space left between opposite arms' points, so the four read as separate
// buttons that still meet in the middle.
const ARM_CENTRE_GAP: f32 = 3.0;
// The pad is a square just big enough for two arm lengths end to end - each
// arm is pinned to one of its edges, so their points meet at its centre.
const PAD_SIZE: f32 = ARM_LENGTH * 2.0 + ARM_CENTRE_GAP;
// Empty slots draw their arm faded to this alpha.
const EMPTY_ARM_ALPHA: f32 = 0.25;
// Distance from the screen's left and bottom edges.
const CORNER_MARGIN: f32 = 12.0;
// The legend column to the pad's right.
const LEGEND_GAP: f32 = 10.0;
const LEGEND_WIDTH: f32 = 200.0;
const LABEL_HEIGHT: f32 = 20.0;
const LABEL_FONT_SIZE: f32 = 14.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Right,
    Down,
    Left,
}

impl Direction {
    const ALL: [Direction; 4] = [Direction::Up, Direction::Right, Direction::Down, Direction::Left];

    fn index(self) -> usize {
        self as usize
    }

    fn name(self) -> &'static str {
        match self {
            Direction::Up => "Up",
            Direction::Right => "Right",
            Direction::Down => "Down",
            Direction::Left => "Left",
        }
    }

    fn action(self) -> &'static str {
        match self {
            Direction::Up => "quick_menu_up",
            Direction::Right => "quick_menu_right",
            Direction::Down => "quick_menu_down",
            Direction::Left => "quick_menu_left",
        }
    }

    /// Marks which arm a legend line belongs to.
    fn arrow(self) -> &'static str {
        match self {
            Direction::Up => "↑",
            Direction::Right => "→",
            Direction::Down => "↓",
            Direction::Left => "←",
        }
    }

    /// How far the UP arm art turns (clockwise) to become this direction's arm.
    fn quarter_turns(self) -> u8 {
        match self {
            Direction::Up => 0,
            Direction::Right => 1,
            Direction::Down => 2,
            Direction::Left => 3,
        }
    }

    /// Where this direction's arm and legend line live under `QUICK_MENU_UI`.
    fn key_path(self) -> String {
        format!("{QUICK_MENU_UI}/Pad/{}", self.name())
    }

    fn label_path(self) -> String {
        format!("{QUICK_MENU_UI}/Legend/{}", self.name())
    }
}

/// What a menu slot can do - applied by `Plane` (see `Plane::apply_quick_action`).
#[derive(Clone, Copy)]
pub enum QuickAction {
    ToggleGear,
    ToggleParkingBrake,
    ToggleFlyByWire,
}

/// The plane's current state, for slot labels like "Landing gear (DOWN)".
pub struct QuickMenuStatus {
    pub gear: GearStatus,
    pub parking_brake: bool,
    pub fly_by_wire: bool,
}

pub enum GearStatus {
    Up,
    Down,
    Moving,
}

impl GearStatus {
    pub fn from_deploy(deploy: f32) -> Self {
        if deploy >= 0.999 {
            GearStatus::Down
        } else if deploy <= 0.001 {
            GearStatus::Up
        } else {
            GearStatus::Moving
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MenuId {
    Root,
    GearAndBrakes,
    FlightControl,
}

#[derive(Clone, Copy)]
enum Slot {
    Empty,
    Submenu(&'static str, MenuId),
    Action(QuickAction),
    /// Never written in `menu()` - a submenu's entry direction turns into
    /// this automatically (see `QuickMenu::slot`).
    Back,
}

impl Slot {
    fn label(&self, status: &QuickMenuStatus) -> String {
        let on_off = |on: bool| if on { "ON" } else { "OFF" };
        match self {
            Slot::Empty => String::new(),
            Slot::Submenu(name, _) => (*name).to_owned(),
            Slot::Back => "Back".to_owned(),
            Slot::Action(QuickAction::ToggleGear) => {
                let gear = match status.gear {
                    GearStatus::Up => "UP",
                    GearStatus::Down => "DOWN",
                    GearStatus::Moving => "MOVING",
                };
                format!("Landing gear ({gear})")
            }
            Slot::Action(QuickAction::ToggleParkingBrake) => {
                format!("Parking brake ({})", if status.parking_brake { "SET" } else { "OFF" })
            }
            Slot::Action(QuickAction::ToggleFlyByWire) => format!("Fly-by-wire ({})", on_off(status.fly_by_wire)),
        }
    }
}

/// A menu's title and its slots, in `Direction` order: Up, Right, Down,
/// Left. In a submenu, leave the slot for the direction that opens it
/// `Empty` - that's where Back goes. To add a menu: a new `MenuId`, its
/// entry here, and a `Slot::Submenu` pointing at it from its parent.
fn menu(id: MenuId) -> (&'static str, [Slot; 4]) {
    match id {
        MenuId::Root => ("Aircraft", [
            Slot::Submenu("Gear & brakes", MenuId::GearAndBrakes),
            Slot::Submenu("Flight control", MenuId::FlightControl),
            Slot::Empty,
            Slot::Empty,
        ]),
        // Opened with Up - Up is Back.
        MenuId::GearAndBrakes => ("Gear & brakes", [
            Slot::Empty,
            Slot::Action(QuickAction::ToggleParkingBrake),
            Slot::Empty,
            Slot::Action(QuickAction::ToggleGear),
        ]),
        // Opened with Right - Right is Back.
        MenuId::FlightControl => ("Flight control", [
            Slot::Action(QuickAction::ToggleFlyByWire),
            Slot::Empty,
            Slot::Empty,
            Slot::Empty,
        ]),
    }
}

/// How one key currently looks - tracked so the UI is only touched when it
/// actually changes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum KeyLook {
    Normal,
    Lit,
    Empty,
}

pub struct QuickMenu {
    /// Open submenus, innermost last, each with the direction that opened
    /// it (its Back). Empty = the root menu.
    open: Vec<(MenuId, Direction)>,
    idle_seconds: f32,
    flash: Option<(Direction, f32)>,
    // What the HUD last showed - see `refresh_ui`.
    shown_visible: Option<bool>,
    shown_title: String,
    shown_labels: [String; 4],
    shown_looks: [Option<KeyLook>; 4],
}

impl QuickMenu {
    pub fn new() -> Self {
        Self {
            open: Vec::new(),
            idle_seconds: 0.0,
            flash: None,
            shown_visible: None,
            shown_title: String::new(),
            shown_labels: Default::default(),
            shown_looks: [None; 4],
        }
    }

    fn current(&self) -> MenuId {
        self.open.last().map(|(menu, _)| *menu).unwrap_or(MenuId::Root)
    }

    /// What `direction` does right now - the current menu's slot, except
    /// that the direction which opened this submenu is always Back.
    fn slot(&self, direction: Direction) -> Slot {
        match self.open.last() {
            Some((_, opened_with)) if *opened_with == direction => Slot::Back,
            _ => menu(self.current()).1[direction.index()],
        }
    }

    /// Reads the D-pad and navigates - returns the action to run, if a
    /// pressed slot has one. Call only while the player has control.
    pub fn update(&mut self, delta_time: f32) -> Option<QuickAction> {
        self.idle_seconds += delta_time;
        if let Some((_, remaining)) = &mut self.flash {
            *remaining -= delta_time;
            if *remaining <= 0.0 {
                self.flash = None;
            }
        }

        let pressed = Direction::ALL.into_iter().find(|direction| input::is_action_just_pressed(direction.action()));
        let Some(direction) = pressed else {
            if !self.open.is_empty() && self.idle_seconds >= IDLE_RETURN_SECONDS {
                self.open.clear();
            }
            return None;
        };

        self.idle_seconds = 0.0;
        match self.slot(direction) {
            Slot::Empty => None,
            Slot::Submenu(_, next) => {
                self.flash = Some((direction, FLASH_SECONDS));
                self.open.push((next, direction));
                None
            }
            Slot::Back => {
                self.flash = Some((direction, FLASH_SECONDS));
                self.open.pop();
                None
            }
            Slot::Action(action) => {
                self.flash = Some((direction, FLASH_SECONDS));
                Some(action)
            }
        }
    }

    /// Brings the HUD in line with the menu - shown only when `visible`.
    /// Touches each node only when what it shows actually changed.
    pub fn refresh_ui(&mut self, app: &mut App, status: &QuickMenuStatus, visible: bool) {
        let mut changed = false;

        if self.shown_visible != Some(visible) {
            if let Some(root) = Ui::get_ui_node(&mut app.ui.renderizable_elements, QUICK_MENU_UI) {
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

        let (title, _) = menu(self.current());
        if self.shown_title != title {
            let path = format!("{QUICK_MENU_UI}/Legend/Title");
            if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &path).and_then(|node| node.as_label_mut()) {
                node.set_text(&mut app.ui.text.font_system, title, true);
                self.shown_title = title.to_owned();
                changed = true;
            }
        }

        for direction in Direction::ALL {
            let slot = self.slot(direction);
            let index = direction.index();

            let text = match slot {
                Slot::Empty => String::new(),
                _ => format!("{}  {}", direction.arrow(), slot.label(status)),
            };
            if self.shown_labels[index] != text {
                if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &direction.label_path()).and_then(|node| node.as_label_mut()) {
                    node.set_text(&mut app.ui.text.font_system, &text, true);
                    self.shown_labels[index] = text;
                    changed = true;
                }
            }

            let look = match (slot, self.flash) {
                (Slot::Empty, _) => KeyLook::Empty,
                (_, Some((flashed, _))) if flashed == direction => KeyLook::Lit,
                _ => KeyLook::Normal,
            };
            if self.shown_looks[index] != Some(look) {
                if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &direction.key_path()) {
                    style_key(node, look);
                    self.shown_looks[index] = Some(look);
                    changed = true;
                }
            }
        }

        if changed {
            app.ui.has_changed = true;
        }
    }
}

fn style_key(node: &mut UiNode, look: KeyLook) {
    let (path, alpha) = match look {
        KeyLook::Normal => (ARM_UNPRESSED, 1.0),
        KeyLook::Lit => (ARM_PRESSED, 1.0),
        KeyLook::Empty => (ARM_UNPRESSED, EMPTY_ARM_ALPHA),
    };
    if let Some(image) = node.as_image_mut() {
        if image.path != path {
            image.path = path.to_owned();
        }
    }
    node.set_alpha(alpha);
}

/// One D-pad arm - the UP arm art, turned to face `direction`, pinned to its
/// own edge of the pad (see `PAD_SIZE`) with its long side running outward.
fn key(direction: Direction) -> UiNode {
    let (width, height, x, y) = match direction {
        Direction::Up => (ARM_WIDTH, ARM_LENGTH, PositionValue::Center(0.0), PositionValue::Start(0.0)),
        Direction::Down => (ARM_WIDTH, ARM_LENGTH, PositionValue::Center(0.0), PositionValue::End(0.0)),
        Direction::Left => (ARM_LENGTH, ARM_WIDTH, PositionValue::Start(0.0), PositionValue::Center(0.0)),
        Direction::Right => (ARM_LENGTH, ARM_WIDTH, PositionValue::End(0.0), PositionValue::Center(0.0)),
    };
    UiNode::image(ARM_UNPRESSED.to_owned(), width, height)
        .set_size(SizeValue::Pixels(width), SizeValue::Pixels(height))
        .set_position(x, y)
        .rotate_image(direction.quarter_turns())
}

/// One legend line - left-aligned, fixed height so the column doesn't shift
/// as lines empty and fill.
fn legend_line(app: &mut App) -> UiNode {
    label(app, "")
        .set_size(SizeValue::Pixels(LEGEND_WIDTH), SizeValue::Pixels(LABEL_HEIGHT))
        .set_font_size(&mut app.ui.text.font_system, LABEL_FONT_SIZE)
        .set_text_color(UiColor::Rgb(220, 220, 220))
        .set_align(Align::Left)
}

/// The HUD: the D-pad - four arms meeting in the middle - tucked into the
/// bottom-left corner, with a legend beside it (the menu's title, then one
/// line per direction, each marked with its arrow). No panel behind it.
/// Built inactive and blank - `QuickMenu::refresh_ui` fills it in and shows
/// it. Its arm textures need loading separately - see `ARM_SPRITES`.
pub fn build_quick_menu_ui(app: &mut App) -> UiNode {
    // Only self-positioned arms inside, on a fixed-size square - so the arms
    // can overlap each other's corners (their V-shaped points nest together)
    // instead of being stacked side by side.
    let pad = UiNode::container()
        .set_size(SizeValue::Pixels(PAD_SIZE), SizeValue::Pixels(PAD_SIZE))
        .set_background_color(UiColor::TRANSPARENT)
        .set_child("Up", key(Direction::Up))
        .set_child("Right", key(Direction::Right))
        .set_child("Down", key(Direction::Down))
        .set_child("Left", key(Direction::Left));

    let mut legend = UiNode::container()
        .set_orientation(Orientation::Vertical)
        .set_size(SizeValue::Fit, SizeValue::Fit)
        .set_background_color(UiColor::TRANSPARENT)
        .set_padding([0.0, 0.0, LEGEND_GAP, 0.0])
        .set_child("Title", legend_line(app).set_text_color(UiColor::WHITE));
    for direction in Direction::ALL {
        legend = legend.set_child(direction.name(), legend_line(app));
    }

    UiNode::container()
        .set_orientation(Orientation::Horizontal)
        .set_size(SizeValue::Fit, SizeValue::Fit)
        .set_position(PositionValue::Start(CORNER_MARGIN), PositionValue::End(-CORNER_MARGIN))
        .set_background_color(UiColor::TRANSPARENT)
        .set_child_anchor(Anchor::Start, Anchor::Center)
        .set_child("Pad", pad)
        .set_child("Legend", legend)
        .active(false)
}
