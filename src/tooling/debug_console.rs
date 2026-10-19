use std::sync::Mutex;

/// Global debug console messages storage
static DEBUG_MESSAGES: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Debug console state
static DEBUG_CONSOLE_VISIBLE: Mutex<bool> = Mutex::new(false);

/// Add a debug message to be displayed this frame.
/// Messages are cleared every frame, so call this each frame you want to display something.
/// 
/// # Example
/// ```
/// debug_text!("Player position: {:?}", position);
/// debug_text!("Velocity: {:.2}", velocity);
/// ```
#[macro_export]
macro_rules! debug_text {
    ($($arg:tt)*) => {
        $crate::tooling::debug_console::add_debug_message(format!($($arg)*))
    };
}

/// Add a debug message programmatically
pub fn add_debug_message(message: String) {
    if let Ok(mut messages) = DEBUG_MESSAGES.lock() {
        messages.push(message);
    }
}

/// Collect and clear all messages for this frame.
/// Call this once per frame after rendering the console.
pub fn drain_messages() -> Vec<String> {
    if let Ok(mut messages) = DEBUG_MESSAGES.lock() {
        messages.drain(..).collect()
    } else {
        Vec::new()
    }
}

/// Get current messages without clearing them (for reading during frame)
pub fn get_messages() -> Vec<String> {
    if let Ok(messages) = DEBUG_MESSAGES.lock() {
        messages.clone()
    } else {
        Vec::new()
    }
}

/// Toggle debug console visibility
pub fn toggle_console() {
    if let Ok(mut visible) = DEBUG_CONSOLE_VISIBLE.lock() {
        *visible = !*visible;
        println!("Debug Console: {}", if *visible { "VISIBLE" } else { "HIDDEN" });
    }
}

/// Check if debug console is visible
pub fn is_console_visible() -> bool {
    if let Ok(visible) = DEBUG_CONSOLE_VISIBLE.lock() {
        *visible
    } else {
        false
    }
}

/// Set console visibility directly
pub fn set_console_visible(visible: bool) {
    if let Ok(mut v) = DEBUG_CONSOLE_VISIBLE.lock() {
        *v = visible;
    }
}
