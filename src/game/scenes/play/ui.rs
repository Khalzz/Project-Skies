//! Code-built play-scene HUD, following the same pattern `main_menu::ui` uses -
//! replaces what used to be `assets/ui/game_ui.ron`. Building in code (rather
//! than RON) is what lets a HUD element keep its own absolute screen position
//! via `.set_position(...)` (which opts a child out of its parent's flex-
//! stacking layout - see `UiNode::node_content_preparation`'s `pending_position`
//! handling) while still living under one real "game_ui" node - RON-declared
//! children have no equivalent opt-out, so before this, each element had to be
//! its own independent top-level sibling just to keep its placement, and
//! `level_planning.ron` had to toggle all of them individually.

use glyphon::cosmic_text::Align;
use glyphon::Weight;

use crate::app::App;
use crate::engine::ui::components::label::Font;
use crate::engine::rendering::ui::ui::Ui;
use crate::engine::ui::color::UiColor;
use crate::engine::ui::layer::Layer;
use crate::engine::ui::ui_node::UiNode;
use crate::engine::ui::ui_transform::{Orientation, PositionValue, SizeValue};
use crate::game::scenes::main_menu::ui as main_menu_ui;
use crate::game::ui::{button, label, with_left_accent, HUD_FONT};

// Small green HUD readout - the shape data_box's own children and compass/
// speed all share (matches what assets/ui/game_ui.ron used to declare for
// each - font_size/color/border_color/alignment; border_width/corner_radius
// were never set there either, so they're left at Style::default() here too,
// same as they always rendered).
fn hud_label(app: &mut App, text: &str, width: f32, height: f32) -> UiNode {
    label(app, text)
        .set_size(SizeValue::Pixels(width), SizeValue::Pixels(height))
        .set_font(&mut app.ui.text.font_system, HUD_FONT)
        .set_font_size(&mut app.ui.text.font_system, 10.0)
        .set_text_color(UiColor::Rgb(0, 255, 0))
        .set_border_color(UiColor::Rgb(0, 255, 0))
        .set_align(Align::Center)
}

/// The in-flight HUD's static-position elements - one real node ("game_ui")
/// wrapping data_box/compass/speed, built inactive (`GameLogic::finish`
/// reactivates it once the mission-intro card is done, via the single
/// `Active(target: "game_ui", ...)` track in `level_planning.ron` - compare to
/// the 3 separate `Active` entries this used to need).
///
/// "velocity_marker" is deliberately NOT included here despite being part of
/// the same HUD reveal - see `build_velocity_marker`'s own doc comment for
/// why it has to stay independent instead.
/// The HUD's readouts - each its own top-level node (like the velocity
/// marker), placed every frame inside the HUD's frame (see
/// GameLogic::update_hud): (key, text, font size, width).
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
pub fn build_hud_readout(app: &mut App, text: &str, font_size: f32, width: f32) -> UiNode {
    label(app, text)
        .set_size(SizeValue::Pixels(width), SizeValue::Pixels(HUD_READOUT_SIZE.1))
        .set_font(&mut app.ui.text.font_system, HUD_FONT)
        .set_font_size(&mut app.ui.text.font_system, font_size)
        .set_text_color(UiColor::Rgb(0, 255, 77))
        .set_background_color(UiColor::TRANSPARENT)
        .set_align(Align::Center)
        .active(false)
}

/// How many pitch ladder numbers there can be on screen at once - each its
/// own top-level node like the readouts, placed (and given its number) every
/// frame by GameLogic::update_hud. Enough for every one: two per line, a
/// line every 5° from -90° to 90° but the horizon.
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

/// The kill feed (see GameLogic::update_kill_feed): how many rows at most,
/// each one's height, its text's size, the room either side of the text
/// inside its box and between its pieces (px), and how many pieces of text
/// a row has - who, what they did, which plane, the rest - each its own
/// label, in its own color.
pub const KILL_FEED_ROWS: usize = 8;
pub const KILL_FEED_ROW_HEIGHT: f32 = 26.0;
pub const KILL_FEED_FONT_SIZE: f32 = 13.0;
pub const KILL_FEED_PADDING: f32 = 10.0;
pub const KILL_FEED_GAP: f32 = 5.0;
pub const KILL_FEED_PIECES: usize = 4;
/// The widest a piece of text can be (px) - its label's own width.
const KILL_FEED_PIECE_MAX_WIDTH: f32 = 320.0;

