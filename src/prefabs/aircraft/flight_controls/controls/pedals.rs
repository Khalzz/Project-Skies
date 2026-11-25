pub struct Pedals {
    pub yaw: f32,
}

impl Pedals {
    pub fn new() -> Pedals {
        Pedals { yaw: 0.0 }
    }

    pub fn follow(&mut self, yaw: f32, dt: f32) {
        let ease = (dt * 6.0).min(1.0);
        self.yaw += (yaw - self.yaw) * ease;
    }
}
