//! The game's screen UI as engine `UiLayer`s.
//!
//! The game's own engine kept one global UI: top-level nodes by key
//! ("game_ui", "pause_menu", "subtitles"...), each placed on its own and
//! reached by path ("game_ui/hud_stores/weapons"). The engine has no global
//! UI for a game - UI is a node's `UiLayer`, living and dying with its
//! scene. So every one of those top-level keys is a node of its own,
//! `ui:<key>`, whose layer's root is that UI tree, and a path's first part
//! picks the layer:
//!
//!     ui_layers::add(scene, app, "pause_menu", menu);
//!     if let Some(title) = scene.ui_node("pause_menu/title") { .. }
//!
//! Layers stack by z index: `ON_TOP` for what the old UI drew always on top
//! (menus, the mission intro), `ON_BOTTOM` for what it drew under the rest.

use pankarta_software::prelude::*;
use pankarta_software::scene_manager::scene_access::SceneAccess;
use pankarta_software::ui::ui_node::UiNode;

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
