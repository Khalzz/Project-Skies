/// A UI node that displays a loaded texture. Deliberately thin - the actual GPU
/// texture lives in Ui::images, cached by path (see Ui::load_image), not here; this
/// just remembers which cached image this node wants drawn each frame. Alpha lives
/// in the owning UiNode's Style instead (see UiNode.style/hover), resolved fresh
/// each frame in node_content_preparation - same reasoning as Label not storing its
/// own color/font_size.
pub struct ImageNode {
    pub path: String,
}

impl ImageNode {
    pub fn new(path: String) -> Self {
        Self { path }
    }
}
