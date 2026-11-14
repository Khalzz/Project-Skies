//! The sandbox's UI - top left: the frame rate and where the camera is.
//! A node of its own (`node`), its `SandboxUi` behavior filling the text in
//! every frame.

use pankarta_software::prelude::*;
use pankarta_software::ui::ui_node::UiNode;
use pankarta_software::ui::ui_transform::SizeValue;
use pankarta_software::ui::widgets::text;
use pankarta_software::ui::TextStyle;

/// The UI's node, showing where the camera `camera` (a node's id) is.
pub fn node(camera: &str) -> Node {
    let line = |placeholder: &str| text(placeholder, &TextStyle::default()).set_size(SizeValue::Pixels(400.0), SizeValue::Fit);
    let root = UiNode::container()
        .set_padding((10.0, 10.0))
        .set_child("framerate", line("- fps"))
        .set_child("camera_position", line("x - y - z -"));
    Node::new("UI")
        .with_property(UiLayer::new(root))
        .with_behavior(SandboxUi { camera: camera.to_owned() })
}

/// Fills the UI's text in, every frame.
pub struct SandboxUi {
    /// The camera whose position it shows, by id.
    camera: String,
}

impl Behavior for SandboxUi {
    fn update(&mut self, node: &mut Node, scene: &mut SceneAccess, app: &mut App) {
        let position = scene
            .get_node(&self.camera)
            .and_then(|camera| camera.get_property::<Transform3D>())
            .map(|transform| transform.position);
        let Some(ui) = node.get_property_mut::<UiLayer>() else { return };
        if let Some(label) = ui.label_mut("framerate") {
            label.set_text(format!("{} fps", app.time.fps().round()));
        }
        if let (Some(label), Some(position)) = (ui.label_mut("camera_position"), position) {
            label.set_text(format!("x {:.1}  y {:.1}  z {:.1}", position.x, position.y, position.z));
        }
    }
}
