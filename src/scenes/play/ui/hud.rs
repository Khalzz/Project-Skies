//! The flight HUD - one element, the "hud" node: a full-screen layer
//! holding everything drawn over the flight as text and boxes:
//! - "game_ui": the data box (top left), the compass, the stores (bottom
//!   right) - shown by the level's tracks once the intro's done;
//! - the readouts (`HUD_READOUTS`), the pitch ladder's and heading tape's
//!   numbers, the velocity marker - placed every frame by Play::update_hud
//!   (see `ui_layers::place`);
//! - the F8 heat view's labels.
//!
//! Its parts are reached by `in_hud`: "hud/game_ui/data_box/timer".

use glyphon::cosmic_text::Align;
use glyphon::Weight;
use pankarta_software::app::App;
use pankarta_software::prelude::Scene;
use pankarta_software::ui::color::UiColor;
use pankarta_software::ui::components::label::Font;
use pankarta_software::ui::ui_node::UiNode;
use pankarta_software::ui::ui_transform::{Orientation, PositionValue, SizeValue};

use crate::ext::ui_layers::{self, GameUi};
use crate::ui::{label, HUD_FONT};

/// The HUD's node (`ui_layers`' key).
pub const HUD: &str = "hud";

/// The path of the HUD's part `key` ("game_ui/data_box", "hud_speed"...).
pub fn in_hud(key: &str) -> String {
    format!("{HUD}/{key}")
}

/// Puts the HUD up - built hidden until the level shows "game_ui".
pub(super) fn spawn(scene: &mut Scene, app: &mut App) {
    let hud = build(app);
    ui_layers::add(scene, app, HUD, hud);
}

/// The HUD's tree - every part at the screen's top left, hidden, until
/// it's placed and shown. Later children draw over earlier ones.
fn build(app: &mut App) -> UiNode {
    let at_origin = |node: UiNode| node.set_position(PositionValue::Start(0.0), PositionValue::Start(0.0));
    let mut root = ui_layers::screen_root().set_child("game_ui", build_game_ui(app));
    for (key, text, font_size, width) in HUD_READOUTS {
        let mut readout = build_hud_readout(app, text, font_size, width);
        match key {
            "hud_radar_altitude" => readout = readout.set_align(Align::Left),
            "hud_corner_fuel" => readout = readout.set_align(Align::Right),
            "hud_altitude_warning" => readout = readout.set_font(HUD_WARN_FACE),
            _ => {}
        }
        root = root.set_child(key, at_origin(readout));
    }
    let pitch_keys = (0..HUD_PITCH_LABELS).map(hud_pitch_label_key);
    let heading_keys = (0..HUD_HEADING_LABELS).map(hud_heading_label_key);
    for key in pitch_keys.chain(heading_keys) {
        root = root.set_child(key, at_origin(build_hud_small_label(app)));
    }
    root = root.set_child("velocity_marker", at_origin(build_velocity_marker(app)));
    // Over the rest of the HUD.
    for index in 0..HEAT_LABELS {
        root = root.set_child(heat_label_key(index), at_origin(build_heat_label(app)));
    }
    root
}

// Small green HUD readout - the shape data_box's children and the compass
// share.
fn hud_label(_app: &mut App, text: &str, width: f32, height: f32) -> UiNode {
    label(text)
        .set_size(SizeValue::Pixels(width), SizeValue::Pixels(height))
        .set_font(HUD_FONT)
        .set_font_size(10.0)
        .set_text_color(UiColor::Rgb(0, 255, 0))
        .set_border_color(UiColor::Rgb(0, 255, 0))
        .set_align(Align::Center)
}

/// The HUD's readouts - placed every frame inside the HUD's frame (see
/// Play::update_hud): (key, text, font size, width).
pub const HUD_READOUTS: [(&str, &str, f32, f32); 9] = [
    ("hud_speed", "0", 12.0, 64.0),
    ("hud_altitude", "0", 12.0, 64.0),
    // The heading, in the middle of the heading tape.
    ("hud_heading", "0", 16.0, 64.0),
    // The G, bottom left - in the Fox-2 mode only.
    ("hud_g", "1.0G", 12.0, 64.0),
    // "WARN" (or "BINGO") inside the square.
    ("hud_altitude_warning", "WARN", HUD_WARN_FONT, HUD_WARN_SIZE.0),
    // The selected weapon, on the square's bottom edge, in attack.
    ("hud_weapon", "", 13.0, 160.0),
    // Left of the fuel bar: percent of the tanks left.
    ("hud_corner_fuel", "0%", 14.0, HUD_FUEL_PERCENT_WIDTH),
    // Just outside the right line's bottom corner.
    ("hud_radar_altitude", "R: 0", 12.0, HUD_RADAR_ALTITUDE_WIDTH),
    // The countermeasure setting, shown only just after it changes.
    ("hud_cms_cue", "", 16.0, 160.0),
];

