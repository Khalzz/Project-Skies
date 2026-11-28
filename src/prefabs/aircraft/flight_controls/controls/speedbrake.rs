/*
    The speedbrake switch: only a request (0 closed .. 1 fully open), how far the panels go and how
    fast is the airframe's. The F-16's is a thumb switch on the throttle; here a key opens them all
    the way or closes them.
*/

pub struct SpeedbrakeSwitch {
    pub request: f32,
}

impl SpeedbrakeSwitch {
    pub fn toggle(&mut self) {
        self.request = if self.request > 0.0 { 0.0 } else { 1.0 };
    }
}
