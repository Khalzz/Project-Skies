use nalgebra::Matrix4;

use super::camera::Camera;
use super::projection::Projection;

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct NearFarUniform {
    pub near: f32,
    pub far: f32,
}

// the cameraUniform will get us the positional matrix of the camera
#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CameraUniform {
    // pub - read directly by handler::CameraHandler::world_to_screen, a
    // sibling module now that this struct has its own file.
    pub view_proj: [[f32; 4]; 4],
    view_position: [f32; 4],
    near: f32,
    far: f32
}

impl CameraUniform {
    pub fn new(near_far: NearFarUniform) -> Self {
        Self {
            view_proj: Matrix4::identity().into(),
            view_position: [0.0; 4],
            near: near_far.near,
            far: near_far.far
        }
    }

    pub fn update_view_proj(&mut self, camera: &Camera, projection: &Projection) {
        // The camera is always at the origin in the space these matrices operate in.
        self.view_position = [0.0, 0.0, 0.0, 1.0];
        self.view_proj = (projection.calc_matrix() * camera.calc_matrix()).into();
    }
}
