use glyphon::cosmic_text::Align::{Center, Left};

use glyphon::Weight;

use crate::app::App;
use crate::engine::ui::components::label::Font;
use crate::engine::ui::color::{Fill, GradientDirection, GradientStop, UiColor};
use crate::engine::ui::ui_node::UiNode;
use crate::engine::ui::ui_transform::{Anchor, BorderEdges, ChildAnchor, SizeValue};
use crate::engine::ui::ui_transform::Orientation::Horizontal;
use crate::engine::ui::ui_transform::PositionValue;

/// A full-screen dark gradient fade - opaque on the left, ramping down to fully
/// transparent by ~30% across, then staying transparent the rest of the way -
/// the main menu's own background treatment (see `main_menu::ui::build`'s
/// "Backdrop"). Shared with `main_menu::ui::show_panel`, which lerps a single
/// persistent backdrop node between this and a solid dark `Fill` as the player
/// navigates between panels - see that module for why it's one shared node
/// rather than each panel carrying its own copy (a per-panel copy can't animate
/// across the switch, since an inactive node's own transition state is frozen,
/// not ticking - see `UiNode::node_content_preparation`'s `is_active` guard).
pub fn main_fade_gradient() -> Fill {
    Fill::gradient(GradientDirection::ToRight, [
        GradientStop::new(0.0, UiColor::Rgba(0, 0, 0, 250)),
        GradientStop::new(0.3, UiColor::Rgba(0, 0, 0, 200)),
        GradientStop::new(1.0, UiColor::TRANSPARENT),
    ])
}

// On/off switch palette - the same "keycap" language as the settings chips
// (see `main_menu::ui::keycap_style`): faint at rest, lit near-white when on.
const SWITCH_TRACK_OFF: UiColor = UiColor::Rgba(255, 255, 255, 28);
const SWITCH_TRACK_ON: UiColor = UiColor::Rgba(255, 255, 255, 225);
const SWITCH_KNOB_OFF: UiColor = UiColor::Rgb(220, 220, 220);
const SWITCH_KNOB_ON: UiColor = UiColor::Rgb(20, 20, 20);

/// An on/off switch: a pill-shaped track with a round knob sitting at its
/// left end (off) or right end (on). Purely visual - whatever owns it flips
/// it with `set_toggle_switch` (see `main_menu::ui::settings_toggle_row`).
pub fn toggle_switch(on: bool) -> UiNode {
    let mut switch = UiNode::container()
        .set_size(SizeValue::Pixels(44.0), SizeValue::Pixels(24.0))
        .set_padding(3.0)
        .set_corner_radius(12.0)
        .set_border_width(1.0)
        .set_border_color(UiColor::Rgba(255, 255, 255, 45))
        .set_transition(120.0)
        .set_child("Knob",
            UiNode::container()
                .set_size(SizeValue::Pixels(18.0), SizeValue::Pixels(18.0))
                .set_corner_radius(9.0)
                .set_transition(120.0));
    set_toggle_switch(&mut switch, on);
    switch
}

/// Moves `switch`'s knob to the matching end and recolors it - `switch` must
/// be a node built by `toggle_switch`. The caller still has to mark the UI
/// changed (`app.ui.has_changed`) so the new layout gets resolved.
pub fn set_toggle_switch(switch: &mut UiNode, on: bool) {
    let (track, knob_color, knob_side) = if on {
        (SWITCH_TRACK_ON, SWITCH_KNOB_ON, Anchor::End)
    } else {
        (SWITCH_TRACK_OFF, SWITCH_KNOB_OFF, Anchor::Start)
    };
    switch.transform.child_anchor = ChildAnchor { horizontal: knob_side, vertical: Anchor::Center };
    switch.update_style(|s| s.set_background_color(track));
    if let Some(knob) = switch.get_children_mut().and_then(|children| children.iter_mut().find(|(id, _)| id == "Knob")) {
        knob.1.update_style(|s| s.set_background_color(knob_color));
    }
}

// Slider - a thin bar, lit up to a round knob, faint past it. Same palette
// as the switch above.
const SLIDER_WIDTH: f32 = 240.0;
const SLIDER_KNOB: f32 = 16.0;
const SLIDER_BAR_HEIGHT: f32 = 4.0;

/// A horizontal slider showing `fraction` (0..1). Purely visual, like
/// `toggle_switch` - whatever owns it reads the mouse against its rect
/// (`slider_fraction_at`) and moves it with `set_slider`. The whole node is
/// the knob's height, so it's easy to grab, not just the thin bar.
pub fn slider(fraction: f32) -> UiNode {
    // Sized here rather than via set_slider - `resolve` applies each node's
    // .set_size(...) once after build, over anything written before it.
    let (fill, rest) = slider_widths(fraction);
    UiNode::container()
        .set_orientation(Horizontal)
        .set_size(SizeValue::Pixels(SLIDER_WIDTH), SizeValue::Pixels(SLIDER_KNOB))
        .set_child_anchor(Anchor::Start, Anchor::Center)
        .set_background_color(UiColor::TRANSPARENT)
        .set_child("Fill", UiNode::container()
            .set_size(SizeValue::Pixels(fill), SizeValue::Pixels(SLIDER_BAR_HEIGHT))
            .set_background_color(SWITCH_TRACK_ON))
        .set_child("Knob", UiNode::container()
            .set_size(SizeValue::Pixels(SLIDER_KNOB), SizeValue::Pixels(SLIDER_KNOB))
            .set_corner_radius(SLIDER_KNOB / 2.0)
            .set_background_color(SWITCH_KNOB_OFF))
        .set_child("Rest", UiNode::container()
            .set_size(SizeValue::Pixels(rest), SizeValue::Pixels(SLIDER_BAR_HEIGHT))
            .set_background_color(SWITCH_TRACK_OFF))
}

