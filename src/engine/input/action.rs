use std::collections::{HashMap, HashSet};

use sdl2::controller::{Axis, Button};
use sdl2::joystick::HatState;
use sdl2::mouse::MouseButton;
use serde::{Deserialize, Serialize};

fn default_deadzone() -> f32 { 0.25 }

/// A flight stick's center is far more precise than a gamepad thumbstick's -
/// a much smaller dead zone (see `Binding::JoystickAxis`).
fn default_joystick_deadzone() -> f32 { 0.05 }

fn is_false(value: &bool) -> bool { !*value }

/// A single physical input source an action can be triggered by. An action fires if
/// ANY of its bindings are active - e.g. Key("space") and ControllerButton("A") both
/// bound to the same action means either one triggers it, regardless of which device
/// it came from. Gamepad bindings are device-agnostic ("any connected controller's A
/// button"); joystick ones (HOTAS sticks, throttles, pedals, button boxes - anything
/// SDL doesn't know as a gamepad) name their device, since every such device has its
/// own "button 1".
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
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
    /// A joystick's button, by its device's name (as SDL reports it) and its
    /// index (0-based - labelled from 1).
    JoystickButton { device: String, button: u32 },
    /// A joystick's axis, by its device's name and index.
    JoystickAxis {
        device: String,
        axis: u32,
        /// Which way is "more" - the axis leaning positive, or negative.
        positive: bool,
        /// Ignored for `full_range` axes.
        #[serde(default = "default_joystick_deadzone")]
        deadzone: f32,
        /// A lever (a throttle): its whole travel is 0..1, end to end -
        /// instead of a centered stick axis, 0 at its center and 1 at one
        /// end (see `input::absolute_value`).
        #[serde(default, skip_serializing_if = "is_false")]
        full_range: bool,
    },
    /// A joystick's hat (a POV switch) pushed one way - "Up", "Down",
    /// "Left" or "Right" (a diagonal counts for both its directions).
    JoystickHat { device: String, hat: u32, direction: String },
}

/// Longest a device's name gets in a binding's label before it's cut short.
const DEVICE_LABEL_LENGTH: usize = 14;

fn short_device(device: &str) -> String {
    if device.chars().count() <= DEVICE_LABEL_LENGTH {
        device.to_owned()
    } else {
        format!("{}…", device.chars().take(DEVICE_LABEL_LENGTH).collect::<String>().trim_end())
    }
}

impl Binding {
    /// Short human-readable label for a rebinding menu - e.g. "W", "Controller A",
    /// "LeftY +", "VKB-Sim Gladi… Btn 3". Not meant to round-trip back into a
    /// Binding, just for display.
    pub fn label(&self) -> String {
        match self {
            Binding::Key(key) => key.clone(),
            Binding::MouseButton(name) => format!("Mouse {name}"),
            Binding::ControllerButton(name) => format!("Controller {name}"),
            Binding::ControllerAxis { axis, positive, .. } => format!("{axis} {}", if *positive { "+" } else { "-" }),
            Binding::JoystickButton { device, button } => format!("{} Btn {}", short_device(device), button + 1),
            Binding::JoystickAxis { device, axis, positive, full_range, .. } => {
                let way = if *full_range {
                    if *positive { " (lever)" } else { " (lever, reversed)" }
                } else if *positive { " +" } else { " -" };
                format!("{} Axis {}{way}", short_device(device), axis + 1)
            }
            Binding::JoystickHat { device, hat, direction } => format!("{} Hat {} {direction}", short_device(device), hat + 1),
        }
    }

    /// This binding as a lever (see `JoystickAxis::full_range`) - for an
    /// action read as an absolute value (a throttle). Anything other than a
    /// joystick axis is left as it is.
    pub fn as_full_range(self) -> Self {
        match self {
            Binding::JoystickAxis { device, axis, positive, deadzone, .. } => Binding::JoystickAxis { device, axis, positive, deadzone, full_range: true },
            other => other,
        }
    }

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

    /// Whether a hat `state` counts as pushed `direction` - a diagonal
    /// counts for both of its directions.
    pub(crate) fn hat_points(state: HatState, direction: &str) -> bool {
        use HatState::*;
        match direction {
            "Up" => matches!(state, Up | LeftUp | RightUp),
            "Down" => matches!(state, Down | LeftDown | RightDown),
            "Left" => matches!(state, Left | LeftUp | LeftDown),
            "Right" => matches!(state, Right | RightUp | RightDown),
            _ => false,
        }
    }

