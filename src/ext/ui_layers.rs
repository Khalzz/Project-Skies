//! The game's screen UI as engine `UiLayer`s.
//!
//! The engine's way: a piece of UI is a node with a `UiLayer` property,
//! living and dying with it. The game makes one node per UI *element* - the
//! flight HUD, the pause menu, the mission intro... - `ui:<key>`, whose
//! layer holds that element's whole tree. A path's first part picks the
//! element, the rest is the path inside its layer:
//!
//!     ui_layers::add(scene, app, "hud", hud);
//!     if let Some(timer) = scene.ui_node("hud/game_ui/data_box/timer") { .. }
//!
//! A part of an element that moves every frame (a HUD readout following
//! the jet) is a child its parent places - see `place`.
//!
//! Layers stack by z index: `ON_TOP` for menus and the mission intro,
//! `ON_BOTTOM` for what's drawn under the rest.

use pankarta_software::prelude::*;
use pankarta_software::scene_manager::scene_access::SceneAccess;
use pankarta_software::ui::ui_node::UiNode;
use pankarta_software::ui::ui_transform::{PositionValue, SizeValue};

/// The z index of a layer the old UI drew always on top.
pub const ON_TOP: i32 = 100;
/// The z index of a layer the old UI drew under everything else.
pub const ON_BOTTOM: i32 = -100;

/// The id of the node holding the top-level UI `key`.
pub fn node_id(key: &str) -> String {
    format!("ui:{key}")
}

/// The node holding `root` as the top-level UI `key`.
pub fn layer_node(key: &str, root: UiNode) -> Node {
    Node::new(node_id(key)).with_property(UiLayer::new(root))
}

/// `path` split into its layer's key and the path inside the layer.
fn split(path: &str) -> (&str, &str) {
    match path.split_once('/') {
        Some((key, rest)) => (key, rest),
        None => (path, ""),
    }
}

/// UI lookups by path on whatever holds the scene's nodes - the `Scene`
/// itself, or a behavior's `SceneAccess`.
pub trait GameUi {
    fn node_mut_by_id(&mut self, id: &str) -> Option<&mut Node>;
    fn node_by_id(&self, id: &str) -> Option<&Node>;

    /// The layer of the top-level UI `key`.
    fn ui_layer(&mut self, key: &str) -> Option<&mut UiLayer> {
        self.node_mut_by_id(&node_id(key))?.get_property_mut::<UiLayer>()
    }

    /// The UI node at `path` - "key" for a top-level one's root,
    /// "key/child/..." inside it.
    fn ui_node(&mut self, path: &str) -> Option<&mut UiNode> {
        let (key, rest) = split(path);
        self.ui_layer(key)?.get_mut(rest)
    }

    /// The UI node at `path`, to read.
    fn ui_node_ref(&self, path: &str) -> Option<&UiNode> {
        let (key, rest) = split(path);
        self.node_by_id(&node_id(key))?.get_property::<UiLayer>()?.get(rest)
    }

    /// Whether there's a top-level UI `key`.
    fn has_ui(&self, key: &str) -> bool {
        self.node_by_id(&node_id(key)).is_some()
    }

    /// Draws the top-level UI `key` over everything else, keeping clicks
    /// from what's under it while it's shown - a modal.
    fn ui_on_top(&mut self, key: impl AsRef<str>) {
        if let Some(layer) = self.ui_layer(key.as_ref()) {
            layer.set_z_index(ON_TOP);
        }
    }

    /// Draws the top-level UI `key` under everything else.
    fn ui_on_bottom(&mut self, key: impl AsRef<str>) {
        if let Some(layer) = self.ui_layer(key.as_ref()) {
            layer.set_z_index(ON_BOTTOM);
        }
    }

    /// Shows or hides the top-level UI `key` - the root's `is_active`.
    fn set_ui_active(&mut self, key: &str, active: bool) {
        if let Some(root) = self.ui_node(key) {
            root.set_active(active);
        }
    }

