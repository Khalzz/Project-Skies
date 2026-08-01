use std::collections::HashMap;
use nalgebra::{vector, Unit, Vector3};
use rapier3d::prelude::*;

use crate::physics::physics_handler::PhysicsData;
use crate::game_nodes::game_object::GameObject;
use crate::resources::load_instances;
use crate::game_nodes::game_object;

/*
todo:
    - Make a new way of saving physics data for each loaded element in the level, this will be saved on the 
    physics thread, and all the modifications will be done on it, allowing us to create our own iteration of a
    "FIXED UPDATE" only dedicated to physics
*/



/// A rigid body's Cuboid colliders, treated as a mass-distributed box assembly, to
/// approximate its rotational inertia. Mass is distributed across colliders proportional
/// to their volume, and each box's own inertia plus its parallel-axis contribution
/// (from sitting off the body's center of mass) is summed. This makes inertia follow
/// whatever real-world dimensions and mass a plane is actually authored with, instead of
/// a single constant every dynamic body shared regardless of size — so a model built at
/// (1,1,1) and scaled up to its real-life size (colliders/positions authored to match)
/// gets physically appropriate rotational resistance automatically, with no per-plane
/// hand-tuning of inertia required.
fn compute_principal_inertia(mass: f32, center_of_mass: Vector3<f32>, colliders: &[game_object::ColliderType]) -> Vector3<f32> {
    struct BoxPart {
        volume: f32,
        half_extents: (f32, f32, f32),
        position: (f32, f32, f32),
    }

    let boxes: Vec<BoxPart> = colliders.iter().filter_map(|collider| match collider {
        game_object::ColliderType::Cuboid { half_extents, position } => Some(BoxPart {
            volume: 8.0 * half_extents.0 * half_extents.1 * half_extents.2,
            half_extents: *half_extents,
            position: *position,
        }),
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

pub fn load_physics_from_level(mut level_path: String, collider_set: &mut ColliderSet, rigidbody_set: &mut RigidBodySet, physics_handlers: &mut HashMap<String, Option<PhysicsData>>) {

    level_path += "/data.ron";

    let mut physics_data: HashMap<String, PhysicsData> = HashMap::new();

    let instances_data_to_load = load_instances(level_path);
    match instances_data_to_load {
        Some(instances) => {
            
            // Load the models name so we can identify all physics data
            let mut models: Vec<String> = vec![];
            
            for data in &instances {
                if !models.contains(&data.model.to_string()) {
                    models.push(data.model.to_string())
                }
            }
            // Load the models name so we can identify all physics data

            // For each model loaded
            for model_name in &models {
                let mut ids: Vec<String> = vec![];
                let mut model_instances:Vec<GameObject> = vec![];

                for game_object in &instances {
                    if &game_object.model == model_name {
                        ids.push(game_object.id.clone());
                        model_instances.push(game_object.clone());
                    }
                }

                for (i, instance_data) in model_instances.iter().enumerate() {
                    // Physics
                    let mut physics_data: Option<PhysicsData> = None;

                    if let Some(physics_obj_data) = &instance_data.metadata.physics {
                        let mut rigid_body = if physics_obj_data.rigidbody.is_static {
                            RigidBodyBuilder::fixed().additional_mass(physics_obj_data.rigidbody.mass).translation(vector![instance_data.transform.position.x, instance_data.transform.position.y, instance_data.transform.position.z]).build()
                        } else {
                            let principal_inertia = compute_principal_inertia(
                                physics_obj_data.rigidbody.mass,
                                physics_obj_data.rigidbody.center_of_mass,
                                &physics_obj_data.colliders,
                            );

                            RigidBodyBuilder::dynamic()
                            .additional_mass_properties(rapier3d::prelude::MassProperties::new(physics_obj_data.rigidbody.center_of_mass.into(), physics_obj_data.rigidbody.mass, principal_inertia))
                            .translation(instance_data.transform.position)
                            .angular_damping(2.0)
                            .build()
                        };

                        rigid_body.set_linvel(physics_obj_data.rigidbody.initial_velocity, true);
                        let rigidbody_handle = rigidbody_set.insert(rigid_body);

                        // Create colliders
                        let mut collider_handles: Vec<ColliderHandle> = Vec::new();

                        for collider_data in &physics_obj_data.colliders {
                            let collider = match collider_data {
                                game_object::ColliderType::Cuboid { half_extents, position } => {
                                    ColliderBuilder::cuboid(half_extents.0, half_extents.1, half_extents.2)
                                        .translation(vector![position.0, position.1, position.2])
                                        .build()
                                },
                                game_object::ColliderType::HalfSpace { normal } => {
                                    ColliderBuilder::halfspace(Unit::new_normalize(*normal)).build()
                                },
                                _ => continue,
                            };
                            let handle = collider_set.insert_with_parent(collider, rigidbody_handle, rigidbody_set);
                            collider_handles.push(handle);
                        }

                        physics_data = Some(PhysicsData { rigidbody_handle, collider_handles, metadata: HashMap::new() });
                    };

                    // println!("loaded data: {}", ids[i]);
                    physics_handlers.insert(ids[i].clone(), physics_data);
                }
            }
        },
        None => eprintln!("The instance data was not correctly loaded"),
    }
}