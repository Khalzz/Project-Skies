/// Keeps a node turned to face another node (by id) - its +Z at it, +Y up -
/// every frame, after the hierarchy's placed both (see
/// `SceneNodes::resolve_hierarchy`). Most useful on a camera.
#[derive(Debug, Clone)]
pub struct LookAt {
    pub target: String,
}
