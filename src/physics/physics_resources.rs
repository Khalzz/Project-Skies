use std::collections::HashMap;
use nalgebra::{vector, Point3, Unit, UnitQuaternion, Vector3};
use rapier3d::prelude::*;

use crate::game_nodes::game_object;

/// A body loaded from a `PhysicsObjectDef`, and its colliders.
pub struct PhysicsData {
    pub rigidbody_handle: RigidBodyHandle,
    pub collider_handles: Vec<ColliderHandle>,
}



/// A rigid body's Cuboid colliders, treated as a mass-distributed box assembly, to
/// approximate its rotational inertia. Mass is distributed across colliders proportional
/// to their volume, and each box's own inertia plus its parallel-axis contribution
/// (from sitting off the body's center of mass) is summed. This makes inertia follow
/// whatever real-world dimensions and mass a plane is actually authored with, instead of
/// a single constant every dynamic body shared regardless of size — so a model built at
/// (1,1,1) and scaled up to its real-life size (colliders/positions authored to match)
/// gets physically appropriate rotational resistance automatically, with no per-plane
/// hand-tuning of inertia required.
pub(crate) fn compute_principal_inertia(mass: f32, center_of_mass: Vector3<f32>, colliders: &[game_object::Collider]) -> Vector3<f32> {
    struct BoxPart {
        volume: f32,
        half_extents: (f32, f32, f32),
        position: (f32, f32, f32),
    }

    let boxes: Vec<BoxPart> = colliders.iter().filter_map(|collider| {
        let offset = collider.offset;
        match &collider.shape {
            game_object::ColliderShape::Cuboid { half_extents } => Some(BoxPart {
                volume: 8.0 * half_extents.0 * half_extents.1 * half_extents.2,
                half_extents: *half_extents,
                position: (offset.x, offset.y, offset.z),
            }),
            // Counted as the box around it.
            game_object::ColliderShape::ConvexHull { points } if !points.is_empty() => {
                let (low, high) = points.iter().fold((points[0], points[0]), |(low, high), point| (low.inf(point), high.sup(point)));
                let half = (high - low) * 0.5;
                let center = (high + low) * 0.5 + offset;
                Some(BoxPart { volume: 8.0 * half.x * half.y * half.z, half_extents: (half.x, half.y, half.z), position: (center.x, center.y, center.z) })
            }
            _ => None,
        }
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

/// A node's `Physics` property (see `engine::scene_manager::physics_bridge`)
/// plus the position and rotation it was spawned at - what its body is made
/// from, extracted once at spawn time the same way
/// `render_bridge::register_static_model` extracts `Transform3D`+`Model` into
/// a plain `GameObject` for rendering. Collected on
/// `SceneContent::physics_bodies` while the scene is built, and made into
/// bodies when its physics starts (see `PhysicsWorld::start`) - a node
/// spawned after that gets its body right away.
#[derive(Clone)]
pub struct PhysicsObjectDef {
    pub id: String,
    pub position: Vector3<f32>,
    pub rotation: UnitQuaternion<f32>,
    pub physics: game_object::Physics,
}

/// Seeds `collider_set`/`rigidbody_set` from the scene's `PhysicsObjectDef`s
/// (see that type's own doc comment) - one body each, with its colliders.
pub fn load_physics_from_definitions(defs: &[PhysicsObjectDef], collider_set: &mut ColliderSet, rigidbody_set: &mut RigidBodySet, physics_handlers: &mut HashMap<String, Option<PhysicsData>>) {
    for def in defs {
        let mut rigid_body = if def.physics.rigidbody.is_static {
            RigidBodyBuilder::fixed()
                .additional_mass(def.physics.rigidbody.mass)
                .translation(vector![def.position.x, def.position.y, def.position.z])
                .build()
        } else {
            let principal_inertia = compute_principal_inertia(
                def.physics.rigidbody.mass,
                def.physics.rigidbody.center_of_mass,
                &def.physics.colliders,
            );

            let mut builder = RigidBodyBuilder::dynamic()
                .additional_mass_properties(rapier3d::prelude::MassProperties::new(def.physics.rigidbody.center_of_mass.into(), def.physics.rigidbody.mass, principal_inertia))
                .translation(def.position)
                .angular_damping(2.0)
                .ccd_enabled(def.physics.rigidbody.ccd)
                .gravity_scale(def.physics.rigidbody.gravity_scale);
            if def.physics.rigidbody.lock_rotations {
                builder = builder.lock_rotations();
            }
            builder.build()
        };

        rigid_body.set_rotation(def.rotation, true);
        rigid_body.set_linvel(def.physics.rigidbody.initial_velocity, true);
        let rigidbody_handle = rigidbody_set.insert(rigid_body);

        let mut collider_handles: Vec<ColliderHandle> = Vec::new();
        for collider_data in &def.physics.colliders {
            let Some(builder) = collider_builder(&collider_data.shape, &def.id) else { continue };
            let mut builder = builder
                .translation(collider_data.offset)
                .sensor(collider_data.sensor);
            // The lower friction and the higher bounce of two touching
            // colliders win - so a 0 friction always slides, whatever it's on.
            if let Some(friction) = collider_data.friction {
                builder = builder.friction(friction).friction_combine_rule(CoefficientCombineRule::Min);
            }
            if let Some(restitution) = collider_data.restitution {
                builder = builder.restitution(restitution).restitution_combine_rule(CoefficientCombineRule::Max);
            }
            let handle = collider_set.insert_with_parent(builder.build(), rigidbody_handle, rigidbody_set);
            collider_handles.push(handle);
        }

        physics_handlers.insert(def.id.clone(), Some(PhysicsData { rigidbody_handle, collider_handles }));
    }
}

/// The Rapier collider for `shape` - None (and said why) for one that can't
/// be built. `id` is the body's node, for the message.
fn collider_builder(shape: &game_object::ColliderShape, id: &str) -> Option<ColliderBuilder> {
    use game_object::ColliderShape;
    match shape {
        ColliderShape::Cuboid { half_extents } => Some(ColliderBuilder::cuboid(half_extents.0, half_extents.1, half_extents.2)),
        ColliderShape::Capsule { half_height, radius } => Some(ColliderBuilder::capsule_y(*half_height, *radius)),
        ColliderShape::HalfSpace { normal } => Some(ColliderBuilder::halfspace(Unit::new_normalize(*normal))),
        ColliderShape::ConvexHull { points } => {
            let points: Vec<Point3<f32>> = points.iter().map(|point| Point3::from(*point)).collect();
            let builder = ColliderBuilder::convex_hull(&points);
            if builder.is_none() {
                eprintln!("convex collider for '{id}' couldn't be built (its points are flat or too few)");
            }
            builder
        }
        ColliderShape::Trimesh { vertices, indices } => {
            let points: Vec<Point3<f32>> = vertices.iter().map(|v| Point3::from(*v)).collect();
            ColliderBuilder::trimesh(points, indices.clone())
                .map_err(|error| eprintln!("trimesh collider for '{id}' couldn't be built: {error}"))
                .ok()
        }
        _ => None,
    }
}