    /// A full-range joystick axis's value 0..1 (see `JoystickAxis::
    /// full_range`) - `None` for any other binding, or while its device
    /// hasn't reported that axis (unplugged).
    pub(crate) fn absolute_value(&self, raw: &RawInputState) -> Option<f32> {
        let Binding::JoystickAxis { device, axis, positive, full_range: true, .. } = self else { return None };
        let value = raw.joystick_axes.get(&(device.clone(), *axis)).copied()?;
        let signed = if *positive { value } else { -value };
        Some(((signed + 1.0) * 0.5).clamp(0.0, 1.0))
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
            Binding::JoystickButton { device, button } => {
                if raw.joystick_buttons_down.contains(&(device.clone(), *button)) { 1.0 } else { 0.0 }
            }
            Binding::JoystickAxis { full_range: true, .. } => self.absolute_value(raw).unwrap_or(0.0),
            Binding::JoystickAxis { device, axis, positive, deadzone, .. } => {
                let value = raw.joystick_axes.get(&(device.clone(), *axis)).copied().unwrap_or(0.0);
                let signed = if *positive { value } else { -value };
                // Past the dead zone, rescaled to start from 0 there - no
                // jump as the stick leaves its center.
                if signed > *deadzone { ((signed - deadzone) / (1.0 - deadzone).max(1e-3)).min(1.0) } else { 0.0 }
            }
            Binding::JoystickHat { device, hat, direction } => match raw.joystick_hats.get(&(device.clone(), *hat)) {
                Some(state) if Self::hat_points(*state, direction) => 1.0,
                _ => 0.0,
            },
        }
    }
}

/// What's physically held down right now, aggregated across every connected device -
/// updated incrementally as SDL events arrive in InputSubsystem::update, then used
/// once per frame to recompute every action's strength. Joystick state is keyed by
/// (device name, index).
#[derive(Default)]
pub struct RawInputState {
    pub keys_down: HashSet<String>,
    pub mouse_buttons_down: HashSet<MouseButton>,
    pub controller_buttons_down: HashSet<Button>,
    pub controller_axes: HashMap<Axis, f32>,
    pub joystick_buttons_down: HashSet<(String, u32)>,
    /// -1..1.
    pub joystick_axes: HashMap<(String, u32), f32>,
    pub joystick_hats: HashMap<(String, u32), HatState>,
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

    /// The action's value from a lever bound to it (see `Binding::
    /// absolute_value`) - the first one connected - `None` without one.
    pub(crate) fn absolute_value(&self, raw: &RawInputState) -> Option<f32> {
        self.bindings.iter().find_map(|binding| binding.absolute_value(raw))
    }

    pub fn is_pressed(&self) -> bool { self.pressed }
    pub fn is_just_pressed(&self) -> bool { self.just_pressed }
    pub fn is_just_released(&self) -> bool { self.just_released }
    pub fn strength(&self) -> f32 { self.strength }
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ActionConfig {
    pub action: String,
    pub bindings: Vec<Binding>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_with_axis(value: f32) -> RawInputState {
        let mut raw = RawInputState::default();
        raw.joystick_axes.insert(("Stick".to_owned(), 0), value);
        raw
    }

    #[test]
    fn a_stick_axis_starts_from_zero_past_its_dead_zone() {
        let binding = Binding::JoystickAxis { device: "Stick".into(), axis: 0, positive: true, deadzone: 0.1, full_range: false };
        assert_eq!(binding.strength(&raw_with_axis(0.05)), 0.0);
        assert!(binding.strength(&raw_with_axis(0.11)) < 0.02);
        assert!((binding.strength(&raw_with_axis(1.0)) - 1.0).abs() < 1e-6);
        assert_eq!(binding.strength(&raw_with_axis(-1.0)), 0.0);
    }

    #[test]
    fn a_lever_reads_end_to_end() {
        let lever = Binding::JoystickAxis { device: "Stick".into(), axis: 0, positive: true, deadzone: 0.05, full_range: true };
        assert_eq!(lever.absolute_value(&raw_with_axis(-1.0)), Some(0.0));
        assert_eq!(lever.absolute_value(&raw_with_axis(0.0)), Some(0.5));
        assert_eq!(lever.absolute_value(&raw_with_axis(1.0)), Some(1.0));
        let reversed = lever.clone();
        let Binding::JoystickAxis { device, axis, deadzone, .. } = reversed else { unreachable!() };
        let reversed = Binding::JoystickAxis { device, axis, positive: false, deadzone, full_range: true };
        assert_eq!(reversed.absolute_value(&raw_with_axis(-1.0)), Some(1.0));
        // Unplugged: no value at all.
        assert_eq!(lever.absolute_value(&RawInputState::default()), None);
    }

    #[test]
    fn a_hat_diagonal_counts_both_ways() {
        assert!(Binding::hat_points(HatState::RightUp, "Up"));
        assert!(Binding::hat_points(HatState::RightUp, "Right"));
        assert!(!Binding::hat_points(HatState::RightUp, "Left"));
        assert!(!Binding::hat_points(HatState::Centered, "Up"));
    }

    #[test]
    fn joystick_bindings_round_trip_through_ron() {
        let bindings = vec![
            Binding::Key("w".into()),
            Binding::JoystickButton { device: "VKB-Sim Gladiator NXT EVO R".into(), button: 4 },
            Binding::JoystickAxis { device: "VKB-Sim STECS".into(), axis: 2, positive: false, deadzone: 0.05, full_range: true },
            Binding::JoystickHat { device: "VKB-Sim Gladiator NXT EVO R".into(), hat: 0, direction: "Up".into() },
        ];
        let text = ron::to_string(&bindings).unwrap();
        let back: Vec<Binding> = ron::from_str(&text).unwrap();
        assert_eq!(back, bindings);
    }
}