/// A row's box.
pub fn kill_feed_key(row: usize) -> String {
    format!("kill_feed_{row}")
}

/// A row's piece of text `piece`.
pub fn kill_feed_piece_key(row: usize, piece: usize) -> String {
    format!("kill_feed_{row}_{piece}")
}

/// A kill feed row's box - dark, see-through, rounded, sized to its text
/// every frame. Built hidden.
pub fn build_kill_feed_box(app: &mut App) -> UiNode {
    label(app, "")
        .set_size(SizeValue::Pixels(KILL_FEED_PIECE_MAX_WIDTH), SizeValue::Pixels(KILL_FEED_ROW_HEIGHT))
        .set_background_color(UiColor::Rgba(0, 0, 0, 150))
        .set_corner_radius(4.0)
        .active(false)
}

/// A piece of a kill feed row's text - on no box of its own (the row's is
/// under it), its color set with its text. Built hidden.
pub fn build_kill_feed_piece(app: &mut App) -> UiNode {
    label(app, "")
        .set_size(SizeValue::Pixels(KILL_FEED_PIECE_MAX_WIDTH), SizeValue::Pixels(KILL_FEED_ROW_HEIGHT))
        .set_font(&mut app.ui.text.font_system, HUD_FONT)
        .set_font_size(&mut app.ui.text.font_system, KILL_FEED_FONT_SIZE)
        .set_text_color(UiColor::Rgb(230, 230, 230))
        .set_background_color(UiColor::TRANSPARENT)
        .set_align(Align::Left)
        .active(false)
}

/// How many heat sources the F8 heat view can label at once (see
/// GameLogic::update_heat_view), and a label's box (px).
pub const HEAT_LABELS: usize = 16;
pub const HEAT_LABEL_SIZE: (f32, f32) = (110.0, 14.0);

pub fn heat_label_key(index: usize) -> String {
    format!("heat_view_{index}")
}

/// One of the `HEAT_LABELS` - built hidden, white so it reads over the
/// heat colors.
pub fn build_heat_label(app: &mut App) -> UiNode {
    label(app, "0")
        .set_size(SizeValue::Pixels(HEAT_LABEL_SIZE.0), SizeValue::Pixels(HEAT_LABEL_SIZE.1))
        .set_font(&mut app.ui.text.font_system, HUD_FONT)
        .set_font_size(&mut app.ui.text.font_system, 10.0)
        .set_text_color(UiColor::Rgb(255, 255, 255))
        .set_background_color(UiColor::TRANSPARENT)
        .set_align(Align::Center)
        .active(false)
}

/// One of the `HUD_PITCH_LABELS` or `HUD_HEADING_LABELS` - built hidden.
pub fn build_hud_small_label(app: &mut App) -> UiNode {
    label(app, "0")
        .set_size(SizeValue::Pixels(HUD_SMALL_LABEL_SIZE.0), SizeValue::Pixels(HUD_SMALL_LABEL_SIZE.1))
        .set_font(&mut app.ui.text.font_system, HUD_FONT)
        .set_font_size(&mut app.ui.text.font_system, 10.0)
        .set_text_color(UiColor::Rgb(0, 255, 77))
        .set_background_color(UiColor::TRANSPARENT)
        .set_align(Align::Center)
        .active(false)
}

pub fn build_game_ui(app: &mut App) -> UiNode {
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
        // The selected weapon and its lock - see GameLogic::update_weapons.
        .set_child("weapons", hud_label(app, "WPN OFF", 250.0, 35.0))
        // Flares and chaff left, and what's released - see
        // GameLogic::update_weapons.
        .set_child("countermeasures", hud_label(app, "", 250.0, 35.0));

    let compass = hud_label(app, "90°", 100.0, 35.0)
        .set_position(PositionValue::Center(0.0), PositionValue::Start(10.0));
    UiNode::container()
        .set_size(SizeValue::Percent(100.0), SizeValue::Percent(100.0))
        .set_position(PositionValue::Start(0.0), PositionValue::Start(0.0))
        .set_background_color(UiColor::TRANSPARENT)
        .set_child("data_box", data_box)
        .set_child("compass", compass)
        .set_child("hud_stores", build_hud_stores(app))
        .active(false)
}

