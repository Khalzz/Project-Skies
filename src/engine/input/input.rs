use std::cell::RefCell;
use std::collections::HashMap;

use sdl2::controller::GameController;
use sdl2::event::Event;
use sdl2::GameControllerSubsystem;
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
///    - begin_capture() / captured_binding() -> for a future rebinding menu: after
///      begin_capture(), the next key/button pressed or axis pushed past its deadzone
///      is captured instead of dispatched normally, and handed back via captured_binding()
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
    capturing: bool,
    captured_binding: Option<Binding>,
}

impl InputSubsystem {
    pub fn new(settings: &str, controller_subsystem: GameControllerSubsystem) -> Self {
        let settings: InputSettings = ron::from_str(settings).expect("Failed to parse input settings");

        let mut actions = HashMap::new();
        for action_config in settings.actions {
            actions.insert(action_config.action, Action::new(action_config.bindings));
        }

        let mouse = Mouse::new(settings.mouse.x_sensitivity, settings.mouse.y_sensitivity);

        Self {
            actions,
            raw: RawInputState::default(),
            mouse,
            controller_subsystem,
            controllers: HashMap::new(),
            capturing: false,
            captured_binding: None,
        }
    }

    pub fn update(&mut self, event_pump: &mut sdl2::EventPump, _delta_time: f32, debug: bool) {
        self.mouse.reset_rel_x();
        self.mouse.reset_rel_y();
        self.mouse.reset_scroll_y();

        for event in event_pump.poll_iter() {
            if self.capturing {
                if let Some(binding) = Self::event_to_binding(&event) {
                    self.captured_binding = Some(binding);
                    self.capturing = false;
                    continue;
                }
            }

            match event {
                Event::KeyDown { keycode: Some(key), .. } => {
                    self.raw.keys_down.insert(key.to_string().to_uppercase());
                    if debug { println!("Key {} pressed", key); }
                }
                Event::KeyUp { keycode: Some(key), .. } => {
                    self.raw.keys_down.remove(&key.to_string().to_uppercase());
                    if debug { println!("Key {} released", key); }
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
                Event::Quit { .. } => {
                    std::process::exit(0);
                }
                _ => {}
            }
        }

        for action in self.actions.values_mut() {
            action.refresh(&self.raw, PRESS_THRESHOLD);
        }
    }

    fn event_to_binding(event: &Event) -> Option<Binding> {
        match event {
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
    }

    /// Takes whatever binding was captured since begin_capture(), if any yet.
    pub fn captured_binding(&mut self) -> Option<Binding> {
        self.captured_binding.take()
    }

    pub fn rebind(&mut self, action: &str, binding: Binding) {
        if let Some(action) = self.actions.get_mut(action) {
            action.bindings.push(binding);
        }
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
pub fn init(settings: &str, controller_subsystem: GameControllerSubsystem) {
    INPUT.with(|cell| {
        if cell.borrow().is_some() {
            panic!("input subsystem already initialized");
        }
        *cell.borrow_mut() = Some(InputSubsystem::new(settings, controller_subsystem));
    });
}

/// Polls SDL2 events and refreshes action/mouse state - call once per frame.
pub fn update(event_pump: &mut sdl2::EventPump, delta_time: f32, debug: bool) {
    with_input_mut(|input| input.update(event_pump, delta_time, debug));
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
