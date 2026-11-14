//! The kill feed - the "kill_feed" element: who downed or damaged whom, top
//! right, each line on its own dark box for `KILL_FEED_SECONDS`, newest on
//! top. Only shows what it's told (`KillFeed::push`) - what happened is the
//! scene's business (see Play::collect_pilot_kills).

use glyphon::cosmic_text::Align;
use pankarta_software::app::App;
use pankarta_software::prelude::Scene;
use pankarta_software::ui::color::UiColor;
use pankarta_software::ui::components::label::text_width;
use pankarta_software::ui::ui_node::UiNode;
use pankarta_software::ui::ui_transform::{PositionValue, SizeValue};

use crate::ext::ui_layers::{self, GameUi};
use crate::ui::{label, HUD_FONT};

/// The kill feed's node.
pub const KILL_FEED: &str = "kill_feed";

/// How long a line stays (s).
const KILL_FEED_SECONDS: f32 = 8.0;
/// How many lines at most, each one's height, its text's size, the room
/// either side of the text inside its box and between its pieces (px).
const KILL_FEED_ROWS: usize = 8;
const KILL_FEED_ROW_HEIGHT: f32 = 26.0;
const KILL_FEED_FONT_SIZE: f32 = 13.0;
const KILL_FEED_PADDING: f32 = 10.0;
const KILL_FEED_GAP: f32 = 5.0;
/// How many pieces of text a line has - who, what they did, to whom, the
/// rest - each its own label, in its own color.
pub const KILL_FEED_PIECES: usize = 4;
/// The widest a piece of text can be (px) - its label's own width.
const KILL_FEED_PIECE_MAX_WIDTH: f32 = 320.0;
/// The gap between lines, and from the screen's top right corner (px).
const ROW_GAP: f32 = 4.0;
const MARGIN: f32 = 16.0;

/// One line: its pieces of text, each with its color and how wide it is
/// (px), and how long it has left.
struct Line {
    pieces: [(String, UiColor, f32); KILL_FEED_PIECES],
    seconds_left: f32,
}

/// The lines on the kill feed, oldest first.
#[derive(Default)]
pub struct KillFeed {
    lines: Vec<Line>,
}

/// A line's box.
fn row_key(row: usize) -> String {
    format!("row_{row}")
}

/// A line's piece of text `piece`.
fn piece_key(row: usize, piece: usize) -> String {
    format!("row_{row}_{piece}")
}

/// Puts the kill feed up, empty. Each line's box comes before its text, so
/// the text is drawn over it.
pub(super) fn spawn(scene: &mut Scene, app: &mut App) {
    let at_origin = |node: UiNode| node.set_position(PositionValue::Start(0.0), PositionValue::Start(0.0));
    let mut root = ui_layers::screen_root();
    for row in 0..KILL_FEED_ROWS {
        root = root.set_child(row_key(row), at_origin(build_box()));
        for piece in 0..KILL_FEED_PIECES {
            root = root.set_child(piece_key(row, piece), at_origin(build_piece()));
        }
    }
    ui_layers::add(scene, app, KILL_FEED, root);
}

/// A line's box - dark, see-through, rounded, sized to its text every
/// frame. Built hidden.
fn build_box() -> UiNode {
    label("")
        .set_size(SizeValue::Pixels(KILL_FEED_PIECE_MAX_WIDTH), SizeValue::Pixels(KILL_FEED_ROW_HEIGHT))
        .set_background_color(UiColor::Rgba(0, 0, 0, 150))
        .set_corner_radius(4.0)
        .active(false)
}

/// A piece of a line's text - on no box of its own (the line's is under
/// it), its color set with its text. Built hidden.
fn build_piece() -> UiNode {
    label("")
        .set_size(SizeValue::Pixels(KILL_FEED_PIECE_MAX_WIDTH), SizeValue::Pixels(KILL_FEED_ROW_HEIGHT))
        .set_font(HUD_FONT)
        .set_font_size(KILL_FEED_FONT_SIZE)
        .set_text_color(UiColor::Rgb(230, 230, 230))
        .set_background_color(UiColor::TRANSPARENT)
        .set_align(Align::Left)
        .active(false)
}

impl KillFeed {
    /// Adds a line - its pieces of text, each in its color. Newest shows on
    /// top.
    pub fn push(&mut self, texts: [(String, UiColor); KILL_FEED_PIECES]) {
        let pieces = texts.map(|(text, color)| {
            let width = text_width(&text, KILL_FEED_FONT_SIZE, HUD_FONT);
            (text, color, width)
        });
        self.lines.push(Line { pieces, seconds_left: KILL_FEED_SECONDS });
    }

    /// Every frame: ages the lines by `delta_time` (0 while paused) and lays
    /// them out - against the right edge of a `screen_width` wide screen,
    /// or hidden when not `shown`.
    pub fn update(&mut self, scene: &mut Scene, delta_time: f32, screen_width: f32, shown: bool) {
        for line in &mut self.lines {
            line.seconds_left -= delta_time;
        }
        self.lines.retain(|line| line.seconds_left > 0.0);
        // Oldest dropped once there are more than it has room for.
        let overflow = self.lines.len().saturating_sub(KILL_FEED_ROWS);
        self.lines.drain(..overflow);

        let height = KILL_FEED_ROW_HEIGHT;
        let right = screen_width - MARGIN;
        let Some(layer) = scene.ui_layer(KILL_FEED) else { return };
        for row in 0..KILL_FEED_ROWS {
            // Newest on top.
            let line = self.lines.iter().rev().nth(row).filter(|_| shown);
            let top = MARGIN + row as f32 * (height + ROW_GAP);
            // The box just fits its text.
            let text_width = line
                .map(|line| {
                    line.pieces.iter().map(|(_, _, width)| width).sum::<f32>()
                        + KILL_FEED_GAP * (KILL_FEED_PIECES - 1) as f32
                })
                .unwrap_or(0.0);
            let box_width = text_width + 2.0 * KILL_FEED_PADDING;
            let left = right - box_width;
            if let Some(node) = layer.get_mut(&row_key(row)) {
                node.set_active(line.is_some());
                ui_layers::place(node, left, top, box_width, height);
            }
            let mut x = left + KILL_FEED_PADDING;
            for piece in 0..KILL_FEED_PIECES {
                let Some(node) = layer.get_mut(&piece_key(row, piece)) else { continue };
                let Some((text, color, width)) = line.map(|line| &line.pieces[piece]) else {
                    node.set_active(false);
                    continue;
                };
                node.set_active(true);
                // A pixel spare, so nothing's clipped by rounding.
                ui_layers::place(node, x, top, width + 2.0, height);
                node.update_style(|style| style.set_text_color(*color));
                if let Some(label) = node.as_label_mut() {
                    label.set_text(text);
                }
                x += width + KILL_FEED_GAP;
            }
        }
    }
}
