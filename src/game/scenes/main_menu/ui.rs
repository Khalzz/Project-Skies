use serde::Deserialize;

use crate::app::App;
use crate::engine::input::input;
use crate::engine::rendering::ui::ui::Ui;
use crate::engine::ui::color::{Fill, UiColor};
use crate::engine::ui::layer::Layer;
use crate::engine::ui::ui_node::UiNode;
use crate::engine::ui::ui_transform::{Anchor, BorderEdges, Orientation, PositionValue, SizeValue};
use crate::game::selected_level::{SelectedLevel, SELECTED_LEVEL};
use crate::game::ui::{button, label, main_fade_gradient, with_left_accent};
use super::rebind_modal;

const SETTINGS_TABS: [&str; 3] = ["Video", "Controller", "Audio"];
const SETTINGS_BACKDROP: UiColor = UiColor::Rgba(0, 0, 0, 250);
const SETTINGS_BLUR: f32 = 64.0;

pub(crate) fn backdrop() -> UiNode {
    UiNode::container()
        .set_size(SizeValue::Percent(100.0), SizeValue::Percent(100.0))
        .set_position(PositionValue::Start(0.0), PositionValue::Start(0.0))
        .set_background_color(main_fade_gradient())
        .set_transition(400.0)
}

fn show_panel(app: &mut App, id: &str) {
    for panel_id in ["Main Menu", "Settings", "Play Select"] {
        if let Some(panel) = Ui::get_ui_node(&mut app.ui.renderizable_elements, panel_id) {
            panel.set_active(panel_id == id);
        }
    }
    if let Some(backdrop) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "Backdrop") {
        // Same flat dark backdrop for Play Select as Settings - same reasoning
        // (SETTINGS_BACKDROP's own doc comment): both have enough of their own
        // sidebar+content layout that a flat backdrop reads better than the
        // main menu's left-to-transparent gradient repeating behind it.
        let dark_panel = id == "Settings" || id == "Play Select";
        let color = if dark_panel { Fill::Solid(SETTINGS_BACKDROP) } else { main_fade_gradient() };
        let blur = if dark_panel { SETTINGS_BLUR } else { 0.0 };
        backdrop.update_style(|s| s.set_background_color(color).set_background_blur(blur));
    }
}

fn show_settings_tab(app: &mut App, id: &str) {
    for tab_id in SETTINGS_TABS {
        if let Some(tab) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &format!("Settings/Content/{tab_id}")) {
            tab.set_active(tab_id == id);
        }
        // Persistent "this is the open tab" indicator on the sidebar button itself -
        // direct style mutation (same pattern as e.g. play.rs's blinking altitude
        // alert), not on_hover/on_press, since it needs to stay showing regardless
        // of whether the mouse is currently over/down on the button at all. Both
        // border and text color are driven from here so they can't drift out of
        // sync with each other as the selected tab changes. Only border_color needs
        // touching - border_edges/border_width are already set once, identically
        // for every state, by with_left_accent (see its own doc comment on why).
        if let Some(button) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &format!("Settings/Sidebar/{tab_id}")) {
            let selected = tab_id == id;
            let border = if selected { UiColor::WHITE } else { UiColor::TRANSPARENT };
            let text = if selected { UiColor::WHITE } else { UiColor::Rgb(200, 200, 200) };
            button.update_style(|s| s.set_border_color(border).set_text_color(text));
        }
    }
}

/// A `with_left_accent` button sized for a sidebar row - `button()`'s own
/// default padding, same height as the main menu's own top-level Play/
/// Settings/Quit buttons (see `build`), not overridden to something taller.
pub(crate) fn sidebar_button(app: &mut App, text: &str, on_click: impl Fn(&mut App) + 'static) -> UiNode {
    with_left_accent(button(app, text, on_click).set_size(SizeValue::Grow, SizeValue::Fit))
}

/// An invisible spacer that claims *all* the leftover main-axis space in its
/// container (see `SizeValue::Grow`) - used to pin whatever comes after it to
/// the far edge instead of stacking directly under whatever comes before,
/// CSS flexbox's `justify-content: space-between` for two groups. See the
/// sidebar below: Title/tabs stay pinned to the top, "Back" gets pushed all
/// the way down to the bottom instead of floating in the middle of the column.
pub fn grow_spacer() -> UiNode {
    UiNode::container().set_size(SizeValue::Grow, SizeValue::Grow).set_background_color(UiColor::TRANSPARENT)
}

