use crate::engine::input::input;

#[derive(Clone)]
pub struct PlaneControls {
    pub throttle: f32,
    pub elevator: f32,
    pub aileron: f32,
    pub rudder: f32,
    pub trim_pitch: f32,
    pub trim_roll: f32,
    pub trim_yaw: f32,
}

impl PlaneControls {
    pub fn new() -> Self {
        Self { throttle: 0.0, elevator: 0.0, aileron: 0.0, rudder: 0.0, trim_pitch: 0.29, trim_roll: 0.0, trim_yaw: 0.0 }
    }
}

pub struct Plane {
    pub controls: PlaneControls,
}

impl Plane {
    pub fn new() -> Self {
        Self { controls: PlaneControls { throttle: 0.0, elevator: 0.0, aileron: 0.0, rudder: 0.0, trim_pitch: 0.29, trim_roll: 0.0, trim_yaw: 0.0 } }
    }

    pub fn update(&mut self, delta_time: f32) {
        self.axis_logic();
        self.throttle_logic(delta_time);
        self.trim_logic(delta_time);
    }

    pub fn axis_logic(&mut self) {
        // get_axis reads continuous strength (not just pressed/not-pressed), so a stick
        // binding gives proportional deflection while a keyboard binding still cleanly
        // resolves to -1.0/0.0/1.0 (a key's strength is always exactly 0.0 or 1.0).
        self.controls.elevator = input::get_axis("pitch_up", "pitch_down");
        self.controls.aileron = input::get_axis("roll_left", "roll_right");
        self.controls.rudder = input::get_axis("rudder_left", "rudder_right");
    }

    pub fn throttle_logic(&mut self, _delta_time: f32) {
        // Absolute position, not a ramp: a trigger's deflection directly sets throttle,
        // like a real lever - so releasing both throttle_up/throttle_down now returns
        // throttle to 0 instead of holding its last value (that "hold when idle" feel
        // was a keyboard-only accommodation, not something a real throttle does).
        self.controls.throttle = input::get_axis("throttle_down", "throttle_up").clamp(0.0, 1.0);
    }

    pub fn trim_logic(&mut self, delta_time: f32) {
        let trim_speed = 0.5;
        if input::is_action_pressed("trim_pitch_up") {
            self.controls.trim_pitch = (self.controls.trim_pitch + trim_speed * delta_time).clamp(-1.0, 1.0);
        }
        if input::is_action_pressed("trim_pitch_down") {
            self.controls.trim_pitch = (self.controls.trim_pitch - trim_speed * delta_time).clamp(-1.0, 1.0);
        }
        if input::is_action_pressed("trim_roll_left") {
            self.controls.trim_roll = (self.controls.trim_roll - trim_speed * delta_time).clamp(-1.0, 1.0);
        }
        if input::is_action_pressed("trim_roll_right") {
            self.controls.trim_roll = (self.controls.trim_roll + trim_speed * delta_time).clamp(-1.0, 1.0);
        }
        if input::is_action_pressed("trim_yaw_left") {
            self.controls.trim_yaw = (self.controls.trim_yaw - trim_speed * delta_time).clamp(-1.0, 1.0);
        }
        if input::is_action_pressed("trim_yaw_right") {
            self.controls.trim_yaw = (self.controls.trim_yaw + trim_speed * delta_time).clamp(-1.0, 1.0);
        }
    }
}
