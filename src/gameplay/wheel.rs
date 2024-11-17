use std::collections::HashMap;

use nalgebra::{vector, Point3, UnitVector3, Vector3};
use rapier3d::{parry::query, prelude::{ColliderHandle, ColliderSet, PhysicsPipeline, QueryFilter, QueryPipeline, Ray, RigidBody, RigidBodyHandle, RigidBodySet}};

use crate::{primitive::manual_vertex::ManualVertex, rendering::instance_management::{InstanceData, ModelDataInstance, PhysicsData}};

pub struct Wheel {
    pub offset: Vector3<f32>,  // Local offset of the wheel relative to the plane
    max_suspension_length: f32,
    pub stiffness: f32,
    pub damping: f32,
    pub shape_name: String,
}

impl Wheel {
    pub fn new(offset: Vector3<f32>, max_suspension_length: f32, stiffness: f32, damping: f32, shape_name: String) -> Self {
        Self { offset, max_suspension_length, stiffness, damping, shape_name }
    }

    pub fn update_wheel(&mut self, physics_data: &PhysicsData, renderizable_lines: &mut Vec<[ManualVertex; 2]>, collider_set: &ColliderSet, rigidbody_set: &mut RigidBodySet, query_pipeline: &QueryPipeline, models: &mut HashMap<String, ModelDataInstance>) -> Option<(Vector3<f32>, Vector3<f32>, Vector3<f32>)> {
        if let Some(rigidbody) = rigidbody_set.get(physics_data.rigidbody_handle) {
            // Origin of the raycast
            let rotation = rigidbody.rotation();
            let suspension_origin = rigidbody.translation() + (rotation * self.offset);
    
            // Direction of the ray (downward in local space)
            let local_ray_direction = -Vector3::y_axis();
            
            // Transforming local ray direction to world space
            let ray_direction = rotation * local_ray_direction;
            
            let max_wheel_position = suspension_origin + (ray_direction.into_inner() * self.max_suspension_length);
            
            // Raycast from the wheel downward to detect the ground
            let ray = Ray::new(suspension_origin.into(), ray_direction.into_inner());
            
            let mut filter = QueryFilter::default();
            filter.exclude_collider = physics_data.collider_handle;

            // Perform raycast
            if let Some((handle, time_of_impact)) = query_pipeline.cast_ray(
                rigidbody_set,
                collider_set,
                &ray,
                self.max_suspension_length,
                true,          // solid: treat colliders as solid
                filter
            ) {
                // Calculate compression based on hit distance
                let compression = 1.0 - (time_of_impact / self.max_suspension_length);
    
                // Calculate spring force (Hooke's law) and damping force
                let spring_force = compression * self.stiffness;
                let damping_force = rigidbody.linvel().y * self.damping;
    
                // Apply total force in the upward direction at the wheel position
                let suspension_force = Vector3::new(0.0, spring_force - damping_force, 0.0);
                let wheel_position = ray.point_at(time_of_impact);

                println!("is touching");
                Self::render_force_lines(suspension_origin, wheel_position.coords, models, renderizable_lines);
                return Some((suspension_force, suspension_origin, wheel_position.coords));
            } else {
                Self::render_force_lines(suspension_origin, max_wheel_position, models, renderizable_lines);
                return Some((Vector3::identity(), suspension_origin, max_wheel_position));
            };
        }

        None
    }

    pub fn render_force_lines(origin: Vector3<f32>, end: Vector3<f32>, models: &mut HashMap<String, ModelDataInstance>, renderizable_lines: &mut Vec<[ManualVertex; 2]>) {
        renderizable_lines.push([
            ManualVertex {
                position: origin.into(),  // Start point of thrust line in world space.
                color: [0.5, 1.0, 0.5],
            },
            ManualVertex {
                position: end.into(), // End point in world space.
                color: [0.5, 1.0, 0.5],
            },
        ]);
    }
}