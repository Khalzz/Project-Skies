/*
    The wheel brakes as the cockpit has them: the toe brakes on top of the rudder pedals, one per main wheel so
    they can brake one side harder to turn, and the parking brake (the F-16's ANTI-SKID switch has a PARKING
    BRAKE position) that holds both on. Only how hard they're asked to brake (0..1); the brakes are the wheels'.
*/

const PRESSURE_RATE: f32 = 3.0; // share of full pressure per second the pilot's toes build or let go of

pub struct WheelBrakes {
    pub left: f32,
    pub right: f32,
    pub parking: bool,
}

impl WheelBrakes {
    pub fn new() -> WheelBrakes {
        WheelBrakes {
            left: 0.0,
            right: 0.0,
            parking: false,
        }
    }

    // Each toe pressing (1) or not (0); the pressure follows at the rate a foot pushes
    pub fn press(&mut self, left: f32, right: f32, dt: f32) {
        let step = PRESSURE_RATE * dt;
        self.left += (left.clamp(0.0, 1.0) - self.left).clamp(-step, step);
        self.right += (right.clamp(0.0, 1.0) - self.right).clamp(-step, step);
    }

    // What each main wheel's brake is asked for: (left, right)
    pub fn command(&self) -> [f32; 2] {
        if self.parking {
            [1.0, 1.0]
        } else {
            [self.left, self.right]
        }
    }
}