/// Same visual language as the main menu itself (see `build`, below) - shares
/// its background with "Main Menu" via the persistent "Backdrop" node (see
/// `show_panel`) instead of a boxed/blurred panel of its own, a left-aligned
/// sidebar of `with_left_accent`-styled buttons (persistent accent on the
/// selected tab, hover preview on the others) instead of a tab strip with
/// borders on every side, and a transparent content area to its right.
///
/// `on_back` is what the sidebar's own "Back" button does - the main menu's
/// own call site passes `|app| show_panel(app, "Main Menu")`, but this fn has
/// no dependency on that panel existing (see `play::ui`'s pause-menu reuse of
/// this same fn, which passes its own equivalent instead) - only the tab
/// switching (`show_settings_tab`, hardcoded to this node's own
/// "Settings/Content/{tab}"/"Settings/Sidebar/{tab}" children) assumes this
/// tree is always registered under the literal id "Settings", which is safe
/// across scenes since only one scene's UI tree is ever alive at a time (see
/// `SceneManager`'s reset semantics, which clear `app.ui` before a scene
/// rebuilds its own).
pub(crate) fn settings_ui(app: &mut App, on_back: impl Fn(&mut App) + 'static) -> UiNode {
    UiNode::container()
        .set_background_color(UiColor::TRANSPARENT)
        .set_size(SizeValue::Grow, SizeValue::Grow)
        .set_orientation(Orientation::Horizontal)
        .set_child("Sidebar",
            UiNode::container()
                .set_orientation(Orientation::Vertical)
                .set_background_color(UiColor::TRANSPARENT)
                .set_size(SizeValue::Pixels(240.0), SizeValue::Grow)
                .set_padding([60.0, 60.0, 40.0, 40.0])
                .set_child_anchor(Anchor::Start, Anchor::Start)
                .set_gap(8.0)
                .set_child("Title", label(app, "Settings").set_font_size(&mut app.ui.text.font_system, 32.0).set_text_color(UiColor::WHITE))
                .set_child("Controller", sidebar_button(app, "Controller", |app| show_settings_tab(app, "Controller")))
                .set_child("Video", sidebar_button(app, "Video", |app| show_settings_tab(app, "Video")))
                .set_child("Audio", sidebar_button(app, "Audio", |app| show_settings_tab(app, "Audio")))
                .set_child("BackSpacer", grow_spacer())
                .set_child("Back", sidebar_button(app, "Back", on_back))
        )
        .set_child("Content",
            UiNode::container()
                .set_background_color(UiColor::TRANSPARENT)
                .set_size(SizeValue::Grow, SizeValue::Grow)
                .set_padding([60.0, 60.0, 30.0, 30.0])
                .set_child_anchor(Anchor::Start, Anchor::Start)
                .set_gap(20.0)
                .set_child("Controller",
                    UiNode::container()
                        .set_background_color(UiColor::TRANSPARENT)
                        // Grow, not Fit - needs to fill Content's full available
                        // height so List (below) has a real bounded remainder to
                        // scroll within, instead of auto-growing to fit however
                        // tall List's own content ends up being.
                        .set_size(SizeValue::Grow, SizeValue::Grow)
                        .set_gap(20.0)
                        .set_child("Title", label(app, "Controller").set_font_size(&mut app.ui.text.font_system, 26.0).set_text_color(UiColor::WHITE))
                        .set_child("List", controller_settings_list(app))
                        .active(true))
                .set_child("Video", label(app, "Video Settings").set_font_size(&mut app.ui.text.font_system, 26.0).set_text_color(UiColor::WHITE).active(false))
                .set_child("Audio", label(app, "Audio Settings").set_font_size(&mut app.ui.text.font_system, 26.0).set_text_color(UiColor::WHITE).active(false))
        )
        .active(false)
}

/// Which actions get a row here, and in what order - see settings/controller.ron
/// (a curated subset of settings/input.ron's full action list: debug/UI-internal
/// actions aren't meant to be player-rebindable, so they're left out of this file
/// entirely rather than filtered here).
const CONTROLLER_ACTIONS_RON: &str = include_str!("../../../../settings/controller.ron");

