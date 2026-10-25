//! One physical key, button or axis direction on one device - what
//! `input::bind` binds an action to, what `input::is_pressed` checks, and
//! what `input::read_input` hands back:
//!
//!     input::bind("jump", InputSource::key(Key::Space));
//!     input::bind("jump", InputSource::controller(Button::A));
//!     input::bind("left", InputSource::controller_axis(Axis::LeftX, false));
//!
//!     if let Some(source) = input::read_input() {
//!         println!("{source}");              // "Space", "Controller A", "VKB-Sim Gladiator: Button 3"
//!         input::bind("fire", source);
//!     }
//!
//! A source can also be an action - `InputSource::action("jump")`, or just
//! `"jump"` wherever a source is taken (`input::get_axis("Left", "Right")`,
//! `input::is_pressed("jump")`): whatever is bound to it.
//!
//! Keys, buttons and axes are named by the types re-exported here (`Key`,
//! `Button`, `Axis`, `MouseButton` - SDL's), so a misspelled one doesn't
//! compile. Joysticks have no such list - their buttons and axes are
//! numbered, by device - so theirs come from `read_input`.
//!
//! Each source also has a name, by device:
//! - `Keyboard`: SDL's key name - "Space", "A", "Left", "Left Shift";
//! - `Mouse`: "Left", "Right", "Middle", "X1", "X2";
//! - `Controller`: a button - "A", "Start", "DPadUp", "LeftShoulder"... - or
//!   an axis pushed one way - "LeftX+", "LeftY-", "TriggerRight+";
//! - `Joystick` (by its device's name): "Button 3", "Axis 2+", "Hat 1 Up" -
//!   counted from 1.

use std::fmt;

pub use sdl2::controller::{Axis, Button};
pub use sdl2::keyboard::Keycode as Key;
pub use sdl2::mouse::MouseButton;

use crate::input::action::Binding;

/// A controller axis bound from a source (see `InputSource::to_binding`)
/// counts from this far along.
const CONTROLLER_AXIS_DEADZONE: f32 = 0.25;

/// What an `InputSource` is on.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum InputDevice {
    Keyboard,
    Mouse,
    /// Any connected gamepad.
    Controller,
    /// A joystick that isn't a gamepad (a HOTAS stick, a throttle, pedals),
    /// by the name SDL reports for it.
    Joystick(String),
    /// Not a device: an action, by name - whatever is bound to it (see
    /// `input::bind`).
    Action,
}

/// One key, button or axis direction - see the module doc.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InputSource {
    pub device: InputDevice,
    pub name: String,
}

impl InputSource {
    pub fn key(key: Key) -> Self {
        Self::named(InputDevice::Keyboard, key.to_string())
    }

    pub fn mouse(button: MouseButton) -> Self {
        Self::named(InputDevice::Mouse, format!("{button:?}"))
    }

    pub fn controller(button: Button) -> Self {
        Self::named(InputDevice::Controller, format!("{button:?}"))
    }

    /// A controller's `axis` pushed one way - toward + if `positive`.
    pub fn controller_axis(axis: Axis, positive: bool) -> Self {
        Self::named(InputDevice::Controller, format!("{axis:?}{}", if positive { "+" } else { "-" }))
    }

    /// The action `name` - whatever is bound to it. A plain `"name"` turns
    /// into this wherever a source is taken.
    pub fn action(name: impl Into<String>) -> Self {
        Self::named(InputDevice::Action, name)
    }

    pub(crate) fn named(device: InputDevice, name: impl Into<String>) -> Self {
        Self { device, name: name.into() }
    }

    /// A source from text - for names that come from a file or a menu: a
    /// key's name ("Space"), or "Mouse <button>" / "Controller <button or
    /// axis>" ("Mouse Left", "Controller A", "Controller LeftX+"), the way
    /// `Display` writes them - or "Action <name>". Joysticks aren't read this
    /// way - their names vary. An error for a name that isn't a key or button.
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if let Some(action) = text.strip_prefix("Action ") {
            return Ok(Self::action(action.trim()));
        }
        let source = if let Some(button) = text.strip_prefix("Mouse ") {
            Self::named(InputDevice::Mouse, button.trim())
        } else if let Some(name) = text.strip_prefix("Controller ") {
            Self::named(InputDevice::Controller, name.trim())
        } else {
            Self::named(InputDevice::Keyboard, text)
        };
        match source.to_binding() {
            Some(_) => Ok(source),
            None => Err(format!("'{text}' isn't a key or a button")),
        }
    }

    /// As an action binding - None for a name its device doesn't have, or
    /// an action (an action isn't bound to another).
    pub fn to_binding(&self) -> Option<Binding> {
        match &self.device {
            InputDevice::Action => None,
            InputDevice::Keyboard => Key::from_name(&self.name).map(|key| Binding::Key(key.to_string())),
            InputDevice::Mouse => Binding::parse_mouse_button(&self.name).map(|_| Binding::MouseButton(self.name.clone())),
            InputDevice::Controller => {
                if let Some((axis, positive)) = split_direction(&self.name) {
                    Binding::parse_controller_axis(axis)
                        .map(|_| Binding::ControllerAxis { axis: axis.to_owned(), positive, deadzone: CONTROLLER_AXIS_DEADZONE })
                } else {
                    Binding::parse_controller_button(&self.name).map(|_| Binding::ControllerButton(self.name.clone()))
                }
            }
            InputDevice::Joystick(device) => {
                let device = device.clone();
                let words: Vec<&str> = self.name.split_whitespace().collect();
                match words.as_slice() {
                    ["Button", number] => Some(Binding::JoystickButton { device, button: number.parse::<u32>().ok()?.checked_sub(1)? }),
                    ["Axis", number] => {
                        let (number, positive) = split_direction(number)?;
                        let axis = number.parse::<u32>().ok()?.checked_sub(1)?;
                        Some(Binding::JoystickAxis { device, axis, positive, deadzone: super::input::JOYSTICK_DEADZONE, full_range: false })
                    }
                    ["Hat", number, direction] if ["Up", "Down", "Left", "Right"].contains(direction) => {
                        Some(Binding::JoystickHat { device, hat: number.parse::<u32>().ok()?.checked_sub(1)?, direction: (*direction).to_owned() })
                    }
                    _ => None,
                }
            }
        }
    }

    /// The source `binding` is bound to.
    pub fn from_binding(binding: &Binding) -> Self {
        let sign = |positive: bool| if positive { "+" } else { "-" };
        match binding {
            Binding::Key(key) => Self::named(InputDevice::Keyboard, key.clone()),
            Binding::MouseButton(button) => Self::named(InputDevice::Mouse, button.clone()),
            Binding::ControllerButton(button) => Self::named(InputDevice::Controller, button.clone()),
            Binding::ControllerAxis { axis, positive, .. } => Self::named(InputDevice::Controller, format!("{axis}{}", sign(*positive))),
            Binding::JoystickButton { device, button } => Self::named(InputDevice::Joystick(device.clone()), format!("Button {}", button + 1)),
            Binding::JoystickAxis { device, axis, positive, .. } => Self::named(InputDevice::Joystick(device.clone()), format!("Axis {}{}", axis + 1, sign(*positive))),
            Binding::JoystickHat { device, hat, direction } => Self::named(InputDevice::Joystick(device.clone()), format!("Hat {} {direction}", hat + 1)),
        }
    }
}

