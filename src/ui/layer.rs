//! Root entry point for building a screen-relative UI tree - mirrors
//! `UiNode::container()`/`.set_child(...)`'s own shape so building top-level UI
//! looks the same as building nested UI.
//!
//! - `Layer::new(app)` - captures the current screen width/height once.
//! - `.set_child(id, child)` - registers a top-level node under `id`, chainable
//!   (like `UiNode::set_child`, each child stays independently addressable via
//!   `Ui::get_ui_node`). Unlike `UiNode::set_child`, each child added here stays an
//!   independent top-level node (not nested inside a rendered container) -
//!   `Container`'s padding/gap/orientation layout would otherwise unconditionally
//!   reposition it via the stacking cursor, clobbering whatever `.set_position(...)`
//!   it resolved to.
//! - `.build(app)` - resolves every child's pending `.set_size(...)`/
//!   `.set_position(...)` (and, recursively, their own children's) against the
//!   screen size (see `UiNode::resolve`), then registers them all into `app.ui`.

use crate::app::App;
use super::ui_node::UiNode;

pub struct Layer {
    width: f32,
    height: f32,
    children: Vec<(String, UiNode)>,
}

impl Layer {
    pub fn new(app: &App) -> Self {
        Self {
            width: app.window_manager.size.width as f32,
            height: app.window_manager.size.height as f32,
            children: Vec::new(),
        }
    }

    pub fn set_child(mut self, id: impl Into<String>, child: UiNode) -> Self {
        self.children.push((id.into(), child));
        self
    }

    pub fn build(self, app: &mut App) {
        let (width, height) = (self.width, self.height);
        for (id, mut node) in self.children {
            node.resolve(width, height);
            app.ui.add_to_ui(id, node);
        }
    }
}
