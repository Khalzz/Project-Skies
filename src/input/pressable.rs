pub struct Pressable {
    pub keys: Option<Vec<String>>,
    is_pressed: bool,
    is_just_pressed: bool,
    is_released: bool,
    time_pressed: f32,
}

impl Pressable {
    pub fn new(keys: Option<Vec<&str>>) -> Self {
        Self {
            keys: keys.map(|keys| keys.iter().map(|k| k.to_string().to_uppercase()).collect()),
            is_pressed: false,
            is_just_pressed: false,
            is_released: false,
            time_pressed: 0.0,
        }
    }

    pub fn is_pressed(&self) -> bool {
        self.is_pressed
    }

    pub fn is_just_pressed(&self) -> bool {
        self.is_just_pressed
    }

    pub fn is_released(&self) -> bool {
        self.is_released
    }

    pub fn set_pressed(&mut self, pressed: bool, delta_time: f32) {
        let was_pressed = self.is_pressed;
        self.is_pressed = pressed;
        
        // Handle just pressed
        self.is_just_pressed = pressed && !was_pressed;
        
        // Handle release
        self.is_released = !pressed && was_pressed;
        
        if pressed {
            self.time_pressed += delta_time;
        }
    }

    pub fn set_just_pressed(&mut self, is_just_pressed: bool) {
        self.is_just_pressed = is_just_pressed;
    }

    pub fn set_released(&mut self, is_released: bool) {
        self.is_released = is_released;
    }
}