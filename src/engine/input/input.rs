use std::cell::RefCell;
use std::collections::HashMap;

use sdl2::controller::GameController;
use sdl2::event::Event;
use sdl2::joystick::Joystick;
use sdl2::{GameControllerSubsystem, JoystickSubsystem};
use serde::Deserialize;

use crate::engine::input::action::{Action, ActionConfig, Binding, RawInputState};
use crate::engine::input::mouse::Mouse;

#[derive(Debug, Deserialize)]
struct MouseSettings {
    x_sensitivity: f32,
    y_sensitivity: f32,
}

#[derive(Debug, Deserialize)]
struct InputSettings {
    actions: Vec<ActionConfig>,
    mouse: MouseSettings,
}

/// Raw keyboard typing this frame, in order - for text fields (see
/// `typed_input`), which need the characters typed and the editing keys
/// (backspace, arrows...) rather than game actions.
#[derive(Clone, Debug)]
pub enum TypedInput {
    /// Text typed - already through the keyboard layout (SDL's TextInput).
    Text(String),
    /// A key going down (`repeat`: held, auto-repeating) or up.
    Key { key: sdl2::keyboard::Keycode, pressed: bool, repeat: bool, ctrl: bool, shift: bool },
}

/// Where the player's own rebinds are saved (every change, see
/// `InputSubsystem::save_bindings`) and read back from at start, over
/// settings/input.ron's defaults (built into the game).
const USER_BINDINGS_PATH: &str = "settings/bindings.ron";
const USER_BINDINGS_HEADER: &str = "// Your controls - written by the game whenever a binding is changed in\n// Settings > Controller, read at start over settings/input.ron's defaults.\n// Delete this file to go back to the defaults.\n";

/// A joystick axis moved this far (of its -1..1 travel) from where it was
/// when capturing started is the one being bound.
const CAPTURE_AXIS_TRAVEL: f32 = 0.5;
/// A captured joystick axis's dead zone - flight sticks center precisely.
const JOYSTICK_DEADZONE: f32 = 0.05;

/// A joystick axis's raw value as -1..1.
fn joystick_axis_value(value: i16) -> f32 {
    (value as f32 / i16::MAX as f32).clamp(-1.0, 1.0)
}

/// The player's saved rebinds (see USER_BINDINGS_PATH) - none if there's no
/// file yet; a broken one is reported and ignored.
fn load_user_bindings() -> Vec<ActionConfig> {
    let Ok(text) = std::fs::read_to_string(USER_BINDINGS_PATH) else { return Vec::new() };
    ron::from_str(&text).unwrap_or_else(|error| {
        eprintln!("{USER_BINDINGS_PATH} isn't valid ({error}) - using the default controls");
        Vec::new()
    })
}

/// How far an action's aggregated strength has to go before it counts as "pressed" -
/// buttons/keys report a clean 0.0 or 1.0 so this only really matters for sticks/triggers.
const PRESS_THRESHOLD: f32 = 0.5;

///    # Input Subsystem

