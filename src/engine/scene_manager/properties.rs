use nalgebra::{UnitQuaternion, Vector3};

use crate::resources::{DoubleSidedMeshes, ModelSource};

/// A `Node` property giving it a 3D position/rotation/scale.
///
/// Deliberately independent of `crate::transform::Transform` and
/// `engine::game_nodes::game_object::Transform` (both tied to the existing
/// `data.ron`-loaded object pipeline, the second via a custom `Deserialize`
/// impl converting Euler angles to a quaternion) - this property system is
/// code-first by design and doesn't go through either's deserialization path.
#[derive(Debug, Clone, Copy)]
pub struct Transform3D {
    pub position: Vector3<f32>,
    pub rotation: UnitQuaternion<f32>,
    pub scale: Vector3<f32>,
}

impl Default for Transform3D {
    fn default() -> Self {
        Transform3D {
            position: Vector3::zeros(),
            rotation: UnitQuaternion::identity(),
            scale: Vector3::new(1.0, 1.0, 1.0),
        }
    }
}

impl Transform3D {
    /// Where a child sitting at `local` from this transform (its parent) is
    /// in the world - see `Parent`. The offset is in meters, turned with the
    /// parent when `inherit_rotation` (otherwise along the world's axes, and
    /// the child keeps its own rotation as is); the parent's scale touches
    /// neither, and the child keeps its own scale.
    pub fn compose(&self, local: &Transform3D, inherit_rotation: bool) -> Transform3D {
        if inherit_rotation {
            Transform3D {
                position: self.position + self.rotation * local.position,
                rotation: self.rotation * local.rotation,
                scale: local.scale,
            }
        } else {
            Transform3D { position: self.position + local.position, rotation: local.rotation, scale: local.scale }
        }
    }

    /// The reverse of `compose`: what `local` puts a child at `world`, from
    /// this transform.
    pub fn relative(&self, world: &Transform3D, inherit_rotation: bool) -> Transform3D {
        if inherit_rotation {
            let inverse = self.rotation.inverse();
            Transform3D {
                position: inverse * (world.position - self.position),
                rotation: inverse * world.rotation,
                scale: world.scale,
            }
        } else {
            Transform3D { position: world.position - self.position, rotation: world.rotation, scale: world.scale }
        }
    }
}

/// Degrees about X, Y and Z (the map file's convention) to a rotation.
pub fn rotation_from_degrees(degrees: Vector3<f32>) -> UnitQuaternion<f32> {
    UnitQuaternion::from_euler_angles(degrees.x.to_radians(), degrees.y.to_radians(), degrees.z.to_radians())
}

/// The reverse of `rotation_from_degrees`.
pub fn degrees_from_rotation(rotation: &UnitQuaternion<f32>) -> Vector3<f32> {
    let (x, y, z) = rotation.euler_angles();
    Vector3::new(x.to_degrees(), y.to_degrees(), z.to_degrees())
}

/// How a child node's position is given - see `Parent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Space {
    /// An offset from the parent's origin, in its frame: the child moves
    /// (and, unless told otherwise, turns) with it.
    #[default]
    Local,
    /// A place in the world, disconnected from the parent: the child stays
    /// where it is whatever the parent does. It's still the parent's child
    /// in the hierarchy, and goes when the parent does.
    Global,
}

/// A node's place under another node - put on for you by `Node::with_child`
/// (the parent's id, the space and whether it turns with the parent come
/// from there), so it's rarely built by hand. Nodes are still kept flat by
/// id (see `SceneNodes`); this is what makes one a child.
///
/// The node's own `Transform3D` is always where it is in the world - every
/// system (renderer, particles, cameras) reads that. For a `Local` child,
/// `local` is the offset from the parent, and `SceneNodes::resolve_hierarchy`
/// writes the world transform from it every frame; for a `Global` child,
/// `local` is unused and its `Transform3D` is the one to move.
///
/// A child with a `Physics` body isn't moved by its parent - the physics
/// does that.
#[derive(Debug, Clone)]
pub struct Parent {
    pub id: String,
    pub space: Space,
    pub local: Transform3D,
    /// A `Local` child turns with the parent - off: it only follows its
    /// position, keeping its own rotation (a chase camera that shouldn't
    /// roll with the plane).
    pub inherit_rotation: bool,
}

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

/// Keeps a node turned to face another node (by id) - its +Z at it, +Y up -
/// every frame, after the hierarchy's placed both (see
/// `SceneNodes::resolve_hierarchy`). Most useful on a camera.
#[derive(Debug, Clone)]
pub struct LookAt {
    pub target: String,
}

/// Marks a node spawned while the scene was already running (a missile, a
/// flare, a respawned aircraft) rather than while it was being built - see
/// `Scene::spawn_node`. The editor's hierarchy hides these unless asked.
#[derive(Debug, Clone, Copy)]
pub struct SpawnedLive;

