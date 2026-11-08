//! The physics, cameras and lighting a plane's `data.ron` describes - read
//! with serde, then turned into the engine's own `Physics` property
//! (`Physics::to_engine`).

use std::collections::HashMap;

use nalgebra::Vector3;
use pankarta_software::physics::{Collider, ColliderShape, Physics as EnginePhysics, RigidBodyData as EngineRigidBodyData};
use rapier3d::prelude::{ColliderSet, RigidBodyHandle, RigidBodySet};
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Lighting {
    pub intensity: f32,
    pub color: Vector3<f32>,
}

#[derive(Debug, Deserialize, Clone)]
pub enum ColliderType {
    Cuboid { half_extents: (f32, f32, f32), #[serde(default)] position: (f32, f32, f32) },
    Ball { radius: f32 },
    Cylinder { half_height: f32, radius: f32 },
    HeightField { heights: Vec<Vec<f32>>, scale_x: f32, scale_y: f32 },
    HalfSpace { normal: Vector3<f32> },
    // Its shape lifted straight from a mesh's own triangles instead of a
    // hand-authored primitive - a model file's path in the assets folder.
    TrimeshFromModel { model_path: String },
    // A mesh's triangles, already read. Meant for static bodies only
    // (Rapier's own trimesh shape isn't reliable for anything dynamic
    // sliding/rolling across sharp internal edges).
    Trimesh { vertices: Vec<Vector3<f32>>, indices: Vec<[u32; 3]> },
    // The smallest convex shape around `points` (in the body's frame) - a
    // shape that follows an outline more closely than a box, still cheap
    // and stable for a moving body. What a `Wing` resolves into. `wing` is
    // that surface's label, so a broken one can be told from the fuselage.
    ConvexHull { points: Vec<Vector3<f32>>, #[serde(default)] wing: Option<String> },
    // A plane's data.ron only: a thin slab following that wing's planform
    // (its aero `shape` - taper, sweep, span), `thickness` meters thick -
    // so the wing collides where it really is. Resolved into a `ConvexHull`
    // when the plane loads (see AircraftSpec::load).
    Wing { label: String, #[serde(default = "default_wing_thickness")] thickness: f32 },
}

fn default_wing_thickness() -> f32 {
    0.25
}

impl ColliderType {
    /// The engine's collider for this one - None for the shapes the engine
    /// doesn't build (an unresolved `Wing`, a height field).
    pub fn to_engine(&self) -> Option<Collider> {
        Some(match self {
            ColliderType::Cuboid { half_extents, position } => Collider::new(ColliderShape::Cuboid { half_extents: *half_extents })
                .offset(Vector3::new(position.0, position.1, position.2)),
            ColliderType::Ball { radius } => Collider::new(ColliderShape::Ball { radius: *radius }),
            ColliderType::Cylinder { half_height, radius } => Collider::new(ColliderShape::Cylinder { half_height: *half_height, radius: *radius }),
            ColliderType::HalfSpace { normal } => Collider::new(ColliderShape::HalfSpace { normal: *normal }),
            ColliderType::TrimeshFromModel { model_path } => Collider::new(ColliderShape::TrimeshFromModel { model_path: model_path.clone() }),
            ColliderType::Trimesh { vertices, indices } => Collider::new(ColliderShape::Trimesh { vertices: vertices.clone(), indices: indices.clone() }),
            ColliderType::ConvexHull { points, .. } => Collider::new(ColliderShape::ConvexHull { points: points.clone() }),
            ColliderType::HeightField { .. } | ColliderType::Wing { .. } => return None,
        })
    }

    /// The wing label of a resolved wing collider.
    pub fn wing(&self) -> Option<&str> {
        match self {
            ColliderType::ConvexHull { wing: Some(label), .. } => Some(label),
            _ => None,
        }
    }
}

/// Stable id stamped on a wing collider (`Collider::user_data`) so the
/// physics half can enable or disable that one surface. Never 0 - 0 is the
/// fuselage and anything that isn't a wing.
pub fn wing_collider_id(label: &str) -> u128 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in label.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let id = hash as u128;
    if id == 0 { 1 } else { id }
}

/// Stamps each wing collider of `body` with its `wing_collider_id` - the
/// engine's `Collider` has no user data of its own, so its colliders are
/// matched to `colliders` (the ones its `Physics` was built from) in order.
/// Only the ones the engine built count (see `ColliderType::to_engine`).
pub fn stamp_wing_colliders(body: RigidBodyHandle, bodies: &RigidBodySet, colliders: &mut ColliderSet, described: &[ColliderType]) {
    let Some(rigidbody) = bodies.get(body) else { return };
    let built = described.iter().filter(|collider| collider.to_engine().is_some());
    for (handle, collider) in rigidbody.colliders().iter().zip(built) {
        if let (Some(label), Some(built)) = (collider.wing(), colliders.get_mut(*handle)) {
            built.user_data = wing_collider_id(label);
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct RigidBodyData {
    pub is_static: bool,
    pub mass: f32,
    pub center_of_mass: Vector3<f32>,
    pub initial_velocity: Vector3<f32>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Physics {
    pub rigidbody: RigidBodyData,
    #[serde(default)]
    pub colliders: Vec<ColliderType>,
}

impl Physics {
    /// The engine's `Physics` property for this body.
    pub fn to_engine(&self) -> EnginePhysics {
        EnginePhysics {
            rigidbody: EngineRigidBodyData {
                is_static: self.rigidbody.is_static,
                mass: self.rigidbody.mass,
                center_of_mass: self.rigidbody.center_of_mass,
                initial_velocity: self.rigidbody.initial_velocity,
                ..EngineRigidBodyData::default()
            },
            colliders: self.colliders.iter().filter_map(ColliderType::to_engine).collect(),
        }
    }
}

fn default_camera_fov() -> f32 { 60.0 }

#[derive(Debug, Deserialize, Clone)]
pub struct Camera {
    pub position: Vector3<f32>,
    #[serde(default = "default_camera_fov")]
    pub fov: f32,
    /// How far a head at this camera can move from `position` - None: no
    /// limit. Used by the cockpit view (see `HeadLimits`).
    #[serde(default)]
    pub head_limits: Option<HeadLimits>,
}

/// The box a pilot's head stays inside, as offsets from the camera's
/// `position` (m, the owner's frame: +X left, +Y up, +Z forward) - everything
/// that moves the head (head tracking, G, looking over the shoulder) is
/// clamped into it, so the eye can't pass through the cockpit around it.
#[derive(Debug, Deserialize, Clone, Copy, PartialEq)]
pub struct HeadLimits {
    pub min: Vector3<f32>,
    pub max: Vector3<f32>,
}

impl HeadLimits {
    /// `offset` held inside the box, axis by axis.
    pub fn clamp(&self, offset: Vector3<f32>) -> Vector3<f32> {
        let (low, high) = (self.min.inf(&self.max), self.min.sup(&self.max));
        offset.sup(&low).inf(&high)
    }
}

pub type Cameras = HashMap<String, Camera>;

/// A rigid body's Cuboid colliders, treated as a mass-distributed box assembly, to
/// approximate its rotational inertia. Mass is distributed across colliders proportional
/// to their volume, and each box's own inertia plus its parallel-axis contribution
/// (from sitting off the body's center of mass) is summed. This makes inertia follow
/// whatever real-world dimensions and mass a plane is actually authored with, instead of
/// a single constant every dynamic body shared regardless of size — so a model built at
/// (1,1,1) and scaled up to its real-life size (colliders/positions authored to match)
/// gets physically appropriate rotational resistance automatically, with no per-plane
/// hand-tuning of inertia required.
pub fn compute_principal_inertia(mass: f32, center_of_mass: Vector3<f32>, colliders: &[ColliderType]) -> Vector3<f32> {
    struct BoxPart {
        volume: f32,
        half_extents: (f32, f32, f32),
        position: (f32, f32, f32),
    }

    let boxes: Vec<BoxPart> = colliders.iter().filter_map(|collider| match collider {
        ColliderType::Cuboid { half_extents, position } => Some(BoxPart {
            volume: 8.0 * half_extents.0 * half_extents.1 * half_extents.2,
            half_extents: *half_extents,
            position: *position,
        }),
        // Counted as the box around it - close enough for a thin wing slab.
        ColliderType::ConvexHull { points, .. } if !points.is_empty() => {
            let (low, high) = points.iter().fold((points[0], points[0]), |(low, high), point| (low.inf(point), high.sup(point)));
            let half = (high - low) * 0.5;
            let center = (high + low) * 0.5;
            Some(BoxPart { volume: 8.0 * half.x * half.y * half.z, half_extents: (half.x, half.y, half.z), position: (center.x, center.y, center.z) })
        }
        _ => None,
    }).collect();

    let total_volume: f32 = boxes.iter().map(|b| b.volume).sum();
    if total_volume <= 0.0 {
        // No box geometry to derive inertia from — fall back to something proportional
        // to mass rather than a fixed constant, so it's at least in the right ballpark.
        let fallback = mass * 2.0;
        return Vector3::new(fallback, fallback, fallback);
    }

    let mut inertia = Vector3::new(0.0f32, 0.0, 0.0);
    for b in &boxes {
        let part_mass = mass * (b.volume / total_volume);
        let (hx, hy, hz) = b.half_extents;

        // Inertia of a solid box of this part's mass about its own center.
        let own_ixx = (part_mass / 3.0) * (hy * hy + hz * hz);
        let own_iyy = (part_mass / 3.0) * (hx * hx + hz * hz);
        let own_izz = (part_mass / 3.0) * (hx * hx + hy * hy);

        // Parallel-axis theorem: account for this part sitting away from the body's COM.
        let offset = Vector3::new(b.position.0, b.position.1, b.position.2) - center_of_mass;
        let (ox, oy, oz) = (offset.x, offset.y, offset.z);

        inertia.x += own_ixx + part_mass * (oy * oy + oz * oz);
        inertia.y += own_iyy + part_mass * (ox * ox + oz * oz);
        inertia.z += own_izz + part_mass * (ox * ox + oy * oy);
    }

    inertia
}