/// One section per action, stacked in a single column - see
/// `controller_settings_section`.
fn controller_settings_list(app: &mut App) -> UiNode {
    let actions: Vec<String> = ron::from_str(CONTROLLER_ACTIONS_RON).expect("Failed to parse settings/controller.ron");

    let mut list = UiNode::container()
        .set_background_color(UiColor::TRANSPARENT)
        // Grow, not Fit - a bounded viewport for scroll to actually happen
        // within (see set_scrollable's own doc comment), rather than always
        // growing tall enough to fit every row. Right padding reserves a
        // gutter for the scrollbar so it doesn't overlap each row's own
        // right edge/content.
        .set_size(SizeValue::Grow, SizeValue::Grow)
        .set_padding([0.0, 0.0, 0.0, 16.0])
        .set_scrollable(true);

    for action in actions {
        let section = controller_settings_section(app, &action);
        list = list.set_child(action, section);
    }

    list
}

/// A single action's settings section - a bare row (no resting background)
/// separated from the next one by a thin bottom border rather than each being
/// its own tinted card, holding its display name beside every one of its
/// current bindings as its own small clickable "chip" (see
/// `controller_chip_nodes`) laid out horizontally, ending in a "+" chip to add
/// another. Clicking an existing chip opens the rebind modal (see
/// `rebind_modal::open`) to replace *that* binding in place (and offers
/// "Delete Bind"); clicking "+" opens the same modal to append a new one
/// instead (no "Delete Bind" - there's nothing to delete yet).
///
/// The row itself has no hover effect of its own (tried a background tint -
/// even a fairly strong one never read as clearly "changing" next to the
/// chips' own much more decisive hover darken right beside it) - only the
/// chips (see `controller_chip_nodes`) highlight on hover.
fn controller_settings_section(app: &mut App, action: &str) -> UiNode {
    let mut chips = UiNode::container()
        .set_orientation(Orientation::Horizontal)
        .set_background_color(UiColor::TRANSPARENT)
        .set_size(SizeValue::Fit, SizeValue::Fit)
        .set_gap(6.0);
    for (key, chip) in controller_chip_nodes(app, action) {
        chips = chips.set_child(key, chip);
    }

    UiNode::container()
        .set_orientation(Orientation::Horizontal)
        .set_background_color(UiColor::TRANSPARENT)
        .set_border_edges(BorderEdges::BOTTOM)
        .set_border_width(1.0)
        .set_border_color(UiColor::Rgba(255, 255, 255, 25))
        .set_padding(14.0)
        .set_size(SizeValue::Grow, SizeValue::Fit)
        .set_child_anchor(Anchor::Start, Anchor::Center)
        .set_gap(16.0)
        .set_child("Name", label(app, &rebind_modal::display_name(action)).set_size(SizeValue::Pixels(150.0), SizeValue::Fit).set_text_color(UiColor::Rgb(200, 200, 200)))
        .set_child("Chips", chips)
}

/// The "keycap" color/hover treatment shared by the settings chips (see
/// `chip_style`, below) and the rebind modal's own "Bind"/"Delete Bind"/
/// "Cancel"/"Save" buttons (see `main_menu::rebind_modal::build`) - one
/// consistent button language across every clickable element in the
/// controller-rebinding flow. Resting state reads as a "keycap": a faint
/// outline over a barely-there fill. Hover is a full inversion rather than a
/// token brightness bump - solid near-white fill, near-black text - a "this
/// key is lit up" look, not just `button()`'s own baked-in hover (a black
/// darken, which reads backward here: darkening an already-faint light-tint
/// button just fades it toward invisible instead of highlighting it) nudged
/// slightly brighter. This call's own fields win over that baked-in hover
/// wherever it sets something (see `UiNode::on_hover`'s own doc comment on
/// the merge) - here that's every field `button()`'s hover touches, so
/// nothing bleeds through from it. Its own `.set_transition(50.0)` is
/// overridden too - too fast a fade for this much bigger a color jump reads
/// as a harsh flash rather than a smooth light-up (see
/// `controller_settings_section`'s own transition-duration comment for the
/// same reasoning in more detail).
///
/// Doesn't size the node at all (beyond the padding baked into the look
/// itself) - callers that want a compact, roughly-square chip layer
/// `.set_min_width_to_height()` on top (see `chip_style`); callers that want
/// their own size (the modal's Grow-width "Bind" button) just set it directly.
pub fn keycap_style(node: UiNode) -> UiNode {
    node
        .set_padding((10.0, 6.0))
        .set_background_color(UiColor::Rgba(255, 255, 255, 28))
        .set_border_color(UiColor::Rgba(255, 255, 255, 45))
        .set_text_color(UiColor::Rgb(220, 220, 220))
        .set_align(glyphon::cosmic_text::Align::Center)
        .set_corner_radius(8.0)
        .on_hover(|s| s
            .set_background_color(UiColor::Rgba(255, 255, 255, 225))
            .set_border_color(UiColor::WHITE)
            .set_text_color(UiColor::Rgb(20, 20, 20)))
        .set_transition(120.0)
}