/// Bottom-right stores, top to bottom. A box is shown only while the jet
/// has something for it (see GameLogic::update_hud). Another page (a radar,
/// later) is another child of `hud_stores` plus a line in that update.
const STORE_ROW_WIDTH: f32 = 168.0;
const STORE_ROW_HEIGHT: f32 = 18.0;
/// How many weapon rows the weapons box has room for.
pub const WEAPON_LIST_ROWS: usize = 8;

fn store_label(app: &mut App, text: &str, font_size: f32) -> UiNode {
    label(app, text)
        .set_size(SizeValue::Pixels(STORE_ROW_WIDTH), SizeValue::Pixels(STORE_ROW_HEIGHT))
        .set_font(&mut app.ui.text.font_system, HUD_FONT)
        .set_font_size(&mut app.ui.text.font_system, font_size)
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
pub fn update_weapon_list(app: &mut App, entries: &[(String, bool)]) {
    for row in 0..WEAPON_LIST_ROWS {
        let key = format!("game_ui/hud_stores/weapons/row{row}");
        let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &key) else { continue };
        match entries.get(row) {
            Some((text, selected)) => {
                node.set_active(true);
                let color = if *selected { UiColor::Rgb(0, 255, 77) } else { UiColor::Rgb(0, 140, 50) };
                node.update_style(|style| style.set_text_color(color));
                if let Some(label) = node.as_label_mut() {
                    let text = if *selected { format!("> {text}") } else { text.clone() };
                    label.set_text(&mut app.ui.text.font_system, &text, true);
                }
            }
            None => node.set_active(false),
        }
    }
}

/// Stays a fully independent top-level node rather than nested inside
/// `build_game_ui` - `scene.rs`'s own `ui_control` repositions this every
/// frame by writing straight to its `transform.x`/`.y` (tracking the plane's
/// projected on-screen position), but a container's layout pass
/// unconditionally re-resolves every child's position each frame it runs
/// (whether that child opted out of flow via `pending_position` or is a
/// normal flow child) - nesting it would fight that every single frame,
/// snapping it back to a fixed position and undoing the tracking. Only a
/// node with no parent at all is never touched by anything but whoever
/// explicitly repositions it.
pub fn build_velocity_marker(app: &mut App) -> UiNode {
    label(app, "o")
        .set_size(SizeValue::Pixels(20.0), SizeValue::Pixels(20.0))
        .set_font_size(&mut app.ui.text.font_system, 14.0)
        .set_text_color(UiColor::Rgb(0, 255, 77))
        .set_align(Align::Center)
        .active(false)
}

/// Dialogue captions - independent of the HUD reveal entirely (see
/// `GameLogic::finish`'s own comment on why "subtitles" was always excluded
/// from the old `HUD_KEYS`), so it's built already active; its own alpha is
/// fully owned by `Subtitle::update` (rests at 0, only shown when a line's
/// actually queued).
pub fn build_subtitles(app: &mut App) -> UiNode {
    label(app, "")
        .set_size(SizeValue::Pixels(800.0), SizeValue::Pixels(35.0))
        .set_position(PositionValue::Center(0.0), PositionValue::End(-150.0))
        .set_text_color(UiColor::Rgba(255, 255, 255, 0))
        .set_background_color(UiColor::TRANSPARENT)
        .set_align(Align::Center)
}

/// "Hold ESC to skip" - shown/hidden every frame by `GameLogic::update`
/// (see its own `EventSystem::input_lock_end`-driven toggle) for as long as
/// player input is locked out by a scripted sequence. Built inactive; no
/// ui_track drives this one, unlike the rest of the HUD - it's tied directly
/// to the input lock itself; a level shouldn't need to hand-author a matching
/// Alpha/Active pair every time it adds one.
pub fn build_skip_prompt(app: &mut App) -> UiNode {
    label(app, "Hold ESC to skip")
        .set_size(SizeValue::Pixels(300.0), SizeValue::Pixels(30.0))
        .set_position(PositionValue::Center(0.0), PositionValue::End(-40.0))
        .set_text_color(UiColor::Rgba(255, 255, 255, 180))
        .set_background_color(UiColor::TRANSPARENT)
        .set_align(Align::Center)
        .active(false)
}