/// The fuel percent's and the radar altitude's boxes (px, before the HUD's
/// scale) - the scene places them against the side lines by these.
pub const HUD_FUEL_PERCENT_WIDTH: f32 = 44.0;
pub const HUD_RADAR_ALTITUDE_WIDTH: f32 = 64.0;
/// A HUD readout's box (px).
pub const HUD_READOUT_SIZE: (f32, f32) = (64.0, 20.0);
/// The low-altitude WARN cue: font size, and the box the text is clipped to
/// (px). Taller than a readout so the larger glyphs aren't cut off.
pub const HUD_WARN_FONT: f32 = 32.0;
pub const HUD_WARN_SIZE: (f32, f32) = (160.0, 48.0);
/// Inter Thin, the lighter face loaded with the HUD font. Regular is the
/// only other weight bundled, so this is the thinner step available.
pub const HUD_WARN_FACE: Font = Font { family: Some("Inter"), weight: Weight::THIN };

/// One of `HUD_READOUTS` - built hidden. The HUD's text is the regular
/// weight, a touch heavier than the thin default.
fn build_hud_readout(_app: &mut App, text: &str, font_size: f32, width: f32) -> UiNode {
    label(text)
        .set_size(SizeValue::Pixels(width), SizeValue::Pixels(HUD_READOUT_SIZE.1))
        .set_font(HUD_FONT)
        .set_font_size(font_size)
        .set_text_color(UiColor::Rgb(0, 255, 77))
        .set_background_color(UiColor::TRANSPARENT)
        .set_align(Align::Center)
        .active(false)
}

/// How many pitch ladder numbers there can be on screen at once - placed
/// (and given its number) every frame by Play::update_hud. Enough for every
/// one: two per line, a line every 5° from -90° to 90° but the horizon.
pub const HUD_PITCH_LABELS: usize = 2 * 36;
/// The same for the heading tape's numbers - it shows 60°, a number every
/// 10°.
pub const HUD_HEADING_LABELS: usize = 8;
/// A pitch ladder or heading tape number's box (px).
pub const HUD_SMALL_LABEL_SIZE: (f32, f32) = (28.0, 14.0);

pub fn hud_pitch_label_key(index: usize) -> String {
    format!("hud_pitch_{index}")
}

pub fn hud_heading_label_key(index: usize) -> String {
    format!("hud_heading_{index}")
}

/// How many heat sources the F8 heat view can label at once (see
/// Play::update_heat_view), and a label's box (px).
pub const HEAT_LABELS: usize = 16;
pub const HEAT_LABEL_SIZE: (f32, f32) = (110.0, 14.0);

pub fn heat_label_key(index: usize) -> String {
    format!("heat_view_{index}")
}

/// One of the `HEAT_LABELS` - built hidden, white so it reads over the
/// heat colors.
fn build_heat_label(_app: &mut App) -> UiNode {
    label("0")
        .set_size(SizeValue::Pixels(HEAT_LABEL_SIZE.0), SizeValue::Pixels(HEAT_LABEL_SIZE.1))
        .set_font(HUD_FONT)
        .set_font_size(10.0)
        .set_text_color(UiColor::Rgb(255, 255, 255))
        .set_background_color(UiColor::TRANSPARENT)
        .set_align(Align::Center)
        .active(false)
}

/// One of the `HUD_PITCH_LABELS` or `HUD_HEADING_LABELS` - built hidden.
fn build_hud_small_label(_app: &mut App) -> UiNode {
    label("0")
        .set_size(SizeValue::Pixels(HUD_SMALL_LABEL_SIZE.0), SizeValue::Pixels(HUD_SMALL_LABEL_SIZE.1))
        .set_font(HUD_FONT)
        .set_font_size(10.0)
        .set_text_color(UiColor::Rgb(0, 255, 77))
        .set_background_color(UiColor::TRANSPARENT)
        .set_align(Align::Center)
        .active(false)
}

