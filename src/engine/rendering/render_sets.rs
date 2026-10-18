//! Named sets of thick lines and SDF text - what a frame plan's `Lines` and
//! `Text` steps draw (see engine::rendering::frame).
//!
//! A scene registers the sets it uses when it starts (`App::add_line_set`,
//! `App::add_text_set`), fills them every frame by name, and puts steps for
//! them in its frame plan. They're dropped when the scene ends, so the next
//! one starts with none.

use std::collections::HashMap;

use super::sdf_text::SdfText;
use super::thick_lines::ThickLines;

#[derive(Default)]
pub struct RenderSets {
    lines: HashMap<String, ThickLines>,
    text: HashMap<String, SdfText>,
}

impl RenderSets {
    pub fn insert_lines(&mut self, name: &str, lines: ThickLines) {
        self.lines.insert(name.to_owned(), lines);
    }

    pub fn insert_text(&mut self, name: &str, text: SdfText) {
        self.text.insert(name.to_owned(), text);
    }

    /// The line set `name`. It has to be registered - a scene only uses the
    /// sets it registered itself, so a missing one is a mistake in the code.
    pub fn lines(&mut self, name: &str) -> &mut ThickLines {
        self.lines.get_mut(name).unwrap_or_else(|| panic!("line set \"{name}\" isn't registered - add it with App::add_line_set when the scene starts"))
    }

    /// The text set `name` (see `lines`).
    pub fn text(&mut self, name: &str) -> &mut SdfText {
        self.text.get_mut(name).unwrap_or_else(|| panic!("text set \"{name}\" isn't registered - add it with App::add_text_set when the scene starts"))
    }

    /// The line set `name`, if there is one.
    pub fn get_lines_mut(&mut self, name: &str) -> Option<&mut ThickLines> {
        self.lines.get_mut(name)
    }

    /// The text set `name`, if there is one.
    pub fn get_text_mut(&mut self, name: &str) -> Option<&mut SdfText> {
        self.text.get_mut(name)
    }

    /// Drops every set - the scene's ending.
    pub fn clear(&mut self) {
        self.lines.clear();
        self.text.clear();
    }
}