// This scene's own panel set for the same show_panel idiom main_menu::ui
// uses - "Settings" is literally main_menu::ui::settings_ui's own tree,
// reused wholesale rather than rebuilt (see that fn's own doc comment on why
// reusing the literal id "Settings" across scenes is safe). Doesn't include
// "PauseBackdrop" - that's driven directly by open_pause_menu/close_pause_menu
// instead, since it should stay visible across both of these panels rather
// than toggle with them.
const PAUSE_PANELS: [&str; 2] = ["Pause", "Settings"];

fn show_pause_panel(app: &mut App, id: &str) {
    for panel_id in PAUSE_PANELS {
        if let Some(panel) = Ui::get_ui_node(&mut app.ui.renderizable_elements, panel_id) {
            panel.set_active(panel_id == id);
        }
    }
}

/// Brings the pause menu up - sets `app.is_paused` (see its own doc comment
/// on `App` for what that actually gates in `GameLogic::update`: physics and
/// `Plane::update`) and shows the root "Pause" panel. Currently only called
/// from the "toggle_pause_menu" key (see `GameLogic::update`), but any UI
/// button could call this too.
pub fn open_pause_menu(app: &mut App) {
    app.is_paused = true;
    // Frees the cursor so the pause buttons are actually clickable -
    // relative mode (see play::scene::GameLogic::finish, which turns it
    // back on for flight) captures/hides it for mouse-look otherwise.
    app.window_manager.context.mouse().set_relative_mouse_mode(false);
    if let Some(backdrop) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "PauseBackdrop") {
        backdrop.set_active(true);
    }
    show_pause_panel(app, "Pause");

    // Hide the flight HUD while paused - PauseBackdrop's gradient fades to
    // fully transparent well before the screen's right edge, exactly where
    // compass/speed/data_box sit, so without this they'd keep showing right
    // through it. Remembers whichever of these were actually active (see
    // App::paused_hud_visibility's own doc comment) so close_pause_menu can
    // restore exactly that instead of just forcing both back on.
    let game_ui_active = Ui::get_ui_node(&mut app.ui.renderizable_elements, "game_ui").is_some_and(|n| n.is_active);
    let velocity_marker_active = Ui::get_ui_node(&mut app.ui.renderizable_elements, "velocity_marker").is_some_and(|n| n.is_active);
    app.paused_hud_visibility = Some((game_ui_active, velocity_marker_active));
    if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "game_ui") {
        node.set_active(false);
    }
    if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "velocity_marker") {
        node.set_active(false);
    }
}

/// Closes the pause menu entirely, whichever of its own panels was showing -
/// shared by the "Resume" button, "toggle_pause_menu" pressed again while
/// already paused, and Restart/Back-to-menu (both about to tear down this
/// scene's whole UI anyway via `SceneManager::open_scene`, but closing first
/// keeps `app.is_paused` correctly cleared before that reset runs - otherwise
/// the next scene would start already paused with nothing on screen to
/// explain why, see `App::is_paused`'s own doc comment).
pub fn close_pause_menu(app: &mut App) {
    app.is_paused = false;
    // Restores mouse-look - unless F8 had the mouse freed for the debug
    // overlay (App::debug_mouse_free). Harmless even for Restart/Back-to-
    // menu (both call this right before SceneManager::open_scene) - whichever
    // scene that switches to sets its own correct mode in its own
    // constructor the very next frame anyway.
    app.window_manager.context.mouse().set_relative_mouse_mode(!app.mouse_free());
    if let Some(backdrop) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "PauseBackdrop") {
        backdrop.set_active(false);
    }
    for panel_id in PAUSE_PANELS {
        if let Some(panel) = Ui::get_ui_node(&mut app.ui.renderizable_elements, panel_id) {
            panel.set_active(false);
        }
    }

    if let Some((game_ui_active, velocity_marker_active)) = app.paused_hud_visibility.take() {
        if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "game_ui") {
            node.set_active(game_ui_active);
        }
        if let Some(node) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "velocity_marker") {
            node.set_active(velocity_marker_active);
        }
    }
}

