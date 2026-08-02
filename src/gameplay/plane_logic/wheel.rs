use nalgebra::Vector3;
use rapier3d::prelude::{ColliderSet, QueryFilter, QueryPipeline, Ray, RigidBodySet};

use crate::{primitive::manual_vertex::ManualVertex, rendering::{instance_management::PhysicsData, render_line::render_basic_line}};

pub struct Wheel {
    pub offset: Vector3<f32>,  // Local offset of the wheel relative to the plane
    max_suspension_length: f32,
    pub stiffness: f32,
    pub damping: f32,
    pub mesh_name: String,
}

impl Wheel {
    pub fn new(offset: Vector3<f32>, max_suspension_length: f32, stiffness: f32, damping: f32, mesh_name: String) -> Self {
        Self { offset, max_suspension_length, stiffness, damping, mesh_name }
    }

    pub fn update_wheel(&mut self, physics_data: &PhysicsData, renderizable_lines: &mut Vec<[ManualVertex; 2]>, collider_set: &ColliderSet, rigidbody_set: &RigidBodySet, query_pipeline: &QueryPipeline) -> Option<(Vector3<f32>, Vector3<f32>, Vector3<f32>)> {
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
            if let Some((_handle, time_of_impact)) = query_pipeline.cast_ray(
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

                render_basic_line(renderizable_lines, suspension_origin, [0.5, 1.0, 0.5], wheel_position.coords, [0.5, 1.0, 0.5]);
                return Some((suspension_force, suspension_origin, wheel_position.coords));
            } else {
                render_basic_line(renderizable_lines, suspension_origin, [0.5, 1.0, 0.5], max_wheel_position, [0.5, 1.0, 0.5]);
                return Some((Vector3::identity(), suspension_origin, max_wheel_position));
            };
        }

        None
    }
}