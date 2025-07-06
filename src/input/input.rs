use std::collections::HashMap;
use crate::input::pressable::Pressable;
use sdl2::event::Event;
use serde::Deserialize;
use crate::input::mouse::Mouse;

#[derive(Debug, Deserialize)]
struct KeyBinding {
    label: String,
    keys: Vec<String>,
}


#[derive(Debug, Deserialize)]
struct MouseSettings {
    x_sensitivity: f32,
    y_sensitivity: f32,
}

#[derive(Debug, Deserialize)]
struct InputSettings {
    keys: Vec<KeyBinding>,
    mouse: MouseSettings,
}

///    # Input Subsystem

///    The input subsystem is a centralized way of handling input, based on this we will be able to be more
///    flexible in the way we define inputs, what should they do and how to access them, also allowing their use
///    on multi thread applications.

///    Here we have this functions:
///    - Update() -> This will be called every frame to update the state of the input
///    - IsPressed() -> This will return true if the key is pressed
///    - IsJustPressed() -> This will return true if the key is just pressed
///    - IsReleased() -> This will return true if the key is released

///    The last 3 functions check for a label created on the Input folder in the settings/input.ron file

///    The structure of the input.ron file is:
/// 
///    ```json
///    {
///        "keys": [
///            {
///                "label": "forward",
///                "keys": ["w", "up"]
///            },
///            {
///                "label": "left",
///                "keys": ["a", "left"]
///            },
///        ]
///    }
///    ```

// TODO: Add a axis "method" this will let me add axis dfrom joysticks or take keyboard input and turn it into a value that goes from -1 to 1
pub struct InputSubsystem {
    pub keys: HashMap<String, Pressable>,
    pub mouse: Mouse,
}

impl InputSubsystem {
    pub fn new(settings: &str) -> Self {
        // TODO: create a new file if the settings are not found
        let settings: InputSettings = ron::from_str(settings).expect("Failed to parse input settings");

        let mut keys = HashMap::new();

        // Parse the settings and create Pressable instances
        for key_binding in settings.keys {
            let key_refs: Vec<&str> = key_binding.keys.iter().map(|s| s.as_str()).collect();
            keys.insert(key_binding.label.clone(), Pressable::new(Some(key_refs)));
        }

        let mouse = Mouse::new(settings.mouse.x_sensitivity, settings.mouse.y_sensitivity);

        Self { keys, mouse }
    }

    pub fn update(&mut self, event_pump: &mut sdl2::EventPump, delta_time: f32, debug: bool) {
        self.reset_release_states();
        self.reset_just_pressed_states();
        self.reset_mouse_relative_movement();

        for event in event_pump.poll_iter() {
            match event {
                Event::KeyDown { keycode, .. } => {
                    for (_, pressable) in self.keys.iter_mut() {
                        if let Some(key) = keycode {
                            match &pressable.keys {
                                Some(keys) => {
                                    if keys.contains(&key.to_string().to_uppercase()) {
                                        if !pressable.is_pressed() {
                                            pressable.set_just_pressed(true);
                                        }
        
                                        pressable.set_pressed(true, delta_time);
        
        
                                        if debug {
                                            println!("Key {} pressed", key);
                                        }
                                    }
                                }
                                None => {
                                    println!("Key {} can't have a null identifier", key);
                                }
                            }
                        }
                    }
                }
                Event::KeyUp { keycode, .. } => {
                    for (_, pressable) in self.keys.iter_mut() {
                        if let Some(key) = keycode {
                            match &pressable.keys {
                                Some(keys) => {
                                    if keys.contains(&key.to_string().to_uppercase()) {
                                        pressable.set_pressed(false, delta_time);
                                        if debug {
                                            println!("Key {} released", key);
                                        }
                                    }
                                },
                                None => {
                                    println!("Key {} can't have a null identifier", key);
                                }
                            }
                            
                        }
                    }
                }
                Event::MouseMotion { xrel, yrel, x, y, .. } => {
                    self.mouse.set_rel_x(xrel);
                    self.mouse.set_rel_y(yrel);

                    // For camera control
                    self.mouse.set_x(self.mouse.get_x() + xrel);
                    self.mouse.set_y((self.mouse.get_y() + yrel).clamp(-170, 170));
                    
                    // For buttons
                    self.mouse.set_raw_x(x);
                    self.mouse.set_raw_y(y);
                }
                Event::Quit { .. } => {
                    std::process::exit(0);
                }
                _ => {}
            }
        }
    }

    pub fn reset_release_states(&mut self) {
        for (_, pressable) in self.keys.iter_mut() {
            pressable.set_released(false);
        }
    }

    pub fn reset_just_pressed_states(&mut self) {
        for (_, pressable) in self.keys.iter_mut() {
            pressable.set_just_pressed(false);
        }
    }

    pub fn reset_input_states(&mut self) {
        for (_, pressable) in self.keys.iter_mut() {
            pressable.set_released(false);
            pressable.set_just_pressed(false);
        }
    }

    pub fn is_pressed(&self, key: &str) -> bool {
        match self.keys.get(key) {
            Some(pressable) => pressable.is_pressed(),
            None => {
                println!("Key {} not found", key);
                false
            }
        }
    }

    pub fn is_just_pressed(&self, key: &str) -> bool {
        match self.keys.get(key) {
            Some(pressable) => pressable.is_just_pressed(),
            None => {
                println!("Key {} not found", key);
                false
            }
        }
    }

    pub fn is_released(&self, key: &str) -> bool {
        match self.keys.get(key) {
            Some(pressable) => pressable.is_released(),
            None => {
                println!("Key {} not found", key);
                false
            }
        }
    }

    pub fn reset_mouse_relative_movement(&mut self) {
        self.mouse.reset_rel_x();
        self.mouse.reset_rel_y();
    }
}
