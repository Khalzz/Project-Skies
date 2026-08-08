use std::collections::{HashMap, HashSet};

use sdl2::controller::{Axis, Button};
use sdl2::mouse::MouseButton;
use serde::Deserialize;

fn default_deadzone() -> f32 { 0.25 }

/// A single physical input source an action can be triggered by. An action fires if
/// ANY of its bindings are active - e.g. Key("space") and ControllerButton("A") both
/// bound to the same action means either one triggers it, regardless of which device
/// it came from (bindings are device-agnostic: "any connected controller's A button",
/// not a specific controller index - see InputSubsystem's doc comment for why).
#[derive(Debug, Deserialize, Clone)]
pub enum Binding {
    Key(String),
    MouseButton(String),
    ControllerButton(String),
    ControllerAxis {
        axis: String,
        // Whether "pressed" means the axis leans positive or negative - lets a single
        // stick axis back two opposite actions (e.g. roll_left / roll_right).
        positive: bool,
        #[serde(default = "default_deadzone")]
        deadzone: f32,
    },
}

impl Binding {
    pub(crate) fn parse_mouse_button(name: &str) -> Option<MouseButton> {
        match name.to_lowercase().as_str() {
            "left" => Some(MouseButton::Left),
            "right" => Some(MouseButton::Right),
            "middle" => Some(MouseButton::Middle),
            "x1" => Some(MouseButton::X1),
            "x2" => Some(MouseButton::X2),
            _ => None,
        }
    }

    pub(crate) fn parse_controller_button(name: &str) -> Option<Button> {
        match name {
            "A" => Some(Button::A),
            "B" => Some(Button::B),
            "X" => Some(Button::X),
            "Y" => Some(Button::Y),
            "Back" => Some(Button::Back),
            "Guide" => Some(Button::Guide),
            "Start" => Some(Button::Start),
            "LeftStick" => Some(Button::LeftStick),
            "RightStick" => Some(Button::RightStick),
            "LeftShoulder" => Some(Button::LeftShoulder),
            "RightShoulder" => Some(Button::RightShoulder),
            "DPadUp" => Some(Button::DPadUp),
            "DPadDown" => Some(Button::DPadDown),
            "DPadLeft" => Some(Button::DPadLeft),
            "DPadRight" => Some(Button::DPadRight),
            _ => None,
        }
    }

    pub(crate) fn parse_controller_axis(name: &str) -> Option<Axis> {
        match name {
            "LeftX" => Some(Axis::LeftX),
            "LeftY" => Some(Axis::LeftY),
            "RightX" => Some(Axis::RightX),
            "RightY" => Some(Axis::RightY),
            "TriggerLeft" => Some(Axis::TriggerLeft),
            "TriggerRight" => Some(Axis::TriggerRight),
            _ => None,
        }
    }

    /// How strongly this one binding is currently activated (0.0..=1.0), given the raw
    /// device state aggregated across every connected device.
    fn strength(&self, raw: &RawInputState) -> f32 {
        match self {
            Binding::Key(key) => {
                if raw.keys_down.contains(&key.to_uppercase()) { 1.0 } else { 0.0 }
            }
            Binding::MouseButton(name) => match Self::parse_mouse_button(name) {
                Some(button) if raw.mouse_buttons_down.contains(&button) => 1.0,
                _ => 0.0,
            },
            Binding::ControllerButton(name) => match Self::parse_controller_button(name) {
                Some(button) if raw.controller_buttons_down.contains(&button) => 1.0,
                _ => 0.0,
            },
            Binding::ControllerAxis { axis, positive, deadzone } => match Self::parse_controller_axis(axis) {
                Some(axis) => {
                    let value = raw.controller_axes.get(&axis).copied().unwrap_or(0.0);
                    let signed = if *positive { value } else { -value };
                    if signed > *deadzone { signed.min(1.0) } else { 0.0 }
                }
                None => 0.0,
            },
        }
    }
}

/// What's physically held down right now, aggregated across every connected device -
/// updated incrementally as SDL events arrive in InputSubsystem::update, then used
/// once per frame to recompute every action's strength.
#[derive(Default)]
pub struct RawInputState {
    pub keys_down: HashSet<String>,
    pub mouse_buttons_down: HashSet<MouseButton>,
    pub controller_buttons_down: HashSet<Button>,
    pub controller_axes: HashMap<Axis, f32>,
}

/// Live state for one action: its bindings plus press/release edge tracking - the same
/// pressed/just_pressed/just_released shape Pressable used for a single key, just
/// aggregated over a whole list of bindings instead of one.
pub struct Action {
    pub bindings: Vec<Binding>,
    strength: f32,
    pressed: bool,
    just_pressed: bool,
    just_released: bool,
}

impl Action {
    pub fn new(bindings: Vec<Binding>) -> Self {
        Self { bindings, strength: 0.0, pressed: false, just_pressed: false, just_released: false }
    }

    pub(crate) fn refresh(&mut self, raw: &RawInputState, threshold: f32) {
        self.strength = self.bindings.iter().map(|binding| binding.strength(raw)).fold(0.0, f32::max);

        let was_pressed = self.pressed;
        self.pressed = self.strength > threshold;
        self.just_pressed = self.pressed && !was_pressed;
        self.just_released = !self.pressed && was_pressed;
    }

    pub fn is_pressed(&self) -> bool { self.pressed }
    pub fn is_just_pressed(&self) -> bool { self.just_pressed }
    pub fn is_just_released(&self) -> bool { self.just_released }
    pub fn strength(&self) -> f32 { self.strength }
}

#[derive(Debug, Deserialize)]
pub struct ActionConfig {
    pub action: String,
    pub bindings: Vec<Binding>,
}
