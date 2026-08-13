use crate::app::App;
use crate::engine::rendering::ui::ui::Ui;
use crate::engine::ui::color::{Fill, UiColor};
use crate::engine::ui::layer::Layer;
use crate::engine::ui::ui_node::UiNode;
use crate::engine::ui::ui_transform::{Anchor, Orientation, PositionValue, SizeValue};
use crate::game::ui::{button, label, main_fade_gradient, with_left_accent};
use super::rebind_modal;

const SETTINGS_TABS: [&str; 3] = ["Video", "Controller", "Audio"];

/// Settings' own background - a solid, constant tone rather than the main
/// menu's left-to-transparent gradient (that gradient exists to keep "Project
/// Skies" and the 3 buttons readable over the moving 3D scene behind them;
/// Settings has enough of its own content that a flat, slightly darker backdrop
/// reads better than a gradient repeating behind a whole sidebar+content
/// layout). See `show_panel`, which lerps the shared "Backdrop" node between
/// this and `main_fade_gradient()` as the player navigates between panels.
const SETTINGS_BACKDROP: UiColor = UiColor::Rgba(0, 0, 0, 252);

/// The single persistent full-screen background both "Main Menu" and
/// "Settings" render on top of (see `build`, which registers it in
/// `app.ui.always_on_bottom` so it's guaranteed to render behind them
/// regardless of `renderizable_elements`' HashMap iteration order - see
/// `Ui::always_on_bottom`'s own doc comment). A `.set_transition(...)` on a
/// single shared node, updated in place by `show_panel`, is what makes the
/// background *animate* between the main menu's gradient and Settings' solid
/// color as you navigate - each panel keeping its own copy (the original
/// design) can't do that, since an inactive node's own transition state is
/// frozen rather than ticking (see `UiNode::node_content_preparation`'s
/// `is_active` early return), so switching panels would just cut instantly
/// from one panel's background to the other's rather than blending between them.
fn backdrop() -> UiNode {
    UiNode::container()
        .set_size(SizeValue::Percent(100.0), SizeValue::Percent(100.0))
        .set_position(PositionValue::Start(0.0), PositionValue::Start(0.0))
        .set_background_color(main_fade_gradient())
        .set_transition(400.0)
}

fn show_panel(app: &mut App, id: &str) {
    for panel_id in ["Main Menu", "Settings"] {
        if let Some(panel) = Ui::get_ui_node(&mut app.ui.renderizable_elements, panel_id) {
            panel.set_active(panel_id == id);
        }
    }
    if let Some(backdrop) = Ui::get_ui_node(&mut app.ui.renderizable_elements, "Backdrop") {
        backdrop.style.background_color = Some(if id == "Settings" { Fill::Solid(SETTINGS_BACKDROP) } else { main_fade_gradient() });
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
            button.style.border_color = Some(if selected { UiColor::WHITE } else { UiColor::TRANSPARENT }.into());
            button.style.text_color = Some(if selected { UiColor::WHITE } else { UiColor::Rgb(200, 200, 200) });
        }
    }
}

/// A `with_left_accent` button sized/padded for a sidebar row - taller than
/// `button()`'s own default padding gives (2px top/bottom read as visually thin
/// stacked one after another down a sidebar) so each row is a comfortable
/// height and click target, without going as far as feeling padded/bloated.
fn sidebar_button(app: &mut App, text: &str, on_click: impl Fn(&mut App) + 'static) -> UiNode {
    with_left_accent(button(app, text, on_click).set_size(SizeValue::Grow, SizeValue::Fit).set_padding((14.0, 6.0)))
}

/// An invisible spacer that claims *all* the leftover main-axis space in its
/// container (see `SizeValue::Grow`) - used to pin whatever comes after it to
/// the far edge instead of stacking directly under whatever comes before,
/// CSS flexbox's `justify-content: space-between` for two groups. See the
/// sidebar below: Title/tabs stay pinned to the top, "Back" gets pushed all
/// the way down to the bottom instead of floating in the middle of the column.
fn grow_spacer() -> UiNode {
    UiNode::container().set_size(SizeValue::Grow, SizeValue::Grow).set_background_color(UiColor::TRANSPARENT)
}