/// A `Node` property pointing at a loaded model by name - matches the
/// `model_ref` keys already used in `App::game_models`
/// (`HashMap<String, ModelDataInstance>`) - plus the model's own `scale`:
/// how big the model file is drawn, separate from the node's own
/// `Transform3D::scale`. The drawn size is node scale x model scale, so a
/// model authored at 1 unit long can be `Model::new("F16").scaled(14.0)` on
/// a node whose own scale stays 1 (see `Model::render_scale`). Build it with
/// `Model::new` (a model named with
/// `resources::declare_model`) or `Model::from_file` (a file on disk) -
/// either way it's loaded the first time a node uses it, not before (see
/// `resources::load_scene_model`).
///
/// Actually connecting this to the renderer - registering a live instance and
/// writing per-frame instance buffers the way `App::renderizable_instances`
/// does for `data.ron`-loaded objects today - is deliberately left as
/// follow-up work, not part of this pass.
#[derive(Debug, Clone)]
pub struct Model {
    pub model_ref: String,
    pub scale: Vector3<f32>,
    /// The file to load it from, for `Model::from_file` - None for a model
    /// named with `resources::declare_model` (or a procedural one).
    pub source: Option<ModelSource>,
}

impl Model {
    /// `model_ref` at its own size (scale 1).
    pub fn new(model_ref: impl Into<String>) -> Self {
        Self { model_ref: model_ref.into(), scale: Vector3::new(1.0, 1.0, 1.0), source: None }
    }

    /// The model in the file at `path` (a `.glb`/`.gltf` anywhere on disk,
    /// e.g. a plane's own folder) - loaded the first time a node uses it,
    /// shared by every node using the same file.
    pub fn from_file(path: impl Into<std::path::PathBuf>) -> Self {
        Self::from_source(ModelSource::asset(path))
    }

    /// The model `source` describes.
    pub fn from_source(source: ModelSource) -> Self {
        Self { model_ref: source.key(), scale: Vector3::new(1.0, 1.0, 1.0), source: Some(source) }
    }

    /// Which meshes skip backface culling - only meaningful for
    /// `from_file`/`from_source` models (a declared one sets its own).
    pub fn double_sided(mut self, double_sided: DoubleSidedMeshes) -> Self {
        if let Some(source) = self.source.take() {
            self.source = Some(source.double_sided(double_sided));
        }
        self
    }

    /// A separate copy of the model, `name`d - its meshes animated apart
    /// from the shared copy's (see `ModelSource::variant`). Only for
    /// `from_file`/`from_source` models.
    pub fn variant(mut self, name: impl Into<String>) -> Self {
        if let Some(source) = self.source.take() {
            let source = source.variant(name);
            self.model_ref = source.key();
            self.source = Some(source);
        }
        self
    }

    /// Drawn `scale` times its file size, the same on every axis.
    pub fn scaled(self, scale: f32) -> Self {
        self.with_scale(Vector3::new(scale, scale, scale))
    }

    /// Drawn `scale` times its file size, per axis.
    pub fn with_scale(mut self, scale: Vector3<f32>) -> Self {
        self.scale = scale;
        self
    }

    /// The scale the model is actually drawn at on a node scaled
    /// `node_scale`: the two multiplied, per axis.
    pub fn render_scale(&self, node_scale: Vector3<f32>) -> Vector3<f32> {
        node_scale.component_mul(&self.scale)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parent() -> Transform3D {
        Transform3D { position: Vector3::new(100.0, 50.0, -20.0), rotation: UnitQuaternion::from_euler_angles(0.2, 1.1, -0.4), scale: Vector3::new(3.0, 3.0, 3.0) }
    }

    #[test]
    fn a_local_offset_turns_with_the_parent_and_ignores_its_scale() {
        let local = Transform3D { position: Vector3::new(0.0, 0.0, -10.0), ..Default::default() };
        let world = parent().compose(&local, true);
        assert!(((world.position - parent().position).magnitude() - 10.0).abs() < 1e-4);
        assert!((world.position - (parent().position + parent().rotation * local.position)).magnitude() < 1e-4);
    }

    #[test]
    fn relative_undoes_compose() {
        let world = Transform3D { position: Vector3::new(7.0, -3.0, 12.0), rotation: UnitQuaternion::from_euler_angles(-0.5, 0.3, 2.0), scale: Vector3::new(1.0, 1.0, 1.0) };
        for inherit_rotation in [true, false] {
            let local = parent().relative(&world, inherit_rotation);
            let back = parent().compose(&local, inherit_rotation);
            assert!((back.position - world.position).magnitude() < 1e-3, "{inherit_rotation}");
            assert!(back.rotation.angle_to(&world.rotation) < 1e-4, "{inherit_rotation}");
        }
    }
}
