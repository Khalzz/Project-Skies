use glyphon::Color;

use crate::engine::ui::ui_node::UiNode;
use crate::engine::ui::ui_transform::{PositionValue, SizeValue};

pub fn card() -> UiNode {
  UiNode::container()
    .set_corner_radius(10.0)
    .set_padding(10.0)
    .set_gap(5.0)
    .set_background_color([0.0, 0.0, 0.0, 0.9])
    .set_text_color(Color::rgba(200, 200, 200, 255))
}
