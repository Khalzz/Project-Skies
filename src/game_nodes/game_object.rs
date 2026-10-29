// the entity is the basic object on this "game engine project", it will have the values needed for our "GameObject2Ds"
// the way we "render our objects its based on our object itself" so i will save that "render value" for later

use nalgebra::{Vector3, Matrix3, Matrix4, UnitQuaternion};
use serde::{Deserialize, Deserializer};

use crate::rendering::instance_management::InstanceRaw;

// Transform struct
#[derive(Debug, Clone, Copy)]
pub struct Transform {
    pub position: Vector3<f32>,
    pub rotation: UnitQuaternion<f32>, // Use UnitQuaternion for normalized rotations
    pub scale: Vector3<f32>,
}

/// One collider on a body: its `shape`, where it sits on the body, and how
/// it touches things - see `Collider::new` and its builder methods.
#[derive(Debug, Deserialize, Clone)]
pub struct Collider {
    pub shape: ColliderShape,
    /// Where its center is, from the body's - in the body's frame.
    #[serde(default)]
    pub offset: Vector3<f32>,
    /// How much it grips what it slides on: 0 none, 1 a lot. None: Rapier's
    /// default (0.5).
    #[serde(default)]
    pub friction: Option<f32>,
    /// How much it bounces back: 0 not at all, 1 everything. None: Rapier's
    /// default (0).
    #[serde(default)]
    pub restitution: Option<f32>,
    /// Detects what overlaps it without stopping anything - a trigger zone.
    #[serde(default)]
    pub sensor: bool,
}

impl Collider {
    /// A collider of `shape` at the body's center, touching things the
    /// default way.
    pub fn new(shape: ColliderShape) -> Self {
        Self { shape, offset: Vector3::zeros(), friction: None, restitution: None, sensor: false }
    }

    /// Placed `offset` from the body's center (in the body's frame).
    pub fn offset(mut self, offset: Vector3<f32>) -> Self {
        self.offset = offset;
        self
    }

    /// Grips `friction` - it wins over what it touches when it's the lower
    /// of the two, so 0 always slides.
    pub fn friction(mut self, friction: f32) -> Self {
        self.friction = Some(friction);
        self
    }

    /// Bounces back `restitution` of its speed - it wins over what it
    /// touches when it's the higher of the two.
    pub fn restitution(mut self, restitution: f32) -> Self {
        self.restitution = Some(restitution);
        self
    }

    /// A trigger: detects overlaps, stops nothing.
    pub fn sensor(mut self) -> Self {
        self.sensor = true;
        self
    }
}

impl From<ColliderShape> for Collider {
    fn from(shape: ColliderShape) -> Self {
        Self::new(shape)
    }
}

/// A collider's shape - centered on where its `Collider` is.
#[derive(Debug, Deserialize, Clone)]
pub enum ColliderShape {
    Cuboid { half_extents: (f32, f32, f32) },
    Ball { radius: f32 },
    // A standing capsule, centered on the body: a cylinder `half_height`
    // above and below the center, capped by half-balls of `radius` - so
    // `half_height + radius` from the center to each end. The usual shape for
    // a character: its round bottom rides over small steps and edges instead
    // of catching on them.
    Capsule { half_height: f32, radius: f32 },
    Cylinder { half_height: f32, radius: f32 },
    HeightField { heights: Vec<Vec<f32>>, scale_x: f32, scale_y: f32 },
    HalfSpace { normal: Vector3<f32> },
    // What a node actually declares when it wants its collision shape lifted
    // straight from a mesh's own triangles instead of a hand-authored
    // primitive - just a reference, the same way `Model { model_ref }` is a
    // name rather than the model's own loaded data. Resolved into a real
    // `Trimesh` (below) by `engine::scene_manager::physics_bridge` at spawn
    // time, scaled by the node's own `Transform3D::scale` - a body is never
    // made from this variant, only from the resolved one.
    TrimeshFromModel { model_path: String },
    // The resolved form of `TrimeshFromModel` - what a collider is
    // actually built from (see `load_physics_from_definitions`).
    // Meant for static bodies only (Rapier's own trimesh shape isn't
    // reliable for anything dynamic sliding/rolling across sharp internal
    // edges) - a good fit for irregular, concave static terrain (mountains,
    // ...) that no primitive shape approximates well.
    Trimesh { vertices: Vec<Vector3<f32>>, indices: Vec<[u32; 3]> },
    // The smallest convex shape around `points` (in the body's frame) - a
    // shape that follows an outline more closely than a box, still cheap
    // and stable for a moving body.
    ConvexHull { points: Vec<Vector3<f32>> },
}