///    Action-based input: instead of checking a single key, you check a named action
///    ("jump") that can be bound to any mix of keyboard keys, mouse buttons, controller
///    buttons, and controller axes at once - whichever one is active wins. Bindings are
///    device-agnostic ("any connected controller's A button"), not tied to a specific
///    controller index, since nothing in this project needs per-player device
///    assignment yet; extending Binding::ControllerButton/Axis with an optional device
///    id later is additive if that ever changes.
///
///    Connected controllers are tracked here too (opened on ControllerDeviceAdded,
///    dropped on ControllerDeviceRemoved) so a controller plugged in mid-game works
///    immediately, without a separate one-shot "find a controller at startup" step.
///
///    Functions:
///    - update() -> called once per frame, polls SDL2 events and refreshes action state
///    - is_action_pressed(action) / is_action_just_pressed(action) / is_action_just_released(action)
///    - action_strength(action) -> the raw 0.0..=1.0 value, for analog reads (throttle, stick deflection)
///    - get_axis(negative, positive) -> action_strength(positive) - action_strength(negative)
///    - begin_capture() / captured_binding() / cancel_capture() -> for a rebinding
///      menu (see game::scenes::main_menu::rebind_modal): after begin_capture(), the next
///      key/button pressed or axis pushed past its deadzone is captured instead of
///      dispatched normally, and handed back via captured_binding() (polled once
///      per frame until it returns Some); cancel_capture() aborts without one.
///    - action_bindings(action) / rebind(action, binding) / rebind_at(action,
///      index, binding) / unbind(action, index) -> read/append/replace-in-place/
///      remove an action's bindings at runtime (in-memory only, not persisted
///      back to settings/input.ron).
///
///    settings/input.ron shape:
///    ```ron
///    (
///        actions: [
///            (action: "jump", bindings: [Key("space"), ControllerButton("A")]),
///        ],
///        mouse: (x_sensitivity: 0.5, y_sensitivity: 0.5),
///    )
///    ```
pub struct InputSubsystem {
    actions: HashMap<String, Action>,
    raw: RawInputState,
    pub mouse: Mouse,
    controller_subsystem: GameControllerSubsystem,
    controllers: HashMap<u32, GameController>,
    joystick_subsystem: JoystickSubsystem,
    /// Every connected joystick that isn't a gamepad (those are
    /// `controllers`) - HOTAS sticks, throttles, pedals... - by instance id.
    /// Its name is what bindings know it by.
    joysticks: HashMap<u32, (Joystick, String)>,
    capturing: bool,
    captured_binding: Option<Binding>,
    /// Every action's bindings as settings/input.ron has them - what
    /// save_bindings compares against, so only the player's changes are
    /// saved.
    default_bindings: HashMap<String, Vec<Binding>>,
    /// Where every joystick axis was when capturing started - an axis only
    /// counts as "moved" once it's gone well away from there (a throttle
    /// lever can rest at either end).
    capture_baseline: HashMap<(String, u32), f32>,
    // This frame's typing - see TypedInput.
    typed: Vec<TypedInput>,
}

impl InputSubsystem {
    pub fn new(settings: &str, controller_subsystem: GameControllerSubsystem, joystick_subsystem: JoystickSubsystem) -> Self {
        let settings: InputSettings = ron::from_str(settings).expect("Failed to parse input settings");

        let mut actions = HashMap::new();
        let mut default_bindings = HashMap::new();
        for action_config in settings.actions {
            default_bindings.insert(action_config.action.clone(), action_config.bindings.clone());
            actions.insert(action_config.action, Action::new(action_config.bindings));
        }
        // The player's own rebinds, over the defaults - see save_bindings.
        for action_config in load_user_bindings() {
            if let Some(action) = actions.get_mut(&action_config.action) {
                action.bindings = action_config.bindings;
            }
        }

        let mouse = Mouse::new(settings.mouse.x_sensitivity, settings.mouse.y_sensitivity);

        Self {
            actions,
            raw: RawInputState::default(),
            mouse,
            controller_subsystem,
            controllers: HashMap::new(),
            joystick_subsystem,
            joysticks: HashMap::new(),
            capturing: false,
            captured_binding: None,
            capture_baseline: HashMap::new(),
            default_bindings,
            typed: Vec::new(),
        }
    }

