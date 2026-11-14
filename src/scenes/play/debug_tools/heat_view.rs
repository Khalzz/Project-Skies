//! F8's heat view - what an IR seeker sees: every heat source as a glowing
//! blob labelled with its heat, over the world dimmed to grey (see
//! Play::update_g_vision).

use super::super::*;

/// The heat view's color for a source `glow` bright (0..1) - an "ironbow"
/// thermal palette: dark purple-red, through orange, to white-hot.
fn heat_color(glow: f32) -> [f32; 3] {
    let mix = |a: [f32; 3], b: [f32; 3], t: f32| {
        [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ]
    };
    const COOL: [f32; 3] = [0.45, 0.0, 0.35];
    const WARM: [f32; 3] = [1.0, 0.45, 0.0];
    const HOT: [f32; 3] = [1.0, 1.0, 0.85];
    let glow = glow.clamp(0.0, 1.0);
    if glow < 0.5 {
        mix(COOL, WARM, glow * 2.0)
    } else {
        mix(WARM, HOT, glow * 2.0 - 1.0)
    }
}

/// A filled disc `radius` UI pixels round `center` (UI pixels), in `color` -
/// as stripes of screen-space line (see ThickLines::screen_space), so it
/// rides the HUD's own lines.
fn heat_blob(
    lines: &mut Vec<LineInstance>,
    center: (f32, f32),
    radius: f32,
    color: [f32; 4],
    to_pixels: f32,
) {
    // Stripes 2 px apart, 2 px thick - solid, and the soft edges blend them.
    let (cx, cy, r) = (
        center.0 * to_pixels,
        center.1 * to_pixels,
        radius * to_pixels,
    );
    let mut dy = -r;
    while dy <= r {
        let half = (r * r - dy * dy).max(0.0).sqrt();
        lines.push(LineInstance {
            start: [cx - half, cy + dy, 0.0],
            end: [cx + half, cy + dy, 0.0],
            color,
            width: 2.0,
        });
        dy += 2.0;
    }
}

impl Play {
    /// F8's heat view - what an IR seeker sees: the world dimmed to grey
    /// (see update_g_vision) and every heat source on it - each aircraft
    /// (its engine, afterburner or fire) and the sun - as a glowing blob,
    /// bigger and whiter the brighter it looks from the camera, labelled
    /// with its heat and how bright it looks (the number a seeker compares,
    /// see play::weapon::heat). The blobs are drawn after the grey grade,
    /// so they keep their color.
    pub(in crate::scenes::play) fn update_heat_view(&mut self, scene: &mut Scene, app: &mut App) {
        // Past this many meters from the camera - not the jet it's sitting in.
        const MIN_DISTANCE: f32 = 30.0;
        app.renderer
            .render_sets
            .lines("heat_screen_lines")
            .dynamic
            .clear();
        if app.input.is_action_just_pressed("toggle_heat_view") {
            self.debug.heat_view = !self.debug.heat_view;
        }
        for index in 0..play_ui::HEAT_LABELS {
            if let Some(node) = scene.ui_node(&play_ui::in_hud(&play_ui::heat_label_key(index))) {
                node.set_active(false);
            }
        }
        if !self.debug.heat_view
            || (self.tactical.tactical_map_active || self.editor.editor_active)
        {
            return;
        }

        let active = scene.cameras.active();
        let eye = active.camera.position().coords;
        let (width, height) = (app.window.size.width, app.window.size.height);
        let on_screen = |point: Vector3<f32>| {
            app.renderer
                .camera_resources
                .world_to_screen(active, Point3::from(point), width, height)
                .map(|point| (point.x as f32, point.y as f32))
        };
        // (where on screen, how bright it looks, its label)
        let mut spots: Vec<((f32, f32), f32, String)> = Vec::new();
        for track in self
            .aircraft_tracks(scene, true)
            .into_iter()
            .chain(self.countermeasures.heat_tracks())
        {
            let distance = (track.position - eye).magnitude();
            if distance < MIN_DISTANCE {
                continue;
            }
            if let Some(at) = on_screen(track.position) {
                let apparent = heat::apparent(track.heat, distance);
                spots.push((at, apparent, format!("{:.1}  ({apparent:.2})", track.heat)));
            }
        }
        if let Some(at) = Self::sun_direction(scene).and_then(|sun| on_screen(eye + sun * 10_000.0))
        {
            spots.push((
                at,
                heat::SUN_APPARENT,
                format!("SUN  ({:.2})", heat::SUN_APPARENT),
            ));
        }
        // Dimmest first, so the brightest draw on top.
        spots.sort_by(|a, b| a.1.total_cmp(&b.1));

        let to_pixels = app.window.pixel_size.height as f32 / (height as f32).max(1.0);
        let (label_width, label_height) = play_ui::HEAT_LABEL_SIZE;
        for (index, ((x, y), apparent, text)) in spots.into_iter().enumerate() {
            // 0 (dim) .. 1 (searing) - halfway at an apparent 1, the sun.
            let glow = apparent / (apparent + 1.0);
            let radius = 4.0 + 30.0 * glow;
            let color = heat_color(glow);
            heat_blob(
                &mut app.renderer.render_sets.lines("heat_screen_lines").dynamic,
                (x, y),
                radius * 2.0,
                [color[0], color[1], color[2], 0.25 * glow + 0.05],
                to_pixels,
            );
            heat_blob(
                &mut app.renderer.render_sets.lines("heat_screen_lines").dynamic,
                (x, y),
                radius,
                [color[0], color[1], color[2], 0.95],
                to_pixels,
            );

            let Some(node) = scene
                .ui_node(&play_ui::in_hud(&play_ui::heat_label_key(index)))
                .filter(|_| index < play_ui::HEAT_LABELS)
            else {
                continue;
            };
            node.set_active(true);
            ui_layers::place(
                node,
                x - label_width / 2.0,
                y + radius + 2.0,
                label_width,
                label_height,
            );
            if let Some(label) = node.as_label_mut() {
                label.set_text(&text);
            }
        }
    }
}