/// Shared look for a small binding chip / the trailing "+" chip (see
/// `controller_chip_nodes`) - `keycap_style` squared into a compact,
/// roughly-square pill rather than a full-width row button (see
/// `set_min_width_to_height`).
fn chip_style(node: UiNode) -> UiNode {
    keycap_style(node)
        .set_min_width_to_height()
}

/// The chip nodes for `controller_settings_row`'s "Chips" child - one per
/// current binding, keyed `"chip_{index}"` (that index is exactly what a click
/// opens the rebind modal with, see `rebind_modal::open`'s `edit_index`), plus
/// a trailing `"chip_add"` "+" chip that opens the same modal with no index
/// (appending a new binding instead of replacing one). Also called directly by
/// `rebind_modal::refresh_chips` to rebuild just this row's chips in place
/// after a binding's added/replaced/deleted, without touching the rest of the
/// settings tree.
pub fn controller_chip_nodes(app: &mut App, action: &str) -> Vec<(String, UiNode)> {
    let bindings = input::action_bindings(action);
    let mut nodes = Vec::with_capacity(bindings.len() + 1);

    for (index, binding) in bindings.iter().enumerate() {
        let action = action.to_owned();
        // No with_left_accent here (unlike most other buttons in this file) -
        // its hover-in white left-edge bar isn't wanted on these small chips,
        // just the plain background/text hover button() already gives.
        let chip = chip_style(button(app, &binding.label().to_uppercase(), move |app| rebind_modal::open(app, &action, Some(index))));
        nodes.push((format!("chip_{index}"), chip));
    }

    let action = action.to_owned();
    let add_chip = chip_style(button(app, "+", move |app| rebind_modal::open(app, &action, None)))
        .set_text_color(UiColor::WHITE)
        // A lone "+" glyph doesn't use any descender space, so centering it
        // by the font's full line-height (see `Label::vertical_positioning_
        // in_rect`) - which reads correctly for a letter/word chip, whose
        // glyphs roughly fill that box - visibly sits it low here. Nudging
        // top/bottom padding asymmetrically (top+bottom sum unchanged, so
        // `set_min_width_to_height`'s already-computed box size above isn't
        // affected) shifts the inner content rect's own center up without
        // resizing the chip.
        .set_padding([4.0, 8.0, 10.0, 10.0]);
    nodes.push(("chip_add".to_owned(), add_chip));

    nodes
}

/// One entry in the Play screen's level list - see `assets/levels.ron`, the
/// only data source (no code path adds/edits these at runtime, unlike the
/// controller bindings). `scene` is what `SceneManager::open_scene` expects
/// (registered in `main.rs`) - not necessarily the same string as `id`,
/// which is purely this list's own stable key (RON authoring convenience,
/// UI node paths, `SELECTED_LEVEL`). `mission_title`/`location`/
/// `mission_date` are for the in-scene mission-intro card (see `play::scene::
/// GameLogic`, the only other reader - via `SELECTED_LEVEL`), not this list's
/// own `name`/`description` - see `assets/levels.ron`'s own doc comment.
#[derive(Deserialize, Clone)]
struct LevelInfo {
    id: String,
    name: String,
    description: String,
    scene: String,
    mission_title: String,
    location: String,
    mission_date: String,
}

const LEVELS_RON: &str = include_str!("../../../../assets/levels.ron");

fn levels() -> Vec<LevelInfo> {
    ron::from_str(LEVELS_RON).expect("Failed to parse assets/levels.ron")
}

