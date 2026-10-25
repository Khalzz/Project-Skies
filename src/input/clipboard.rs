//! The system clipboard - through SDL, which is up for as long as the app
//! is (see App::new). Used by the text inputs (Ctrl+C/X/V) and the debug
//! windows (egui's copy).

/// Puts `text` on the system clipboard.
pub fn set_clipboard(text: &str) {
    let Ok(text) = std::ffi::CString::new(text) else { return };
    unsafe {
        sdl2::sys::SDL_SetClipboardText(text.as_ptr());
    }
}

/// What's on the system clipboard, if it's text.
pub fn clipboard() -> Option<String> {
    unsafe {
        let raw = sdl2::sys::SDL_GetClipboardText();
        if raw.is_null() {
            return None;
        }
        let text = std::ffi::CStr::from_ptr(raw).to_string_lossy().into_owned();
        sdl2::sys::SDL_free(raw as *mut std::ffi::c_void);
        (!text.is_empty()).then_some(text)
    }
}
