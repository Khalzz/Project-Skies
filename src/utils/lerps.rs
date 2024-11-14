use nalgebra::{Point3, Quaternion, Vector3};


pub fn lerp(start: f32, end: f32, t: f32) -> f32 {
    start + (end - start) * t
}

pub fn lerp_point3(start: Point3<f32>, end: Point3<f32>, t: f32) -> Point3<f32> {
    Point3::new(
        start.x + (end.x - start.x) * t,
        start.y + (end.y - start.y) * t,
        start.z + (end.z - start.z) * t
    )
}

pub fn lerp_vector3(start: Vector3<f32>, end: Vector3<f32>, t: f32) -> Vector3<f32> {
    Vector3::new(
        start.x + (end.x - start.x) * t,
        start.y + (end.y - start.y) * t,
        start.z + (end.z - start.z) * t
    )
}

pub fn lerp_quaternion(start: Quaternion<f32>, end: Quaternion<f32>, t: f32) -> Quaternion<f32> {
    Quaternion::new(
        start.w + (end.w - start.w) * t,
        start.i + (end.i - start.i) * t,
        start.j + (end.j - start.j) * t,
        start.k + (end.k - start.k) * t
    )
}