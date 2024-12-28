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
    pub top: f32,
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
}

#[derive(Clone, Debug)]
pub struct UiTransform {
    pub rect: Rect,
    pub x: f32,
    pub y: f32,
    pub height: f32,
    pub width: f32,
    pub rotation: f32,
    pub smooth_change: bool
}

impl UiTransform {
    pub fn new(x: f32, y: f32, height: f32, width: f32, rotation: f32, smooth_change: bool) -> Self {

        let rect = Rect {
            top: y as f32,
            left: x as f32,
            bottom: (y + height) as f32,
            right: (x + width) as f32,
        };

        Self {
            rect,
            x,
            y,
            height,
            width,
            rotation,
            smooth_change
        }
    }

    pub fn apply_transformation(&mut self) {
        self.rect = Rect {
            top: self.y as f32,
            left: self.x as f32,
            bottom: (self.y + self.height) as f32,
            right: (self.x + self.width) as f32,
        };
    }
}