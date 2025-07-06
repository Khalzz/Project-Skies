use crate::input::input::InputSubsystem;
use crate::input::utils::to_axis;

#[derive(Clone)]
pub struct PlaneControls {
    pub throttle: f32,
    pub elevator: f32,
    pub aileron: f32,
    pub rudder: f32,
}

impl PlaneControls {
    pub fn new() -> Self {
        Self { throttle: 0.0, elevator: 0.0, aileron: 0.0, rudder: 0.0 }
    }
}

pub struct Plane {
    pub controls: PlaneControls,
}

impl Plane {
    pub fn new() -> Self {
        Self { controls: PlaneControls { throttle: 0.0, elevator: 0.0, aileron: 0.0, rudder: 0.0 } }
    }

    pub fn update(&mut self, delta_time: f32, input_subsystem: &InputSubsystem) {
        self.axis_logic(input_subsystem);
        self.throttle_logic(input_subsystem, delta_time);
    }

    pub fn axis_logic(&mut self, input_subsystem: &InputSubsystem) {
        self.controls.elevator = to_axis(input_subsystem.is_pressed("pitch_up"), input_subsystem.is_pressed("pitch_down"));
        self.controls.aileron = to_axis(input_subsystem.is_pressed("roll_left"), input_subsystem.is_pressed("roll_right"));
        self.controls.rudder = to_axis(input_subsystem.is_pressed("rudder_left"), input_subsystem.is_pressed("rudder_right"));
    }

    pub fn throttle_logic(&mut self, input_subsystem: &InputSubsystem, delta_time: f32) {
        if input_subsystem.is_pressed("throttle_up") {
            self.controls.throttle = (self.controls.throttle + 1.0 * delta_time).clamp(0.0, 1.0);
        }

        if input_subsystem.is_pressed("throttle_down") {
            self.controls.throttle = (self.controls.throttle - 1.0 * delta_time).clamp(0.0, 1.0);
        }
    }
}

