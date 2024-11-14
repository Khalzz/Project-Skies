use nalgebra::{Matrix4, Quaternion, UnitQuaternion, Vector3};

#[derive(Debug, Clone, Copy)]
pub struct Transform {
    pub position: Vector3<f32>,
    pub rotation: Quaternion<f32>,
    pub scale: Vector3<f32>,
}

impl Transform {
    pub fn new(position: Vector3<f32>, rotation: Quaternion<f32>, scale: Vector3<f32>) -> Self {
        Self {
            position,
            rotation,
            scale,
        }
    }

    pub fn to_matrix(&self) -> Matrix4<f32> {
        let translation = Matrix4::new_translation(&self.position);
        let rotation = UnitQuaternion::from_quaternion(self.rotation).to_homogeneous();
        let scale = Matrix4::new_nonuniform_scaling(&self.scale);
        translation * rotation * scale
    }

    pub fn to_matrix_bufferable(&self) -> [[f32; 4]; 4] {
        let translation = Matrix4::new_translation(&self.position);
        let rotation = UnitQuaternion::from_quaternion(self.rotation).to_homogeneous();
        let scale = Matrix4::new_nonuniform_scaling(&self.scale);
        let matrix = translation * rotation * scale;
        // Convert Matrix4<f32> to [[f32; 4]; 4]
        matrix.into()
    }
}