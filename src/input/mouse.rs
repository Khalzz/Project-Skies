use crate::input::pressable::Pressable;

pub struct Mouse {
    x: i32,
    y: i32,
    rel_x: i32,
    rel_y: i32,
    l_click: Pressable,
    r_click: Pressable,
    x_sensitivity: f32,
    y_sensitivity: f32,
    raw_x: i32,
    raw_y: i32,
}

/// Mouse
/// Raw x and y are the raw values from the mouse, defining their position on the screen
/// X and y are the values that are used for camera handling and other mouse based movements

impl Mouse {
    pub fn new(x_sensitivity: f32, y_sensitivity: f32) -> Self {
        Self {
            rel_x: 0,
            rel_y: 0,
            x: 0,
            y: 0,
            l_click: Pressable::new(None),
            r_click: Pressable::new(None),
            x_sensitivity,
            y_sensitivity,
            raw_x: 0,
            raw_y: 0,
        }
    }

    pub fn get_x(&self) -> i32 {
        self.x
    }

    pub fn get_y(&self) -> i32 {
        self.y
    }

    pub fn set_x(&mut self, x: i32) {
        self.x = x;
    }

    pub fn set_y(&mut self, y: i32) {
        self.y = y;
    }

    pub fn get_rel_x(&self) -> i32 {
        self.rel_x
    }

    pub fn get_rel_y(&self) -> i32 {
        self.rel_y
    }

    pub fn set_rel_x(&mut self, rel_x: i32) {
        self.rel_x = rel_x;
    }

    pub fn set_rel_y(&mut self, rel_y: i32) {
        self.rel_y = rel_y;
    }

    

    pub fn reset_rel_x(&mut self) {
        self.rel_x = 0;
    }

    pub fn reset_rel_y(&mut self) {
        self.rel_y = 0;
    }

    pub fn get_sensitivity(&self) -> (f32, f32) {
        (self.x_sensitivity, self.y_sensitivity)
    }

    pub fn set_sensitivity(&mut self, x_sensitivity: f32, y_sensitivity: f32) {
        self.x_sensitivity = x_sensitivity;
        self.y_sensitivity = y_sensitivity;
    }

    pub fn set_raw_x(&mut self, x: i32) {
        self.raw_x = x;
    }

    pub fn set_raw_y(&mut self, y: i32) {
        self.raw_y = y;
    }

}