use nalgebra::{Matrix4, Point3, Quaternion, UnitQuaternion, Vector3};

use crate::transform::Transform;

#[derive(Copy, Clone, Debug)]
pub struct Camera {
    pub transform: Transform,
    pub look_at: Option<Point3<f32>>,
    pub up: Vector3<f32>,
    pub rotation_modifier: UnitQuaternion<f32>,
}

impl Camera {
    pub fn new(transform: Transform) -> Self {
        Self {
            transform,
            look_at: None,
            up: Vector3::y_axis().into_inner(),
            rotation_modifier: UnitQuaternion::identity(),
        }
    }

    pub fn position(&self) -> Point3<f32> {
        Point3::from(self.transform.position)
    }

    pub fn set_position(&mut self, position: Point3<f32>) {
        self.transform.position = position.coords;
    }

    pub fn yaw(&self) -> f32 {
        yaw_pitch_from_rotation(self.transform.rotation).0
    }

    pub fn pitch(&self) -> f32 {
        yaw_pitch_from_rotation(self.transform.rotation).1
    }

    pub fn set_yaw_pitch(&mut self, yaw: f32, pitch: f32) {
        self.transform.rotation = yaw_pitch_rotation(yaw.to_degrees(), pitch.to_degrees());
    }

    pub fn look_at(&mut self, target: Point3<f32>) {
        let direction = target - self.position();
        let normalized_dir = direction.normalize();

        // Calculate pitch and yaw using asin and atan2 for the direction vector.
        let pitch = normalized_dir.y.asin();
        let yaw = normalized_dir.z.atan2(normalized_dir.x);
        self.set_yaw_pitch(yaw, pitch);
    }

    pub fn calc_forward_direction(&self) -> Vector3<f32> {
        UnitQuaternion::from_quaternion(self.transform.rotation) * Vector3::new(1.0, 0.0, 0.0)
    }

    pub fn calc_matrix(&self) -> Matrix4<f32> {
        let direction = self.calc_forward_direction();
        let modified_direction = self.rotation_modifier * direction;
        let modified_up = self.rotation_modifier * self.up;

        let origin = Point3::origin();
        Matrix4::look_at_rh(&origin, &(origin + modified_direction), &modified_up)
    }
}

fn yaw_pitch_from_rotation(rotation: Quaternion<f32>) -> (f32, f32) {
    let forward = UnitQuaternion::from_quaternion(rotation) * Vector3::new(1.0, 0.0, 0.0);
    let yaw = forward.z.atan2(forward.x);
    let pitch = forward.y.clamp(-1.0, 1.0).asin();
    (yaw, pitch)
}

pub fn yaw_pitch_rotation(yaw_deg: f32, pitch_deg: f32) -> Quaternion<f32> {
    let rotation = UnitQuaternion::from_axis_angle(&Vector3::y_axis(), -yaw_deg.to_radians())
        * UnitQuaternion::from_axis_angle(&Vector3::z_axis(), pitch_deg.to_radians());
    *rotation
}
