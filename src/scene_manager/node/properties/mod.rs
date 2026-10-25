//! The engine's own `Node` properties - one file each. Everything is
//! re-exported here, so `node::properties::Material` etc. work as before.

pub mod camera_lens;
pub mod look_at;
pub mod material;
pub mod model;
pub mod offset_2d;
pub mod parent;
pub mod shape_2d;
pub mod spawned_live;
pub mod transform;

pub use camera_lens::CameraLens;
pub use look_at::LookAt;
pub use material::Material;
pub use model::Model;
pub use offset_2d::Offset2D;
pub use parent::{Parent, Space};
pub use shape_2d::{Shape2D, ShapeKind};
pub use spawned_live::SpawnedLive;
pub use transform::{degrees_from_rotation, rotation_from_degrees, Transform2D, Transform3D};
