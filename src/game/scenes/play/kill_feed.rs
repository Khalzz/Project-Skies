//! The kill feed - who downed whom, in the corner for a few seconds
//! (`update_kill_feed`), and the pilots killed this frame
//! (`collect_pilot_kills`).

use super::*;

/// How long a kill stays in the kill feed (s).
pub(super) const KILL_FEED_SECONDS: f32 = 8.0;

/// One line of the kill feed - see Play::update_kill_feed: its pieces
/// of text (who, what they did, which plane, the rest), each with its color
/// and how wide it is (px).
pub(super) struct KillFeedEntry {
    pub(super) pieces: [(String, UiColor, f32); play_ui::KILL_FEED_PIECES],
    pub(super) seconds_left: f32,
}

impl Play {
    /// Pilot kills and per-part blast damage, into the kill feed.
    pub(super) fn collect_pilot_kills(&mut self, scene: &mut Scene, app: &mut App) {
        let names: Vec<String> = self
            .flight_manager
            .map()
            .aircraft()
            .map(|(_, placement)| placement.name.clone())
            .collect();
        const PLAIN: UiColor = UiColor::Rgb(230, 230, 230);
        for name in names {
            let Some(plane) = scene
                .content
                .nodes
                .get_mut(&name)
                .and_then(|node| node.get_behavior_mut::<Plane>())
            else {
                continue;
            };
            let hits = plane.take_part_hits();
            let kill = plane.take_kill_report();
            for hit in hits {
                let damage = if hit.destroyed {
                    format!("{} -{:.0} destroyed", hit.part, hit.dealt)
                } else {
                    format!("{} -{:.0}", hit.part, hit.dealt)
                };
                println!(
                    "Hit - {name}'s {} took {:.0} from {}'s {}",
                    hit.part, hit.dealt, hit.shooter, hit.weapon
                );
                self.push_kill_feed(
                    app,
                    [
                        (hit.shooter.clone(), self.team_color(&hit.shooter)),
                        ("damaged".to_owned(), PLAIN),
                        (name.clone(), self.team_color(&name)),
                        (damage, PLAIN),
                    ],
                );
            }
            if let Some((shooter, weapon)) = kill {
                println!("Splash - {name} killed by {shooter}'s {weapon}");
                self.push_kill_feed(
                    app,
                    [
                        (shooter.clone(), self.team_color(&shooter)),
                        ("killed".to_owned(), PLAIN),
                        (name.clone(), self.team_color(&name)),
                        (format!("with {}", weapon_label(&weapon)), PLAIN),
                    ],
                );
            }
        }
    }

    /// One line on the kill feed. Newest stays on top.
    pub(super) fn push_kill_feed(
        &mut self,
        app: &mut App,
        texts: [(String, UiColor); play_ui::KILL_FEED_PIECES],
    ) {
        let pieces = texts.map(|(text, color)| {
            let width = text_width(&text, play_ui::KILL_FEED_FONT_SIZE, HUD_FONT);
            (text, color, width)
        });
        self.kill_feed.push(KillFeedEntry {
            pieces,
            seconds_left: KILL_FEED_SECONDS,
        });
    }

    /// The aircraft `name`'s coalition's color - grey when it isn't in the
    /// map.
    pub(super) fn team_color(&self, name: &str) -> UiColor {
        match self.flight_manager.map().find(name) {
            Some((coalition, _)) => {
                UiColor::Rgb(coalition.color.0, coalition.color.1, coalition.color.2)
            }
            None => UiColor::Rgb(180, 180, 180),
        }
    }

    /// The kill feed, top right: the latest kills (newest on top), each on
    /// its own dark box for KILL_FEED_SECONDS - not over F10's map. Every
    /// frame.
    pub(super) fn update_kill_feed(&mut self, scene: &mut Scene, app: &mut App) {
        const ROW_GAP: f32 = 4.0;
        const MARGIN: f32 = 16.0;
        if !app.time.is_paused() {
            for entry in &mut self.kill_feed {
                entry.seconds_left -= app.time.delta_time;
            }
            self.kill_feed.retain(|entry| entry.seconds_left > 0.0);
        }
        // Oldest dropped once there are more than it has room for.
        let overflow = self.kill_feed.len().saturating_sub(play_ui::KILL_FEED_ROWS);
        self.kill_feed.drain(..overflow);

        let shown = !(self.tactical.tactical_map_active || self.editor.editor_active);
        let height = play_ui::KILL_FEED_ROW_HEIGHT;
        let right = app.window.size.width as f32 - MARGIN;
        // Puts a node at (x, y), `width` wide - shown or hidden.
        let place = |node: &mut UiNode, x: f32, y: f32, width: f32, visible: bool| {
            node.set_active(visible);
            node.transform.x = x;
            node.transform.y = y;
            node.transform.rect.left = x;
            node.transform.rect.top = y;
            node.transform.rect.right = x + width;
            node.transform.rect.bottom = y + height;
        };
        for row in 0..play_ui::KILL_FEED_ROWS {
            // Newest on top.
            let entry = self.kill_feed.iter().rev().nth(row).filter(|_| shown);
            let top = MARGIN + row as f32 * (height + ROW_GAP);
            // The box just fits its text, against the right edge.
            let text_width = entry
                .map(|entry| {
                    entry.pieces.iter().map(|(_, _, width)| width).sum::<f32>()
                        + play_ui::KILL_FEED_GAP * (play_ui::KILL_FEED_PIECES - 1) as f32
                })
                .unwrap_or(0.0);
            let box_width = text_width + 2.0 * play_ui::KILL_FEED_PADDING;
            let left = right - box_width;
            if let Some(node) = scene.ui_node(&play_ui::kill_feed_key(row)) {
                place(node, left, top, box_width, entry.is_some());
            }
            let mut x = left + play_ui::KILL_FEED_PADDING;
            for piece in 0..play_ui::KILL_FEED_PIECES {
                let Some(node) = scene.ui_node(&play_ui::kill_feed_piece_key(row, piece)) else {
                    continue;
                };
                let Some((text, color, width)) = entry.map(|entry| &entry.pieces[piece]) else {
                    node.set_active(false);
                    continue;
                };
                // A pixel spare, so nothing's clipped by rounding.
                place(node, x, top, width + 2.0, true);
                node.update_style(|style| style.set_text_color(*color));
                if let Some(label) = node.as_label_mut() {
                    label.set_text(text);
                }
                x += width + play_ui::KILL_FEED_GAP;
            }
        }
    }
}
