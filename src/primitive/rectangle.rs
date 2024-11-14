
pub const NUM_INDICES: u32 = 6;

use nalgebra::{vector, Quaternion, UnitQuaternion, Vector3};

use crate::{app::Size, rendering::vertex::VertexUi};

use crate::app::MousePos;

#[derive(Copy, Clone, Debug)]
pub struct RectPos {
    pub top: u32,
    pub left: u32,
    pub bottom: u32,
    pub right: u32,
}

#[derive(Debug, Clone)]
pub struct Rectangle {
    pub position: RectPos,
    pub color: [f32; 4],
    color_active: [f32; 4],
    pub border_color: [f32; 4],
    border_color_active: [f32; 4],
    pub rotation: Quaternion<f32>
}

impl Rectangle {
    pub fn new(position: RectPos, color: [f32; 4], color_active: [f32; 4], border_color: [f32; 4], border_color_active: [f32; 4], rotation: Quaternion<f32>) -> Self {
        Self {
            color,
            color_active,
            border_color,
            border_color_active,
            position,
            rotation
        }
    }

    pub fn indices(&self, base: u16) -> [u16; 6] {
        [base, 1 + base, 2 + base, base, 2 + base, 3 + base]
    }

    pub fn vertices( &mut self, is_active: bool, size: &Size ) -> [VertexUi; 4] {
        // We define the position of each object based on the height defined
        let top = 1.0 - (self.position.top as f32 / (size.height as f32 / 2.0));
        let left = (self.position.left as f32 / (size.width as f32 / 2.0)) - 1.0;
        let bottom = 1.0 - (self.position.bottom as f32 / (size.height as f32 / 2.0));
        let right = (self.position.right as f32 / (size.width as f32 / 2.0)) - 1.0;

        let rect = [
            self.position.top as f32,
            self.position.left as f32,
            self.position.bottom as f32,
            self.position.right as f32,
        ];

        let mut color = self.color;
        let mut border_color = self.border_color;

        if is_active {
            color = self.color_active;
            border_color = self.border_color_active;
        }

        let x_center = left + ((right - left) / 2.0);
        let y_center = top + ((bottom - top) / 2.0);

        let center = vector![x_center, y_center, 0.0];

        let left_top = Self::rotate_from_center(vector![left, top, 0.0], center, self.rotation);
        let left_bottom = Self::rotate_from_center(vector![left, bottom, 0.0], center, self.rotation);
        let right_top = Self::rotate_from_center(vector![right, top, 0.0], center, self.rotation);
        let right_bottom = Self::rotate_from_center(vector![right, bottom, 0.0], center, self.rotation);

        // let left_top = [left, top, 0.0];
        // let left_bottom = [left, bottom, 0.0];
        // let right_top = [right, top, 0.0];
        // let right_bottom = [right, bottom, 0.0];

        [
            VertexUi { position: left_top, color, rect, border_color, },
            VertexUi { position: left_bottom, color, rect, border_color, },
            VertexUi { position: right_bottom, color, rect, border_color, },
            VertexUi { position: right_top, color, rect, border_color, },
        ]
    }
    
    pub fn rotate_from_center(vector: Vector3<f32>, center: Vector3<f32>, rotation: Quaternion<f32>) -> [f32; 3] {
        // Translate the vector to the origin (center point becomes the origin)
        let plane_rot = rotation.conjugate();
        let world_rot: Quaternion<f32> = Quaternion::identity();

        let result =  plane_rot - world_rot;
        let euler = UnitQuaternion::from_quaternion(result.conjugate()).euler_angles();

        let translated_vector = vector - center;

        // Rotate the translated vector
        let rotated_vector = UnitQuaternion::from_axis_angle(&Vector3::z_axis(), euler.2).transform_vector(&translated_vector);

        // Translate back to the original position
        let final_vector = rotated_vector + center;

        final_vector.into()
    }
    
    pub fn is_hovered(&self, mouse_coords: &MousePos) -> bool {
        let rect_pos = self.position; 
        mouse_coords.x > rect_pos.left as f64 && mouse_coords.x < rect_pos.right as f64 && mouse_coords.y > rect_pos.top as f64 && mouse_coords.y < rect_pos.bottom as f64
    }
}