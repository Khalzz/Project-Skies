use cgmath::{Matrix4, Quaternion, Vector3};

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
        let translation = Matrix4::from_translation(self.position);
        let rotation = Matrix4::from(self.rotation);
        let scale = Matrix4::from_nonuniform_scale(self.scale.x, self.scale.y, self.scale.z);
        translation * rotation * scale
    }

    pub fn to_matrix_bufferable(&self) -> [[f32; 4]; 4] {
        let translation = Matrix4::from_translation(self.position);
        let rotation = Matrix4::from(self.rotation);
        let scale = Matrix4::from_nonuniform_scale(self.scale.x, self.scale.y, self.scale.z);
        let matrix = translation * rotation * scale;
        // Convert Matrix4<f32> to [[f32; 4]; 4]
        matrix.into()
    }
}