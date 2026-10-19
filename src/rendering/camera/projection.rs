use nalgebra::{Matrix4, Perspective3};

// Maps OpenGL's [-1, 1] NDC z-range to wgpu's [0, 1] range, and reverses it
// (near -> 1, far -> 0) so depth precision concentrates on distant geometry
// instead of being wasted right in front of the near plane. Must be paired
// with CompareFunction::Greater and a depth clear value of 0.0.
#[rustfmt::skip]
pub const OPENGL_TO_WGPU_MATRIX: Matrix4<f32> = Matrix4::new(
    1.0, 0.0, 0.0, 0.0,
    0.0, 1.0, 0.0, 0.0,
    0.0, 0.0, -0.5, 0.5,
    0.0, 0.0, 0.0, 1.0,
);

// Projection struct using nalgebra's Perspective3 for perspective projection calculations.
pub struct Projection {
    aspect: f32,
    pub fovy: f32,
    pub znear: f32,
    zfar: f32,
}

impl Projection {
    pub fn new(width: u32, height: u32, fovy: f32, znear: f32, zfar: f32) -> Self {
        Self {
            aspect: width as f32 / height as f32,
            fovy,
            znear,
            zfar,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.aspect = width as f32 / height as f32;
    }

    pub fn calc_matrix(&self) -> Matrix4<f32> {
        OPENGL_TO_WGPU_MATRIX * Perspective3::new(self.aspect, self.fovy.to_radians(), self.znear, self.zfar).to_homogeneous()
    }
}
