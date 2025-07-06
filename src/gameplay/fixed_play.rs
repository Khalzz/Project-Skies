struct FixedPlay {
    pub plane: Plane,
    pub wheels: Vec<Wheel>,
}

impl FixedPlay {
    pub fn new() -> Self {
        Self { plane: Plane::new() }
    }

    pub fn update(&mut self, delta_time: f32) {
        for wheel in self.wheels.iter_mut() {
            wheel.update(delta_time);
        }
    }
}