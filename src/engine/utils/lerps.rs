use nalgebra::{Point3, Quaternion, Vector3};


pub fn lerp(start: f32, end: f32, t: f32) -> f32 {
    start + (end - start) * t
}

/// Eases a 0..1 progress value with a soft start/end (0 and 1 slopes both flatten
/// out) instead of the linear ramp `t` would give - clamps out-of-range input so
/// callers can pass a raw `elapsed / duration` without pre-clamping it themselves.
pub fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn lerp_u32(start: u32, end: u32, t: f32,) -> i32 {
    // Calculate the interpolated value
    (start as f32 * (1.0 - t) + end as f32 * t).round() as i32
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