/// Selects `id` as the current level: stages it for the "Play" button (see
/// `SELECTED_LEVEL`, which `play::scene::GameLogic::new` reads back once the
/// chosen scene actually opens - see its own doc comment for why this static
/// exists at all), fills in Content's name/description text, and updates
/// every sidebar button's persistent selected-highlight - same direct-style-
/// mutation pattern as `show_settings_tab`, see its own doc comment for why
/// (needs to stay showing regardless of hover/press state). No-op if `id`
/// isn't in `assets/levels.ron`.
fn show_level(app: &mut App, id: &str) {
    let Some(level) = levels().into_iter().find(|l| l.id == id) else { return };
    *SELECTED_LEVEL.lock().unwrap() = Some(SelectedLevel {
        scene: level.scene.clone(),
        mission_title: level.mission_title.clone(),
        location: level.location.clone(),
        mission_date: level.mission_date.clone(),
    });

    if let Some(name) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "Play Select/Main/Content/Name").and_then(|n| n.as_label_mut()) {
        name.set_text(&mut app.ui.text.font_system, &level.name, false);
    }
    if let Some(description) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "Play Select/Main/Content/Description").and_then(|n| n.as_label_mut()) {
        description.set_text(&mut app.ui.text.font_system, &level.description, false);
    }
    for other in levels() {
        if let Some(button) = Ui::get_ui_node(&mut app.ui.renderizable_elements, &format!("Play Select/Main/Sidebar/List/{}", other.id)) {
            let selected = other.id == id;
            let border = if selected { UiColor::WHITE } else { UiColor::TRANSPARENT };
            let text = if selected { UiColor::WHITE } else { UiColor::Rgb(200, 200, 200) };
            button.update_style(|s| s.set_border_color(border).set_text_color(text));
        }
    }
    app.ui.has_changed = true;
}

/// A scrollable list of level buttons on the left (see `set_scrollable`'s own
/// doc comment - same pattern as the Controller settings list, just Pixels-
/// width instead of a sidebar-wide Grow) and a details panel on the right:
/// a placeholder preview box (no real level art exists yet - see
/// `assets/levels.ron`'s own doc comment), name, and description. Below both
/// (see `footer_button`), a single full-width "Actions" bar holds "Back" and
/// "Play" together - separated from the Sidebar/Content pair above it by a
/// top border, rather than "Back" living in the Sidebar and "Play" pinned to
/// Content's own bottom-right corner as if they belonged to two unrelated
/// sections.
fn play_select_ui(app: &mut App) -> UiNode {
    let mut level_list = UiNode::container()
        .set_orientation(Orientation::Vertical)
        .set_background_color(UiColor::TRANSPARENT)
        .set_size(SizeValue::Grow, SizeValue::Grow)
        .set_gap(6.0)
        .set_scrollable(true);
    for level in levels() {
        let id = level.id.clone();
        let row = sidebar_button(app, &level.name, move |app| show_level(app, &id));
        level_list = level_list.set_child(level.id, row);
    }

    UiNode::container()
        .set_orientation(Orientation::Vertical)
        .set_background_color(UiColor::TRANSPARENT)
        .set_size(SizeValue::Grow, SizeValue::Grow)
        .set_child("Main",
            UiNode::container()
                .set_background_color(UiColor::TRANSPARENT)
                .set_size(SizeValue::Grow, SizeValue::Grow)
                .set_orientation(Orientation::Horizontal)
                .set_child("Sidebar",
                    UiNode::container()
                        .set_orientation(Orientation::Vertical)
                        .set_background_color(UiColor::TRANSPARENT)
                        // A bit wider than Settings' own 240px sidebar - level
                        // names run longer than a settings tab label.
                        .set_size(SizeValue::Pixels(320.0), SizeValue::Grow)
                        .set_padding([60.0, 20.0, 40.0, 20.0])
                        .set_child_anchor(Anchor::Start, Anchor::Start)
                        .set_gap(16.0)
                        .set_child("Title", label(app, "Select Mission").set_font_size(&mut app.ui.text.font_system, 32.0).set_text_color(UiColor::WHITE))
                        .set_child("List", level_list)
                )
                .set_child("Content",
                    UiNode::container()
                        .set_orientation(Orientation::Vertical)
                        .set_background_color(UiColor::TRANSPARENT)
                        .set_size(SizeValue::Grow, SizeValue::Grow)
                        .set_padding([60.0, 20.0, 20.0, 60.0])
                        .set_child_anchor(Anchor::Start, Anchor::Start)
                        .set_gap(16.0)
                        .set_child("Image",
                            UiNode::container()
                                .set_background_color(UiColor::Rgba(255, 255, 255, 10))
                                .set_border_color(UiColor::Rgba(255, 255, 255, 30))
                                .set_border_width(1.0)
                                .set_corner_radius(10.0)
                                .set_size(SizeValue::Grow, SizeValue::Pixels(260.0))
                                .set_child_anchor(Anchor::Center, Anchor::Center)
                                .set_child("Placeholder", label(app, "No preview available").set_text_color(UiColor::Rgb(140, 140, 140))))
                        .set_child("Name", label(app, "").set_size(SizeValue::Grow, SizeValue::Fit).set_font_size(&mut app.ui.text.font_system, 28.0).set_text_color(UiColor::WHITE))
                        .set_child("Description", label(app, "").set_size(SizeValue::Grow, SizeValue::Fit).set_text_color(UiColor::Rgb(200, 200, 200)))
                )
        )
        .set_child("Actions",
            UiNode::container()
                .set_orientation(Orientation::Horizontal)
                .set_background_color(UiColor::TRANSPARENT)
                .set_border_edges(BorderEdges::TOP)
                .set_border_width(1.0)
                .set_border_color(UiColor::Rgba(255, 255, 255, 25))
                .set_size(SizeValue::Grow, SizeValue::Fit)
                .set_padding([20.0, 20.0, 60.0, 60.0])
                .set_child_anchor(Anchor::Start, Anchor::Center)
                .set_child("Back", footer_button(app, "Back", |app| show_panel(app, "Main Menu")))
                .set_child("Spacer", grow_spacer())
                .set_child("Play", footer_button(app, "Play", |app| {
                    if let Some(selected) = SELECTED_LEVEL.lock().unwrap().clone() {
                        app.scene_manager.open_scene(&selected.scene);
                    }
                }))
        )
        .active(false)
}

