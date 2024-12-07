/// # Ui transform
/// 
/// This structs will have a main objective, let us set positioning of objects in the screen, by setting values like:
///     - Rect
///     - X and Y position
///     - Height and width
///     - Rotation (not yet implemented)
/// And more soon to follow

#[derive(Clone, Debug)]
pub struct Rect {
    pub top: u32,
    pub left: u32,
    pub bottom: u32,
    pub right: u32,
}

#[derive(Clone, Debug)]
pub struct UiTransform {
    pub rect: Rect,
    pub x: f32,
    pub y: f32,
    pub height: f32,
    pub width: f32,
    pub rotation: f32
}

impl UiTransform {
    pub fn new(x: f32, y: f32, height: f32, width: f32, rotation: f32) -> Self {

        let rect = Rect {
            top: y as u32,
            left: x as u32,
            bottom: (y + height) as u32,
            right: (x + width) as u32,
        };

        Self {
            rect,
            x,
            y,
            height,
            width,
            rotation,
        }
    }

    pub fn apply_transformation(&mut self) {
        self.rect = Rect {
            top: self.y as u32,
            left: self.x as u32,
            bottom: (self.y + self.height) as u32,
            right: (self.x + self.width) as u32,
        };
    }
}