/// The data box, the compass and the stores. Built hidden - the level's
/// tracks show it.
fn build_game_ui(app: &mut App) -> UiNode {
    let data_box = UiNode::container()
        .set_size(SizeValue::Fit, SizeValue::Fit)
        .set_position(PositionValue::Start(10.0), PositionValue::Start(10.0))
        .set_padding(20.0)
        .set_gap(5.0)
        .set_background_color(UiColor::Rgba(0, 0, 0, 128))
        .set_child("timer", hud_label(app, "00:00:00:00", 250.0, 35.0))
        .set_child("framerate", hud_label(app, "100fps", 250.0, 35.0))
        .set_child("g", hud_label(app, "1g", 250.0, 35.0))
        .set_child("power", hud_label(app, "0%", 250.0, 35.0))
        .set_child("fuel", hud_label(app, "Fuel: 0 kg", 250.0, 35.0))
        // The selected weapon and its lock - see Play::update_weapons.
        .set_child("weapons", hud_label(app, "WPN OFF", 250.0, 35.0))
        // Flares and chaff left, and what's released - see
        // Play::update_weapons.
        .set_child("countermeasures", hud_label(app, "", 250.0, 35.0));

    let compass = hud_label(app, "90°", 100.0, 35.0)
        .set_position(PositionValue::Center(0.0), PositionValue::Start(10.0));
    ui_layers::screen_root()
        .set_child("data_box", data_box)
        .set_child("compass", compass)
        .set_child("hud_stores", build_hud_stores(app))
        .active(false)
}

/// Bottom-right stores, top to bottom. A box is shown only while the jet
/// has something for it (see Play::update_hud). Another page (a radar,
/// later) is another child of `hud_stores` plus a line in that update.
const STORE_ROW_WIDTH: f32 = 168.0;
const STORE_ROW_HEIGHT: f32 = 18.0;
/// How many weapon rows the weapons box has room for.
pub const WEAPON_LIST_ROWS: usize = 8;

fn store_label(_app: &mut App, text: &str, font_size: f32) -> UiNode {
    label(text)
        .set_size(SizeValue::Pixels(STORE_ROW_WIDTH), SizeValue::Pixels(STORE_ROW_HEIGHT))
        .set_font(HUD_FONT)
        .set_font_size(font_size)
        .set_text_color(UiColor::Rgb(0, 255, 77))
        .set_background_color(UiColor::TRANSPARENT)
        .set_align(Align::Left)
}

fn store_box(app: &mut App, title: &str, rows: &[(&str, &str)]) -> UiNode {
    let mut box_node = UiNode::container()
        .set_orientation(Orientation::Vertical)
        .set_size(SizeValue::Fit, SizeValue::Fit)
        .set_padding(8.0)
        .set_gap(2.0)
        .set_background_color(UiColor::Rgba(0, 0, 0, 140))
        .set_child("title", store_label(app, title, 11.0));
    for (id, text) in rows {
        box_node = box_node.set_child(*id, store_label(app, text, 13.0));
    }
    box_node
}

/// The bottom-right column: the weapon list. Text is left-aligned. Fuel and
/// radar altitude sit on the HUD square instead, and the countermeasure
/// setting is a cue that fades. Built hidden.
fn build_hud_stores(app: &mut App) -> UiNode {
    let mut weapons = store_box(app, "WPN", &[]);
    for row in 0..WEAPON_LIST_ROWS {
        weapons = weapons.set_child(format!("row{row}"), store_label(app, "", 13.0).active(false));
    }
    UiNode::container()
        .set_orientation(Orientation::Vertical)
        .set_size(SizeValue::Fit, SizeValue::Fit)
        .set_position(PositionValue::End(-16.0), PositionValue::End(-16.0))
        .set_gap(8.0)
        .set_background_color(UiColor::TRANSPARENT)
        .set_child("weapons", weapons)
        .active(false)
}

/// Fills the weapons box from `entries` - each one's name and whether it's
/// the one selected. A `>` marks only that row. Rows past the last entry
/// are hidden.
pub fn update_weapon_list(ui: &mut impl GameUi, _app: &mut App, entries: &[(String, bool)]) {
    for row in 0..WEAPON_LIST_ROWS {
        let Some(node) = ui.ui_node(&in_hud(&format!("game_ui/hud_stores/weapons/row{row}"))) else { continue };
        match entries.get(row) {
            Some((text, selected)) => {
                node.set_active(true);
                let color = if *selected { UiColor::Rgb(0, 255, 77) } else { UiColor::Rgb(0, 140, 50) };
                node.update_style(|style| style.set_text_color(color));
                if let Some(label) = node.as_label_mut() {
                    let text = if *selected { format!("> {text}") } else { text.clone() };
                    label.set_text(&text);
                }
            }
            None => node.set_active(false),
        }
    }
}

/// The flight path marker's "o" - shown with "game_ui" by the level's
/// tracks; on the HUD's plate it's drawn as HUD text instead (see
/// Play::update_hud).
fn build_velocity_marker(_app: &mut App) -> UiNode {
    label("o")
        .set_size(SizeValue::Pixels(20.0), SizeValue::Pixels(20.0))
        .set_font_size(14.0)
        .set_text_color(UiColor::Rgb(0, 255, 77))
        .set_align(Align::Center)
        .active(false)
}