/// "Back"/"Play" in the Play screen's shared bottom "Actions" bar (see
/// `play_select_ui`) - one shared style so the two read as equally-weighted
/// peers instead of two differently-sized buttons that happen to be near
/// each other. Generous horizontal padding (wider than `sidebar_button`'s
/// own) - "Play" specifically was clipping its own leading "P" against
/// `with_left_accent`'s left border at the tighter padding this used to have
/// - but only vertical padding actually affects button *height*, so that
/// stays close to `sidebar_button`'s own (6.0) rather than matching the
/// horizontal bump 1:1.
fn footer_button(app: &mut App, text: &str, on_click: impl Fn(&mut App) + 'static) -> UiNode {
    with_left_accent(button(app, text, on_click).set_padding((28.0, 7.0)))
}

pub fn build(app: &mut App) {
    let main_menu = UiNode::container()
        .set_size(SizeValue::Grow, SizeValue::Grow)
        .set_corner_radius(0.0)
        .set_background_color(UiColor::TRANSPARENT)
        .set_child(
            "Buttons",
            UiNode::container()
                .set_orientation(Orientation::Vertical)
                .set_size(SizeValue::Fit, SizeValue::Fit)
                .set_position(PositionValue::Start(40.0), PositionValue::Center(0.0))
                .set_child("title", label(app, "Project Skies").set_font_size(&mut app.ui.text.font_system, 50.0))
                .set_child("Play", with_left_accent(button(app, "Play", |app| show_panel(app, "Play Select")).set_size(SizeValue::Grow, SizeValue::Fit)))
                .set_child("Settings", with_left_accent(button(app, "Settings", |app| show_panel(app, "Settings")).set_size(SizeValue::Grow, SizeValue::Fit)))
                .set_child("Quit", with_left_accent(button(app, "Quit", |_app| std::process::exit(0)).set_size(SizeValue::Grow, SizeValue::Fit)))
        );

    Layer::new(app)
        .set_child("Backdrop", backdrop())
        .set_child("Main Menu", main_menu)
        .set_child("Settings", settings_ui(app, |app| show_panel(app, "Main Menu")))
        .set_child("Play Select", play_select_ui(app))
        .set_child("RebindModal", rebind_modal::build(app))
        .build(app);
    app.ui.always_on_bottom.push("Backdrop".to_owned());
    app.ui.always_on_top.push("RebindModal".to_owned());
    show_settings_tab(app, "Controller");
    // Selects the first level so Content/Play aren't empty/dead the first time
    // the player opens the Play screen, before ever clicking a level button.
    if let Some(first) = levels().first() {
        show_level(app, &first.id);
    }
}