    /// Shows or hides the whole element `key` - its layer, whatever its
    /// nodes' own `is_active` (which tracks and the game keep setting).
    fn set_ui_visible(&mut self, key: &str, visible: bool) {
        if let Some(layer) = self.ui_layer(key) {
            layer.set_visible(visible);
        }
    }
}

/// Puts `node` - a child in an element's layer - `width` x `height` at
/// (`x`, `y`) from its parent's top left (the screen's, for a child of a
/// full-screen root), from this frame's layout on. A parent lays its
/// children out every frame, so a child's rect written by hand is undone;
/// its position (`set_position`) is what the layout keeps.
pub fn place(node: &mut UiNode, x: f32, y: f32, width: f32, height: f32) {
    node.transform.width = width;
    node.transform.height = height;
    // set_position is a builder - the node is taken out for it and put back.
    let taken = std::mem::replace(node, UiNode::container());
    *node = taken.set_position(PositionValue::Start(x), PositionValue::Start(y));
    // Its rect right away too, for what reads it before the layout runs.
    node.move_to(x, y);
}

/// A full-screen container at the screen's top left - the root of an
/// element whose parts are placed on their own (see `place`).
pub fn screen_root() -> UiNode {
    UiNode::container()
        .set_size(SizeValue::Percent(100.0), SizeValue::Percent(100.0))
        .set_position(PositionValue::Start(0.0), PositionValue::Start(0.0))
        .set_background_color(UiColor::TRANSPARENT)
}

impl GameUi for Scene {
    fn node_mut_by_id(&mut self, id: &str) -> Option<&mut Node> {
        self.get_node_mut(id)
    }

    fn node_by_id(&self, id: &str) -> Option<&Node> {
        self.get_node(id)
    }
}

impl GameUi for SceneAccess<'_> {
    fn node_mut_by_id(&mut self, id: &str) -> Option<&mut Node> {
        self.get_node_mut(id)
    }

    fn node_by_id(&self, id: &str) -> Option<&Node> {
        self.get_node(id)
    }
}

/// Puts `root` up as the top-level UI `key` (replacing one already there).
pub fn add(scene: &mut Scene, app: &mut App, key: impl AsRef<str>, root: UiNode) {
    let key = key.as_ref();
    if scene.has_ui(key) {
        scene.despawn_node(app, &node_id(key));
    }
    if let Err(error) = scene.spawn_node(app, layer_node(key, root)) {
        eprintln!("couldn't add UI '{key}': {error}");
    }
}

/// `add`, from a behavior - done once every behavior has run this frame.
pub fn add_from_behavior(scene: &mut SceneAccess, key: &str, root: UiNode) {
    scene.despawn(node_id(key));
    scene.spawn(layer_node(key, root));
}

/// Takes the top-level UI `key` down.
pub fn remove(scene: &mut Scene, app: &mut App, key: impl AsRef<str>) {
    scene.despawn_node(app, &node_id(key.as_ref()));
}

/// What a UI click handler reaches the UI through: the node whose layer was
/// clicked (taken out of the scene while its handler runs - see the
/// engine's `UiNode::on_click`) and the rest of the scene, as one.
pub struct ClickUi<'a, 'b> {
    pub node: &'a mut Node,
    pub scene: &'a mut SceneAccess<'b>,
}

impl GameUi for ClickUi<'_, '_> {
    fn node_mut_by_id(&mut self, id: &str) -> Option<&mut Node> {
        if self.node.id == id {
            Some(self.node)
        } else {
            self.scene.get_node_mut(id)
        }
    }

    fn node_by_id(&self, id: &str) -> Option<&Node> {
        if self.node.id == id {
            Some(self.node)
        } else {
            self.scene.get_node(id)
        }
    }
}

/// A click handler written against `ClickUi` - what the engine's
/// `UiNode::on_click` takes.
pub fn click(mut handler: impl FnMut(&mut ClickUi, &mut App) + 'static) -> impl FnMut(&mut Node, &mut SceneAccess, &mut App) + 'static {
    move |node, scene, app| handler(&mut ClickUi { node, scene }, app)
}