/// Registers the pause menu's own layer ("PauseBackdrop"/"Pause"/"Settings",
/// all built inactive) - call once from `GameLogic::finish`. Reuses
/// main_menu's own gradient backdrop and Settings panel wholesale (see
/// `main_menu::ui::backdrop`/`settings_ui`'s own doc comments) rather than
/// rebuilding either from scratch.
pub fn build_pause_menu(app: &mut App) {
    let pause_panel = UiNode::container()
        .set_size(SizeValue::Grow, SizeValue::Grow)
        .set_background_color(UiColor::TRANSPARENT)
        .set_child(
            "Buttons",
            UiNode::container()
                .set_orientation(Orientation::Vertical)
                // Fixed width, not Fit - a Fit container sizes itself around
                // its children's *natural* size, but a Grow-width child (every
                // button below, so they all line up to the same edge) defers
                // its own size to whatever the container decides instead of
                // contributing to that decision - so Fit here would size the
                // menu to whichever child ISN'T Grow ("title"), too narow for
                // "Back to Main Menu" and clipping it. Wide enough for that
                // longest label with room to spare.
                .set_size(SizeValue::Pixels(320.0), SizeValue::Fit)
                .set_position(PositionValue::Start(40.0), PositionValue::Center(0.0))
                .set_child("title", label(app, "Paused").set_font_size(&mut app.ui.text.font_system, 50.0))
                .set_child("Resume", with_left_accent(button(app, "Resume", close_pause_menu).set_size(SizeValue::Grow, SizeValue::Fit)))
                .set_child("Restart", with_left_accent(button(app, "Restart", |app: &mut App| {
                    close_pause_menu(app);
                    app.scene_manager.open_scene("playing");
                }).set_size(SizeValue::Grow, SizeValue::Fit)))
                .set_child("Settings", with_left_accent(button(app, "Settings", |app: &mut App| show_pause_panel(app, "Settings")).set_size(SizeValue::Grow, SizeValue::Fit)))
                .set_child("MainMenu", with_left_accent(button(app, "Back to Main Menu", |app: &mut App| {
                    close_pause_menu(app);
                    app.scene_manager.open_scene("main_menu");
                }).set_size(SizeValue::Grow, SizeValue::Fit)))
        )
        .active(false);

    let settings = main_menu_ui::settings_ui(app, |app| show_pause_panel(app, "Pause")).active(false);

    Layer::new(app)
        .set_child("PauseBackdrop", main_menu_ui::backdrop())
        .set_child("Pause", pause_panel)
        .set_child("Settings", settings)
        .build(app);

    // Layer::build resolves every child but doesn't know any of these should
    // start hidden - PauseBackdrop has no .active(false) of its own to chain
    // (main_menu::ui::backdrop() is shared with main_menu, which wants it
    // active immediately), so it's set explicitly here instead.
    if let Some(backdrop) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "PauseBackdrop") {
        backdrop.set_active(false);
    }
    app.ui.always_on_top.push("PauseBackdrop".to_owned());
    app.ui.always_on_top.push("Pause".to_owned());
    app.ui.always_on_top.push("Settings".to_owned());
}

/// Shows the death screen - see `play::scene::GameLogic::check_water_death`'s
/// own doc comment for what actually triggers this. Sets DeathBackdrop/
/// DeathPanel active immediately, no fade-in (the black screen should read as
/// "quite instantly", per the request this came out of) - unlike the pause
/// menu's own gradient backdrop, which is meant to be a translucent overlay,
/// not a hard cut.
#[allow(dead_code)] // not shown for now - see play::scene::GameLogic::check_crash
pub fn open_death_screen(app: &mut App) {
    // Frees the cursor so Restart/Back to Main Menu are actually clickable -
    // same reasoning as open_pause_menu's own doc comment.
    app.window_manager.context.mouse().set_relative_mouse_mode(false);
    if let Some(backdrop) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "DeathBackdrop") {
        backdrop.set_active(true);
    }
    if let Some(panel) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "DeathPanel") {
        panel.set_active(true);
    }
}

/// Closes the death screen - only ever called right before Restart/Back-to-
/// menu tears the whole scene down anyway via `SceneManager::open_scene`
/// (same reasoning as `close_pause_menu`'s own doc comment - keeps state
/// clean before the reset, not because there's any "resume" affordance from
/// death).
fn close_death_screen(app: &mut App) {
    // Harmless even though this is always followed by SceneManager::
    // open_scene - see close_pause_menu's own doc comment on the same call.
    app.window_manager.context.mouse().set_relative_mouse_mode(true);
    if let Some(backdrop) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "DeathBackdrop") {
        backdrop.set_active(false);
    }
    if let Some(panel) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "DeathPanel") {
        panel.set_active(false);
    }
}

