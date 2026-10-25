/// Makes a node a camera - registered with the scene's cameras under the
/// node's id when it spawns, and kept where the node is every frame (see
/// `Scene::sync_cameras`). A camera looks along its node's +Z, its +Y up
/// (the same way a model faces) - so a camera child of a plane with no
/// rotation looks where the plane's nose points.
#[derive(Debug, Clone, Copy)]
pub struct CameraLens {
    /// Vertical field of view, degrees.
    pub fovy: f32,
    pub znear: f32,
    /// The size it's drawn at (logical pixels), when it isn't the whole
    /// screen - F10's map draws in only part of it.
    pub viewport: Option<(u32, u32)>,
}

impl CameraLens {
    pub fn new(fovy: f32) -> Self {
        Self { fovy, znear: 0.1, viewport: None }
    }
}

impl Default for CameraLens {
    fn default() -> Self {
        Self::new(60.0)
    }
}
