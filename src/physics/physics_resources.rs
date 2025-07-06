use std::collections::HashMap;
use nalgebra::{vector, Unit};
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
                            let principal_inertia = nalgebra::Vector3::new(10000.0, 10000.0, 100000.0);

                            RigidBodyBuilder::dynamic()
                            .additional_mass_properties(rapier3d::prelude::MassProperties::new(physics_obj_data.rigidbody.center_of_mass.into(), physics_obj_data.rigidbody.mass, principal_inertia))
                            .translation(instance_data.transform.position)
                            .build()
                        };

                        rigid_body.set_linvel(physics_obj_data.rigidbody.initial_velocity, true);
                        let rigidbody_handle = rigidbody_set.insert(rigid_body);

                        // collisions
                        let collider_handle = match &physics_obj_data.collider {
                            Some(collider_data) => {
                                let collider = match collider_data {
                                    game_object::ColliderType::Cuboid { half_extents } => {
                                        ColliderBuilder::cuboid(half_extents.0, half_extents.1, half_extents.2).build()
                                    },
                                    game_object::ColliderType::HalfSpace { normal } => {
                                        ColliderBuilder::halfspace(Unit::new_normalize(*normal)).build()
                                    },
                                    _ => todo!(),
                                };

                                Some(collider_set.insert_with_parent(collider, rigidbody_handle, rigidbody_set))
                            },
                            None => {
                                None
                            },
                        };

                        physics_data = Some(PhysicsData { rigidbody_handle, collider_handle, metadata: HashMap::new() });
                    };

                    // println!("loaded data: {}", ids[i]);
                    physics_handlers.insert(ids[i].clone(), physics_data);
                }
            }
        },
        None => eprintln!("The instance data was not correctly loaded"),
    }
}