#[derive(Debug, Deserialize, Clone)]
pub struct RigidBodyData {
    pub is_static: bool,
    pub mass: f32,
    pub center_of_mass: Vector3<f32>,
    pub initial_velocity: Vector3<f32>,
    /// Never turns, whatever pushes it - a character standing upright.
    #[serde(default)]
    pub lock_rotations: bool,
    /// Continuous collision detection: checks the path it moves along
    /// between steps too, so moving fast it can't pass through something
    /// thin. Costs a little more each step.
    #[serde(default)]
    pub ccd: bool,
    /// How much of the world's gravity (`app.physics.gravity()`) it feels:
    /// 1 all of it, 2 twice, 0 none (it floats), negative pulls it up.
    /// Follows the world's gravity when that changes. Changed while it runs
    /// with `app.physics.set_gravity_scale`.
    #[serde(default = "full_gravity")]
    pub gravity_scale: f32,
}

fn full_gravity() -> f32 {
    1.0
}

impl Default for RigidBodyData {
    /// A moving body of 1 kg, at rest, free to turn.
    fn default() -> Self {
        Self { is_static: false, mass: 1.0, center_of_mass: Vector3::zeros(), initial_velocity: Vector3::zeros(), lock_rotations: false, ccd: false, gravity_scale: 1.0 }
    }
}

impl RigidBodyData {
    /// A body that never moves - the ground, walls.
    pub fn fixed() -> Self {
        Self { is_static: true, mass: 0.0, ..Default::default() }
    }

    /// A moving body of `mass` kilograms.
    pub fn dynamic(mass: f32) -> Self {
        Self { mass, ..Default::default() }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct Physics {
    pub rigidbody: RigidBodyData,
    #[serde(default)]
    pub colliders: Vec<Collider>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct GameObject {
    pub id: String,
    pub model: String,
    pub transform: Transform,
    pub children: Vec<GameObject>,
}

#[derive(Debug, Deserialize, Clone, Copy)]
pub struct RawTransform {
    pub position: Vector3<f32>,
    pub rotation: Vector3<f32>,
    pub scale: Vector3<f32>,
}

// Implementing `to_raw` to convert Transform into InstanceRaw
impl Transform {
    // camera_position makes the model matrix camera-relative: subtracting it here,
    // once, on the CPU keeps the numbers the GPU multiplies small no matter how far
    // the object is from the world origin, avoiding the precision loss (z-fighting,
    // vertex jitter) that baking absolute world coordinates into the matrix causes.
    pub fn to_raw(&self, camera_position: Vector3<f32>) -> InstanceRaw {
        let translation = Matrix4::new_translation(&(self.position - camera_position));
        let rotation = self.rotation.to_homogeneous();
        let scale = Matrix4::new_nonuniform_scaling(&self.scale);
        let model = translation * rotation * scale;

        InstanceRaw {
            model: model.into(),
            normal: Matrix3::from(self.rotation).into(),
        }
    }

    // Function to create Transform from RawTransform, converting Euler angles to quaternion
    fn from_raw(raw: RawTransform) -> Self {
        let rotation = UnitQuaternion::from_euler_angles(
            raw.rotation.x.to_radians(),
            raw.rotation.y.to_radians(),
            raw.rotation.z.to_radians(),
        );

        Transform {
            position: raw.position,
            rotation,
            scale: raw.scale,
        }
    }
}

// Custom deserialization for Transform
impl<'de> Deserialize<'de> for Transform {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw_transform = RawTransform::deserialize(deserializer)?;
        Ok(Transform::from_raw(raw_transform))
    }
}