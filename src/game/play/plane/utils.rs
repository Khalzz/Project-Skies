use nalgebra::Vector3;

pub fn scale_6(value: Vector3<f32>, pos_x: f32, neg_x: f32, pos_y: f32, neg_y: f32, pos_z: f32, neg_z: f32) -> Vector3<f32> {
    let mut result = value;

    if result.x > 0.0 {
        result.x *= pos_x;
    } else if result.x < 0.0 {
        result.x *= neg_x;
    }

    if result.y > 0.0 {
        result.y *= pos_y;
    } else if result.y < 0.0 {
        result.y *= neg_y;
    }

    if result.z > 0.0 {
        result.z *= pos_z;
    } else if result.z < 0.0 {
        result.z *= neg_z;
    }

    result
}