/// The lit/faint bar widths either side of the knob at `fraction`.
fn slider_widths(fraction: f32) -> (f32, f32) {
    let travel = SLIDER_WIDTH - SLIDER_KNOB;
    let fraction = fraction.clamp(0.0, 1.0);
    (travel * fraction, travel * (1.0 - fraction))
}

/// Moves `slider`'s knob to `fraction` (0..1) - `slider` must be a node built
/// by `slider`. The caller still has to mark the UI changed.
pub fn set_slider(slider: &mut UiNode, fraction: f32) {
    let (fill, rest) = slider_widths(fraction);
    if let Some(children) = slider.get_children_mut() {
        for (id, child) in children.iter_mut() {
            match id.as_str() {
                "Fill" => child.transform.width = fill,
                "Rest" => child.transform.width = rest,
                _ => {}
            }
        }
    }
}

/// Where along `slider` (built by `slider`) the mouse is, 0..1 - the knob's
/// center follows the cursor, clamped to the ends.
pub fn slider_fraction_at(slider: &UiNode, mouse_x: f32) -> f32 {
    let travel = SLIDER_WIDTH - SLIDER_KNOB;
    ((mouse_x - slider.transform.rect.left - SLIDER_KNOB / 2.0) / travel).clamp(0.0, 1.0)
}

pub fn card() -> UiNode {
  UiNode::container()
    .set_corner_radius(10.0)
    .set_padding(10.0)
    .set_gap(5.0)
    .set_background_color(UiColor::Rgba(0, 0, 0, 230))
    .set_text_color(UiColor::Rgb(200, 200, 200))
}

/// Inter Regular (bundled, see `Ui::new`) - sturdier and easier to read at
/// small HUD sizes than the default sans-serif, and the same on every platform.
pub const HUD_FONT: Font = Font { family: Some("Inter"), weight: Weight::NORMAL };

pub fn label(app: &mut App, label: &str) -> UiNode {
  UiNode::label(&mut app.ui.text.font_system, label, None, None)
}

pub fn button(app: &mut App, text: &str, on_click: impl Fn(&mut App) + 'static) -> UiNode {
  label(app, text)
    .set_text_color(UiColor::Rgb(200, 200, 200))
    // .set_corner_radius(10.0)
    .set_padding((10.0, 2.0))
    .set_font_size(&mut app.ui.text.font_system, 20.0)
    .on_hover(|s| s.set_background_color(UiColor::Rgba(0, 0, 0, 128)).set_text_color(UiColor::WHITE))
    .set_transition(50.0)
    .on_click(on_click)
    .set_border_width(1.0)
}

/// Wraps a node (typically a `button(...)`) with a white left-edge accent bar
/// that's invisible at rest and fades in on hover - the main menu's visual
/// language (see `main_menu::ui::build`) for "this is interactive/selected",
/// used in place of a full border or a background box. Merges with (rather than
/// replacing) whatever hover effect `node` already has - see `UiNode::on_hover`'s
/// own doc comment.
///
/// border_edges/border_width are set on the *resting* style too, identically to
/// hover's - only border_color actually differs between the two (transparent vs
/// white). Both of those are lerped by `.set_transition(...)` same as color is;
/// setting them only on hover made the accent visibly widen alongside the color
/// fading in, reading as "growing" rather than just appearing - keeping
/// width/edges constant across both states leaves color as the only thing that
/// actually animates.
pub fn with_left_accent(node: UiNode) -> UiNode {
    node
        .set_border_edges(BorderEdges::LEFT)
        .set_border_width(3.0)
        .set_border_color(UiColor::TRANSPARENT)
        .on_hover(|s| s.set_border_color(UiColor::WHITE))
}

pub fn modal(app: &mut App) -> UiNode {
  card()
    .set_position(PositionValue::Center(0.0), PositionValue::Center(0.0))
    .set_child("title", label(app, "Este es un ejemplo de un modal").set_align(Left))
    .set_child("options", card()
      .set_padding(0.0)
      .set_background_color(UiColor::TRANSPARENT)
      .set_orientation(Horizontal)
      .set_child("Cancel", button(app, "Cancel", |_| {}).set_align(Center).set_border_color(UiColor::WHITE))
      .set_child("Accept", button(app, "Accept", |_| {}).set_align(Center).set_border_color(UiColor::WHITE))
    )
}