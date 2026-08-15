use glyphon::cosmic_text::Align::{Center, Left};

use crate::app::App;
use crate::engine::ui::color::{Fill, GradientDirection, GradientStop, UiColor};
use crate::engine::ui::ui_node::UiNode;
use crate::engine::ui::ui_transform::BorderEdges;
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

pub fn card() -> UiNode {
  UiNode::container()
    .set_corner_radius(10.0)
    .set_padding(10.0)
    .set_gap(5.0)
    .set_background_color(UiColor::Rgba(0, 0, 0, 230))
    .set_text_color(UiColor::Rgb(200, 200, 200))
}

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