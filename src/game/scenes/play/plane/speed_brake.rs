use super::airframe::approach;

/// How fast the boards open and close, as a fraction of the travel per
/// second (1.0 => a one-second stroke).
const SPEED_BRAKE_RATE_PER_S: f32 = 0.8;

/// The speed brake - two boards on the aft fuselage. `deploy` is the whole
/// state: 0 stowed, 1 fully open, anything between is mid-stroke. It only
/// adds drag (see `Airframe`); the boards themselves are drawn on the F5
/// overlay from `deploy`.
pub struct SpeedBrake {
    /// 0..1 extension.
    pub deploy: f32,
    /// What the pilot last commanded (`true` = open).
    commanded_out: bool,
}

impl SpeedBrake {
    pub fn new() -> Self {
        Self { deploy: 0.0, commanded_out: false }
    }

    pub fn request_toggle(&mut self) {
        self.commanded_out = !self.commanded_out;
    }

    pub fn set_commanded_out(&mut self, out: bool) {
        self.commanded_out = out;
    }

    pub fn tick(&mut self, delta_time: f32) {
        let target = if self.commanded_out { 1.0 } else { 0.0 };
        self.deploy = approach(self.deploy, target, SPEED_BRAKE_RATE_PER_S, delta_time);
    }
}