/// Registers the death screen's own layer ("DeathBackdrop"/"DeathPanel", both
/// built inactive) - call once from `GameLogic::finish`, same as
/// `build_pause_menu`. A plain solid-black backdrop rather than the pause
/// menu's own gradient one (`main_menu_ui::backdrop()`) - this is meant to
/// read as "the screen went black", not a translucent overlay you can still
/// see gameplay through.
pub fn build_death_screen(app: &mut App) {
    let backdrop = UiNode::container()
        .set_size(SizeValue::Percent(100.0), SizeValue::Percent(100.0))
        .set_background_color(UiColor::Rgba(0, 0, 0, 255))
        .active(false);

    let panel = UiNode::container()
        .set_orientation(Orientation::Vertical)
        .set_size(SizeValue::Pixels(360.0), SizeValue::Fit)
        .set_position(PositionValue::Center(0.0), PositionValue::Center(0.0))
        .set_background_color(UiColor::TRANSPARENT)
        .set_child("title", label(app, "YOU DIED").set_font_size(&mut app.ui.text.font_system, 50.0))
        .set_child("Restart", with_left_accent(button(app, "Restart", |app: &mut App| {
            close_death_screen(app);
            app.scene_manager.open_scene("playing");
        }).set_size(SizeValue::Grow, SizeValue::Fit)))
        .set_child("MainMenu", with_left_accent(button(app, "Back to Main Menu", |app: &mut App| {
            close_death_screen(app);
            app.scene_manager.open_scene("main_menu");
        }).set_size(SizeValue::Grow, SizeValue::Fit)))
        .active(false);

    Layer::new(app)
        .set_child("DeathBackdrop", backdrop)
        .set_child("DeathPanel", panel)
        .build(app);

    app.ui.always_on_top.push("DeathBackdrop".to_owned());
    app.ui.always_on_top.push("DeathPanel".to_owned());
}

/// F3 debug view - a readout of fps and player position. Built inactive;
/// nothing here is driven by a ui_track - this isn't part of the authored scene timeline,
/// it's a dev tool toggled by the same "toggle_console" action as
/// `debug_text!`'s own console.
///
/// Three separate single-line labels stacked in a `data_box`-style container
/// (see `build_game_ui`'s own "data_box" - same shape: `Fit` size, padding,
/// gap, blurred dark backing), not one multi-line label - `Label`'s own
/// vertical centering (`vertical_positioning_in_rect`) positions around a
/// single line's height regardless of how many lines are actually in the
/// buffer, so a multi-line label here was pushing its first lines above its
/// own clip bounds (only the last line stayed visible). Every other label in
/// this HUD is single-line, which is why nothing else has hit this.
pub fn build_debug_panel(app: &mut App) -> UiNode {
    let stats_line = |app: &mut App, text: &str| {
        label(app, text)
            .set_size(SizeValue::Pixels(320.0), SizeValue::Pixels(20.0))
            .set_font_size(&mut app.ui.text.font_system, 14.0)
            .set_text_color(UiColor::Rgb(0, 255, 0))
            .set_align(Align::Left)
    };

    let stats_box = UiNode::container()
        .set_size(SizeValue::Fit, SizeValue::Fit)
        .set_position(PositionValue::Start(10.0), PositionValue::Start(10.0))
        .set_padding(20.0)
        .set_gap(5.0)
        .set_background_color(UiColor::Rgba(0, 0, 0, 128))
        .set_background_blur(20.0)
        .set_child("fps", stats_line(app, "0 FPS"))
        .set_child("position", stats_line(app, "Player position: (0, 0, 0)"))
        .set_child("bank", stats_line(app, "Bank: 0°"))
        .set_child("ocean", stats_line(app, "Ocean: -"));

    UiNode::container()
        .set_size(SizeValue::Percent(100.0), SizeValue::Percent(100.0))
        .set_position(PositionValue::Start(0.0), PositionValue::Start(0.0))
        .set_background_color(UiColor::TRANSPARENT)
        .set_child("stats_box", stats_box)
        .active(false)
}