/// "Space", "Mouse Left", "Controller A", "<joystick>: Button 3" - for
/// keyboard, mouse and controller, text `InputSource::parse` reads back.
impl fmt::Display for InputSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.device {
            InputDevice::Keyboard => write!(f, "{}", self.name),
            InputDevice::Mouse => write!(f, "Mouse {}", self.name),
            InputDevice::Controller => write!(f, "Controller {}", self.name),
            InputDevice::Joystick(device) => write!(f, "{device}: {}", self.name),
            InputDevice::Action => write!(f, "Action {}", self.name),
        }
    }
}

/// A plain name is an action: `get_axis("Left", "Right")`.
impl From<&str> for InputSource {
    fn from(action: &str) -> Self {
        Self::action(action)
    }
}

impl From<String> for InputSource {
    fn from(action: String) -> Self {
        Self::action(action)
    }
}

impl From<&InputSource> for InputSource {
    fn from(source: &InputSource) -> Self {
        source.clone()
    }
}

/// "LeftX+" to ("LeftX", true) - None without a trailing + or -.
fn split_direction(name: &str) -> Option<(&str, bool)> {
    if let Some(axis) = name.strip_suffix('+') {
        Some((axis, true))
    } else {
        name.strip_suffix('-').map(|axis| (axis, false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_sources_are_named_like_bindings() {
        assert_eq!(InputSource::key(Key::Space).to_binding(), Some(Binding::Key("Space".into())));
        assert_eq!(InputSource::key(Key::LShift).name, "Left Shift");
        assert_eq!(InputSource::mouse(MouseButton::X1).to_binding(), Some(Binding::MouseButton("X1".into())));
        assert_eq!(InputSource::controller(Button::DPadUp).to_binding(), Some(Binding::ControllerButton("DPadUp".into())));
        assert_eq!(InputSource::controller_axis(Axis::LeftX, false).to_binding(), Some(Binding::ControllerAxis { axis: "LeftX".into(), positive: false, deadzone: CONTROLLER_AXIS_DEADZONE }));
        assert_eq!(InputSource::controller_axis(Axis::TriggerRight, true).name, "TriggerRight+");
    }

    #[test]
    fn text_names_each_device() {
        assert_eq!(InputSource::parse("Space").unwrap(), InputSource::key(Key::Space));
        assert_eq!(InputSource::parse("Mouse Left").unwrap(), InputSource::mouse(MouseButton::Left));
        assert_eq!(InputSource::parse("Controller A").unwrap(), InputSource::controller(Button::A));
        assert_eq!(InputSource::parse("Controller LeftX-").unwrap(), InputSource::controller_axis(Axis::LeftX, false));
        assert!(InputSource::parse("Spacebarr").is_err());
        assert!(InputSource::parse("Controller Z").is_err());
    }

    #[test]
    fn a_source_round_trips_through_its_binding() {
        let joystick = InputDevice::Joystick("VKB-Sim Gladiator".into());
        for source in [
            InputSource::key(Key::LShift),
            InputSource::mouse(MouseButton::X1),
            InputSource::controller(Button::DPadUp),
            InputSource::controller_axis(Axis::TriggerRight, true),
            InputSource::named(joystick.clone(), "Button 3"),
            InputSource::named(joystick.clone(), "Axis 2-"),
            InputSource::named(joystick, "Hat 1 Up"),
        ] {
            let binding = source.to_binding().unwrap_or_else(|| panic!("{source}"));
            assert_eq!(InputSource::from_binding(&binding), source);
        }
    }

    #[test]
    fn a_plain_name_is_an_action() {
        let source: InputSource = "jump".into();
        assert_eq!(source, InputSource::action("jump"));
        assert_eq!(source.device, InputDevice::Action);
        assert_eq!(source.to_binding(), None);
    }

    #[test]
    fn display_reads_back() {
        for text in ["Space", "Mouse Right", "Controller LeftShoulder", "Controller RightY+", "Action jump"] {
            assert_eq!(InputSource::parse(text).unwrap().to_string(), text);
        }
    }
}