    // Returns true if an Event::Quit (window close button, Cmd+Q, etc.) was
    // seen this poll - App::run's own loop is what actually decides what to
    // do with that (break its loop gracefully, sending a physics shutdown
    // command first - see that loop's own comment on why this used to be a
    // raw std::process::exit(0) here instead, and why that was wrong).
    pub fn update(&mut self, event_pump: &mut sdl2::EventPump, _delta_time: f32, debug: bool) -> bool {
        self.mouse.reset_rel_x();
        self.mouse.reset_rel_y();
        self.mouse.reset_scroll_y();
        self.typed.clear();

        let mut quit_requested = false;

        for event in event_pump.poll_iter() {
            if self.capturing {
                if let Some(binding) = self.event_to_binding(&event) {
                    self.captured_binding = Some(binding);
                    self.capturing = false;
                    continue;
                }
            }

            match event {
                Event::KeyDown { keycode: Some(key), keymod, repeat, .. } => {
                    self.typed.push(Self::typed_key(key, keymod, true, repeat));
                    self.raw.keys_down.insert(key.to_string().to_uppercase());
                    if debug { println!("Key {} pressed", key); }
                }
                Event::KeyUp { keycode: Some(key), keymod, .. } => {
                    self.typed.push(Self::typed_key(key, keymod, false, false));
                    self.raw.keys_down.remove(&key.to_string().to_uppercase());
                    if debug { println!("Key {} released", key); }
                }
                Event::TextInput { text, .. } => {
                    self.typed.push(TypedInput::Text(text));
                }
                Event::MouseButtonDown { mouse_btn, .. } => {
                    self.raw.mouse_buttons_down.insert(mouse_btn);
                }
                Event::MouseButtonUp { mouse_btn, .. } => {
                    self.raw.mouse_buttons_down.remove(&mouse_btn);
                }
                Event::MouseMotion { xrel, yrel, x, y, .. } => {
                    self.mouse.add_rel_x(xrel);
                    self.mouse.add_rel_y(yrel);

                    // For camera control
                    self.mouse.set_x(self.mouse.get_x() + xrel);
                    self.mouse.set_y((self.mouse.get_y() + yrel).clamp(-170, 170));

                    // For buttons
                    self.mouse.set_raw_x(x);
                    self.mouse.set_raw_y(y);
                }
                Event::MouseWheel { y, .. } => {
                    self.mouse.add_scroll_y(y as f32);
                }
                Event::ControllerButtonDown { button, .. } => {
                    self.raw.controller_buttons_down.insert(button);
                }
                Event::ControllerButtonUp { button, .. } => {
                    self.raw.controller_buttons_down.remove(&button);
                }
                Event::ControllerAxisMotion { axis, value, .. } => {
                    self.raw.controller_axes.insert(axis, value as f32 / i16::MAX as f32);
                }
                Event::ControllerDeviceAdded { which, .. } => {
                    // `which` is a device index here (not an instance id).
                    if let Ok(controller) = self.controller_subsystem.open(which) {
                        let instance_id = controller.instance_id();
                        println!("Controller connected: {}", controller.name());
                        self.controllers.insert(instance_id, controller);
                    }
                }
                Event::ControllerDeviceRemoved { which, .. } => {
                    // `which` is the instance id here (not a device index).
                    self.controllers.remove(&which);
                }
                // Joysticks that aren't gamepads - a gamepad is a joystick
                // too, but it's handled as a controller above (and its joystick
                // events are ignored below, not in `joysticks`).
                Event::JoyDeviceAdded { which, .. } => {
                    // `which` is a device index here.
                    if !self.controller_subsystem.is_game_controller(which) {
                        self.open_joystick(which);
                    }
                }
                Event::JoyDeviceRemoved { which, .. } => {
                    // `which` is the instance id here.
                    if let Some((_, name)) = self.joysticks.remove(&which) {
                        println!("Joystick disconnected: {name}");
                        self.raw.joystick_buttons_down.retain(|(device, _)| *device != name);
                        self.raw.joystick_axes.retain(|(device, _), _| *device != name);
                        self.raw.joystick_hats.retain(|(device, _), _| *device != name);
                    }
                }
                Event::JoyButtonDown { which, button_idx, .. } => {
                    if let Some(name) = self.joystick_name(which) {
                        self.raw.joystick_buttons_down.insert((name, button_idx as u32));
                    }
                }
                Event::JoyButtonUp { which, button_idx, .. } => {
                    if let Some(name) = self.joystick_name(which) {
                        self.raw.joystick_buttons_down.remove(&(name, button_idx as u32));
                    }
                }
                Event::JoyAxisMotion { which, axis_idx, value, .. } => {
                    if let Some(name) = self.joystick_name(which) {
                        self.raw.joystick_axes.insert((name, axis_idx as u32), joystick_axis_value(value));
                    }
                }
                Event::JoyHatMotion { which, hat_idx, state, .. } => {
                    if let Some(name) = self.joystick_name(which) {
                        self.raw.joystick_hats.insert((name, hat_idx as u32), state);
                    }
                }
                Event::Quit { .. } => {
                    quit_requested = true;
                }
                _ => {}
            }
        }

        for action in self.actions.values_mut() {
            action.refresh(&self.raw, PRESS_THRESHOLD);
        }

        quit_requested
    }

