const THROTTLE_RATE: f32 = 0.5; // full travel in 2 seconds
const LEVER_MOVED: f32 = 0.002;

pub struct Throttle {
    position: f32,
    last_lever: Option<f32>,
}

impl Throttle {
    pub fn new(position: f32) -> Throttle {
        Throttle {
            position: position.clamp(0.0, 1.0),
            last_lever: None,
        }
    }

    pub fn nudge(&mut self, direction: f32, dt: f32) {
        self.position = (self.position + direction * THROTTLE_RATE * dt).clamp(0.0, 1.0);
    }

    // The lever only takes over when it actually moves
    pub fn follow_lever(&mut self, lever: Option<f32>) {
        let Some(lever) = lever else {
            self.last_lever = None;
            return;
        };

        if self
            .last_lever
            .is_none_or(|last| (last - lever).abs() > LEVER_MOVED)
        {
            self.position = lever.clamp(0.0, 1.0);
            self.last_lever = Some(lever);
        }
    }

    pub fn set(&mut self, position: f32) {
        self.position = position.clamp(0.0, 1.0);
    }

    pub fn position(&self) -> f32 {
        self.position
    }
}