/// Same visual language as the main menu itself (see `build`, below) - shares
/// its background with "Main Menu" via the persistent "Backdrop" node (see
/// `show_panel`) instead of a boxed/blurred panel of its own, a left-aligned
/// sidebar of `with_left_accent`-styled buttons (persistent accent on the
/// selected tab, hover preview on the others) instead of a tab strip with
/// borders on every side, and a transparent content area to its right.
fn settings_ui(app: &mut App) -> UiNode {
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
                .set_child("Back", sidebar_button(app, "Back", |app| show_panel(app, "Main Menu")))
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
                        .set_size(SizeValue::Grow, SizeValue::Fit)
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
const CONTROLLER_ACTIONS_RON: &str = include_str!("../../../settings/controller.ron");

/// Split into 2 side-by-side columns rather than one long list - 18 actions in
/// a single column ran well past a comfortable reading height. Rows go
/// "Left/{action}" or "Right/{action}" depending on which half they land in -
/// see rebind_modal::sync_settings_row, which checks both since it doesn't
/// track which column any given action ended up in.
fn controller_settings_list(app: &mut App) -> UiNode {
    let mut actions: Vec<String> = ron::from_str(CONTROLLER_ACTIONS_RON).expect("Failed to parse settings/controller.ron");
    let right_actions = actions.split_off(actions.len().div_ceil(2));
    let left_actions = actions;

    UiNode::container()
        .set_background_color(UiColor::TRANSPARENT)
        .set_orientation(Orientation::Horizontal)
        .set_size(SizeValue::Grow, SizeValue::Fit)
        .set_gap(30.0)
        .set_child("Left", controller_settings_column(app, left_actions))
        .set_child("Right", controller_settings_column(app, right_actions))
}

/// One row per action in `actions`, each showing its current binding(s) and
/// opening the rebind modal (see rebind_modal::open) for that specific action
/// on click.
fn controller_settings_column(app: &mut App, actions: Vec<String>) -> UiNode {
    let mut column = UiNode::container()
        .set_background_color(UiColor::TRANSPARENT)
        .set_size(SizeValue::Grow, SizeValue::Fit)
        .set_gap(6.0);

    for action in actions {
        let text = format!("{}: {}", rebind_modal::display_name(&action), rebind_modal::binding_summary(&action));
        let key = action.clone();
        let row = with_left_accent(
            button(app, &text, move |app| rebind_modal::open(app, &action))
                .set_size(SizeValue::Grow, SizeValue::Fit)
                .set_padding((16.0, 8.0))
                .set_text_color(UiColor::Rgb(220, 220, 220))
        );
        column = column.set_child(key, row);
    }

    column
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
                .set_child("Play", with_left_accent(button(app, "Play", |app| app.scene_manager.open_scene("playing")).set_size(SizeValue::Grow, SizeValue::Fit)))
                .set_child("Settings", with_left_accent(button(app, "Settings", |app| show_panel(app, "Settings")).set_size(SizeValue::Grow, SizeValue::Fit)))
                .set_child("Quit", with_left_accent(button(app, "Quit", |_app| std::process::exit(0)).set_size(SizeValue::Grow, SizeValue::Fit)))
        );

    Layer::new(app)
        .set_child("Backdrop", backdrop())
        .set_child("Main Menu", main_menu)
        .set_child("Settings", settings_ui(app))
        .set_child("RebindModal", rebind_modal::build(app))
        .build(app);
    app.ui.always_on_bottom.push("Backdrop".to_owned());
    app.ui.always_on_top.push("RebindModal".to_owned());
    show_settings_tab(app, "Controller");
}