    fn typed_key(key: sdl2::keyboard::Keycode, keymod: sdl2::keyboard::Mod, pressed: bool, repeat: bool) -> TypedInput {
        use sdl2::keyboard::Mod;
        TypedInput::Key {
            key,
            pressed,
            repeat,
            ctrl: keymod.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD),
            shift: keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD),
        }
    }

    /// Opens the joystick at device index `index` and reads where its axes
    /// and hats are right now (SDL only reports them again once they move).
    fn open_joystick(&mut self, index: u32) {
        let Ok(joystick) = self.joystick_subsystem.open(index) else { return };
        let name = joystick.name();
        println!("Joystick connected: {name} ({} axes, {} buttons, {} hats)", joystick.num_axes(), joystick.num_buttons(), joystick.num_hats());
        for axis in 0..joystick.num_axes() {
            if let Ok(value) = joystick.axis(axis) {
                self.raw.joystick_axes.insert((name.clone(), axis), joystick_axis_value(value));
            }
        }
        for hat in 0..joystick.num_hats() {
            if let Ok(state) = joystick.hat(hat) {
                self.raw.joystick_hats.insert((name.clone(), hat), state);
            }
        }
        self.joysticks.insert(joystick.instance_id(), (joystick, name));
    }

    /// The name of the joystick with instance id `which` - `None` for one
    /// that isn't open (a gamepad's own joystick events).
    fn joystick_name(&self, which: u32) -> Option<String> {
        self.joysticks.get(&which).map(|(_, name)| name.clone())
    }

    fn event_to_binding(&self, event: &Event) -> Option<Binding> {
        match event {
            Event::JoyButtonDown { which, button_idx, .. } => {
                Some(Binding::JoystickButton { device: self.joystick_name(*which)?, button: *button_idx as u32 })
            }
            Event::JoyHatMotion { which, hat_idx, state, .. } => {
                let direction = ["Up", "Down", "Left", "Right"].into_iter().find(|direction| Binding::hat_points(*state, direction))?;
                Some(Binding::JoystickHat { device: self.joystick_name(*which)?, hat: *hat_idx as u32, direction: direction.to_owned() })
            }
            Event::JoyAxisMotion { which, axis_idx, value, .. } => {
                // Moved well away from where it was when capturing started -
                // that way is "positive".
                let device = self.joystick_name(*which)?;
                let axis = *axis_idx as u32;
                let start = self.capture_baseline.get(&(device.clone(), axis)).copied().unwrap_or(0.0);
                let moved = joystick_axis_value(*value) - start;
                (moved.abs() > CAPTURE_AXIS_TRAVEL).then(|| Binding::JoystickAxis {
                    device,
                    axis,
                    positive: moved > 0.0,
                    deadzone: JOYSTICK_DEADZONE,
                    full_range: false,
                })
            }
            Event::KeyDown { keycode: Some(key), .. } => Some(Binding::Key(key.to_string())),
            Event::MouseButtonDown { mouse_btn, .. } => Some(Binding::MouseButton(format!("{:?}", mouse_btn))),
            Event::ControllerButtonDown { button, .. } => Some(Binding::ControllerButton(format!("{:?}", button))),
            Event::ControllerAxisMotion { axis, value, .. } => {
                let normalized = *value as f32 / i16::MAX as f32;
                if normalized.abs() > 0.5 {
                    Some(Binding::ControllerAxis { axis: format!("{:?}", axis), positive: normalized > 0.0, deadzone: 0.25 })
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub fn is_action_pressed(&self, action: &str) -> bool {
        match self.actions.get(action) {
            Some(action) => action.is_pressed(),
            None => { println!("Action {} not found", action); false }
        }
    }

    pub fn is_action_just_pressed(&self, action: &str) -> bool {
        match self.actions.get(action) {
            Some(action) => action.is_just_pressed(),
            None => { println!("Action {} not found", action); false }
        }
    }

    pub fn is_action_just_released(&self, action: &str) -> bool {
        match self.actions.get(action) {
            Some(action) => action.is_just_released(),
            None => { println!("Action {} not found", action); false }
        }
    }

    pub fn action_strength(&self, action: &str) -> f32 {
        self.actions.get(action).map_or(0.0, Action::strength)
    }

    pub fn get_axis(&self, negative: &str, positive: &str) -> f32 {
        self.action_strength(positive) - self.action_strength(negative)
    }

    /// Starts listening for the next physical input, for a rebinding menu - see
    /// captured_binding().
    pub fn begin_capture(&mut self) {
        self.capturing = true;
        self.captured_binding = None;
        self.capture_baseline = self.raw.joystick_axes.clone();
    }

    /// The value of a lever bound to `action` (see `Binding::JoystickAxis::
    /// full_range`) - 0..1 end to end - while one's connected.
    pub fn absolute_value(&self, action: &str) -> Option<f32> {
        self.actions.get(action)?.absolute_value(&self.raw)
    }

    /// The bindings of every action the player has changed from the defaults,
    /// to `settings/bindings.ron` - only those, so later changes to the
    /// defaults still reach every other action.
    fn save_bindings(&self) {
        let mut actions: Vec<ActionConfig> = self.actions.iter()
            .filter(|(name, action)| self.default_bindings.get(*name) != Some(&action.bindings))
            .map(|(name, action)| ActionConfig { action: name.clone(), bindings: action.bindings.clone() })
            .collect();
        actions.sort_by(|a, b| a.action.cmp(&b.action));
        let text = match ron::ser::to_string_pretty(&actions, ron::ser::PrettyConfig::new().indentor("    ")) {
            Ok(text) => text,
            Err(error) => {
                eprintln!("Couldn't write the bindings out: {error}");
                return;
            }
        };
        if let Err(error) = std::fs::write(USER_BINDINGS_PATH, format!("{USER_BINDINGS_HEADER}{text}\n")) {
            eprintln!("Couldn't save the bindings to {USER_BINDINGS_PATH}: {error}");
        }
    }

    /// Takes whatever binding was captured since begin_capture(), if any yet.
    pub fn captured_binding(&mut self) -> Option<Binding> {
        self.captured_binding.take()
    }

    pub fn rebind(&mut self, action: &str, binding: Binding) {
        if let Some(action) = self.actions.get_mut(action) {
            action.bindings.push(binding);
        }
        self.save_bindings();
    }

    /// Like rebind, but replaces one binding in place by its index (see
    /// action_bindings) instead of appending - a no-op if the index is out of
    /// range, so a stale index from a UI that hasn't refreshed yet can't panic.
    pub fn rebind_at(&mut self, action: &str, index: usize, binding: Binding) {
        if let Some(action) = self.actions.get_mut(action) {
            if index < action.bindings.len() {
                action.bindings[index] = binding;
            }
        }
        self.save_bindings();
    }

    /// Cloned out (not a reference) since a rebinding menu needs to render this
    /// list into UI nodes it owns independently of the input subsystem's own
    /// lifetime/borrow rules.
    pub fn action_bindings(&self, action: &str) -> Vec<Binding> {
        self.actions.get(action).map(|a| a.bindings.clone()).unwrap_or_default()
    }

    /// Removes one binding by its index in that action's list (see
    /// action_bindings) - a no-op if the index is out of range, so a stale index
    /// from a UI that hasn't refreshed yet can't panic.
    pub fn unbind(&mut self, action: &str, index: usize) {
        if let Some(action) = self.actions.get_mut(action) {
            if index < action.bindings.len() {
                action.bindings.remove(index);
            }
        }
        self.save_bindings();
    }

    /// Aborts an in-progress begin_capture() without producing a binding - e.g. a
    /// rebinding menu's own "cancel" button, or closing the menu while still
    /// waiting on input.
    pub fn cancel_capture(&mut self) {
        self.capturing = false;
        self.captured_binding = None;
    }
}

// Global input singleton - lets any code query input state (input::is_action_pressed("jump"))
// without threading a &InputSubsystem parameter through every function that needs it,
// the same way Godot's Input singleton works. thread_local rather than a cross-thread
// static: GameControllerSubsystem/GameController wrap raw SDL pointers that aren't
// Send/Sync, and SDL controller/event handling is only ever valid on the thread that
// initialized it anyway (the main thread here) - a thread_local is the honest fit,
// not a workaround, since nothing in this project touches input off the main thread.
thread_local! {
    static INPUT: RefCell<Option<InputSubsystem>> = RefCell::new(None);
}

fn with_input<T>(f: impl FnOnce(&InputSubsystem) -> T) -> T {
    INPUT.with(|cell| {
        let cell = cell.borrow();
        let input = cell.as_ref().expect("input::init() must be called before using the input subsystem");
        f(input)
    })
}

fn with_input_mut<T>(f: impl FnOnce(&mut InputSubsystem) -> T) -> T {
    INPUT.with(|cell| {
        let mut cell = cell.borrow_mut();
        let input = cell.as_mut().expect("input::init() must be called before using the input subsystem");
        f(input)
    })
}

/// Initializes the global input singleton from settings/input.ron - call once, before
/// the first update()/is_action_pressed() call (see App::run), on the main thread.
pub fn init(settings: &str, controller_subsystem: GameControllerSubsystem, joystick_subsystem: JoystickSubsystem) {
    INPUT.with(|cell| {
        if cell.borrow().is_some() {
            panic!("input subsystem already initialized");
        }
        *cell.borrow_mut() = Some(InputSubsystem::new(settings, controller_subsystem, joystick_subsystem));
    });
}

/// The value of a lever bound to `action` - 0..1 end to end - while one's
/// connected (see `Binding::JoystickAxis::full_range`). For an action read as
/// a position (a throttle) rather than pressed/not.
pub fn absolute_value(action: &str) -> Option<f32> {
    with_input(|input| input.absolute_value(action))
}

/// Polls SDL2 events and refreshes action/mouse state - call once per frame.
/// Returns true if the OS/window manager requested a close this poll - see
/// InputSubsystem::update's own doc comment.
pub fn update(event_pump: &mut sdl2::EventPump, delta_time: f32, debug: bool) -> bool {
    with_input_mut(|input| input.update(event_pump, delta_time, debug))
}

pub fn is_action_pressed(action: &str) -> bool {
    with_input(|input| input.is_action_pressed(action))
}

pub fn is_action_just_pressed(action: &str) -> bool {
    with_input(|input| input.is_action_just_pressed(action))
}

pub fn is_action_just_released(action: &str) -> bool {
    with_input(|input| input.is_action_just_released(action))
}

pub fn action_strength(action: &str) -> f32 {
    with_input(|input| input.action_strength(action))
}

pub fn get_axis(negative: &str, positive: &str) -> f32 {
    with_input(|input| input.get_axis(negative, positive))
}

pub fn mouse_rel_x() -> i32 {
    with_input(|input| input.mouse.get_rel_x())
}

pub fn mouse_rel_y() -> i32 {
    with_input(|input| input.mouse.get_rel_y())
}

// Absolute, window-relative, top-left-origin pixels - the same coordinate space
// UiTransform.rect is already in, so this is directly usable for UI hit-testing
// (see UiNode::on_hover) with no conversion.
pub fn mouse_x() -> i32 {
    with_input(|input| input.mouse.get_raw_x())
}

pub fn mouse_y() -> i32 {
    with_input(|input| input.mouse.get_raw_y())
}

pub fn mouse_scroll_y() -> f32 {
    with_input(|input| input.mouse.get_scroll_y())
}

// Raw left-button-down state, distinct from any bound action - for feeding
// egui::RawInput (see engine::rendering::egui_overlay), which needs the
// literal button state rather than a game action.
pub fn mouse_left_button_down() -> bool {
    with_input(|input| input.raw.mouse_buttons_down.contains(&sdl2::mouse::MouseButton::Left))
}

// Raw right-button-down state - for feeding egui (see mouse_left_button_down).
pub fn mouse_right_button_down() -> bool {
    with_input(|input| input.raw.mouse_buttons_down.contains(&sdl2::mouse::MouseButton::Right))
}

// Whether either Shift / Ctrl key is held right now - for feeding egui its
// modifiers (see mouse_left_button_down).
pub fn shift_down() -> bool {
    with_input(|input| input.raw.keys_down.contains("LEFT SHIFT") || input.raw.keys_down.contains("RIGHT SHIFT"))
}

pub fn ctrl_down() -> bool {
    with_input(|input| input.raw.keys_down.contains("LEFT CTRL") || input.raw.keys_down.contains("RIGHT CTRL"))
}

// Raw middle-button-down state - for feeding egui (see mouse_left_button_down).
pub fn mouse_middle_button_down() -> bool {
    with_input(|input| input.raw.mouse_buttons_down.contains(&sdl2::mouse::MouseButton::Middle))
}

/// This frame's raw typing, in order - see `TypedInput`.
pub fn typed_input() -> Vec<TypedInput> {
    with_input(|input| input.typed.clone())
}

pub fn mouse_sensitivity() -> (f32, f32) {
    with_input(|input| input.mouse.get_sensitivity())
}

pub fn begin_capture() {
    with_input_mut(|input| input.begin_capture());
}

pub fn captured_binding() -> Option<Binding> {
    with_input_mut(|input| input.captured_binding())
}

pub fn rebind(action: &str, binding: Binding) {
    with_input_mut(|input| input.rebind(action, binding));
}

pub fn rebind_at(action: &str, index: usize, binding: Binding) {
    with_input_mut(|input| input.rebind_at(action, index, binding));
}

pub fn action_bindings(action: &str) -> Vec<Binding> {
    with_input(|input| input.action_bindings(action))
}

pub fn unbind(action: &str, index: usize) {
    with_input_mut(|input| input.unbind(action, index));
}

pub fn cancel_capture() {
    with_input_mut(|input| input.cancel_capture());
}
