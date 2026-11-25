pub struct Stick {
    pub pitch: f32, // -1 forward (nose down) .. 1 back (nose up)
    pub roll: f32,  // -1 left .. 1 right
}

impl Stick {
    pub fn follow(&mut self, pitch: f32, roll: f32, dt: f32) {
        let ease = (dt * 6.0).min(1.0);
        self.pitch += (pitch - self.pitch);
        self.roll += (roll - self.roll);
    }
}
