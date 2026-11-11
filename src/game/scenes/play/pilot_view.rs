//! What the pilot sees happen to them - the G-effects on the view
//! (`update_g_vision`), crashing and the wreck view (`check_crash`,
//! `show_crashes`), and F8's heat view. Its state is `PilotView`, the
//! scene's `pilot_view`.

use super::*;

pub(super) struct PilotView {
    // This frame's G-effects (see update_g_vision) - handed to the g_vision
    // shader in after_cameras and cleared, so they only last the frame.
    pub(super) screen_effects: ScreenEffects,
    // Whether the HUD is hidden because the pilot can't see - see
    // update_g_vision.
    pub(super) hud_hidden_by_g: bool,
    // Seconds since the plane was wrecked - see check_crash.
    pub(super) crash_time: f32,
    // The pilot was killed with the camera in the cockpit - the crash's
    // fade shows through to the wreck view (see update_g_vision).
    pub(super) crash_seen_from_cockpit: bool,
    // F8's heat view is on - see update_heat_view.
    pub(super) heat_view: bool,
}

/// The heat view's color for a source `glow` bright (0..1) - an "ironbow"
/// thermal palette: dark purple-red, through orange, to white-hot.
pub(super) fn heat_color(glow: f32) -> [f32; 3] {
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
pub(super) fn heat_blob(
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
    /// Once the plane has crashed into the water (see Plane::check_wreck -
    /// the physics keep running: it gets dragged to a stop, floats, sinks),
    /// the camera backs out into a free orbit around the wreck. No death
    /// screen for now (play_ui::open_death_screen) - restart from the pause
    /// menu.
    ///
    /// When the crash kills the pilot the view cuts to black first (see
    /// Pilot's DEATH_*) and the camera switches while it's black, so the
    /// cut isn't seen. A crash the pilot survives switches after
    /// WRECK_VIEW_DELAY_SECONDS - long enough to tell whether it was fatal
    /// (then it waits for the black instead).
    pub(super) fn check_crash(&mut self, scene: &mut Scene, app: &App) {
        if self.is_dead {
            return;
        }
        let Some(plane) = scene
            .content
            .nodes
            .get(self.flight_manager.player_id())
            .and_then(|node| node.get_behavior::<Plane>())
        else {
            return;
        };
        if !plane.wrecked {
            return;
        }
        self.pilot_view.crash_time += app.time.delta_time;
        let ready = if plane.pilot.is_dead() {
            plane.pilot.death_view_hidden()
        } else {
            self.pilot_view.crash_time >= Self::WRECK_VIEW_DELAY_SECONDS
        };
        if !ready {
            return;
        }
        self.is_dead = true;
        let sea_level = app.particles.sea_level().unwrap_or(0.0);
        if let Some(camera) = CameraControl::player_camera_mut(scene) {
            camera.enter_wreck_view(sea_level);
        }
    }

    /// Drains the colors, closes in the tunnel or reds out the view as the
    /// pilot does (see plane::pilot::Pilot::screen_effects) - every frame,
    /// since screen effects only last one (see after_cameras).
    /// The flight HUD is drawn under that same pass, so it greys, dims and
    /// tunnels with the world. It's still switched off once crashed, and
    /// while the map, the editor or an animation asks for it gone.
    pub(super) fn update_g_vision(&mut self, scene: &mut Scene, app: &mut App) {
        let player_id = self.flight_manager.player_id();
        let Some(plane) = scene
            .content
            .nodes
            .get(player_id)
            .and_then(|node| node.get_behavior::<Plane>())
        else {
            return;
        };
        // Only seen through the pilot's eyes - out of the cockpit the pilot
        // still greys/blacks/reds out (and loses the plane), and a crash
        // still kills them, it just isn't shown. A crash seen from the
        // cockpit fades out and back in on the wreck view, the camera having
        // left the cockpit by then.
        let in_cockpit = self.camera.in_cockpit_of(scene, player_id);
        if !plane.pilot.is_dead() {
            self.pilot_view.crash_seen_from_cockpit = false;
        } else if in_cockpit {
            self.pilot_view.crash_seen_from_cockpit = true;
        }
        let shown = in_cockpit || self.pilot_view.crash_seen_from_cockpit;
        let mut effects = if shown {
            plane.pilot.screen_effects()
        } else {
            ScreenEffects::default()
        };
        // F8's heat view: the world dim and grey under the heat sources
        // (see update_heat_view).
        if self.pilot_view.heat_view
            && !(self.tactical.tactical_map_active || self.editor.editor_active)
        {
            effects = ScreenEffects::default();
            effects.desaturation = 1.0;
            effects.darkening = 0.75;
        }
        self.pilot_view.screen_effects = effects;

        let hide = plane.wrecked
            || plane.pilot.is_dead()
            || (self.tactical.tactical_map_active || self.editor.editor_active)
            || self.hud.animation_hides_hud;
        if !hide && self.pilot_view.hud_hidden_by_g {
            // Vision's back - the flight HUD returns (the quick menu shows
            // itself again once the pilot has control, see Plane).
            for hud in Self::FLIGHT_HUD {
                if let Some(node) = scene.ui_node(hud) {
                    node.set_active(true);
                }
            }
        }
        self.pilot_view.hud_hidden_by_g = hide;
    }

    /// F8's heat view - what an IR seeker sees: the world dimmed to grey
    /// (see update_g_vision) and every heat source on it - each aircraft
    /// (its engine, afterburner or fire) and the sun - as a glowing blob,
    /// bigger and whiter the brighter it looks from the camera, labelled
    /// with its heat and how bright it looks (the number a seeker compares,
    /// see play::weapon::heat). The blobs are drawn after the grey grade,
    /// so they keep their color.
    pub(super) fn update_heat_view(&mut self, scene: &mut Scene, app: &mut App) {
        // Past this many meters from the camera - not the jet it's sitting in.
        const MIN_DISTANCE: f32 = 30.0;
        app.renderer
            .render_sets
            .lines("heat_screen_lines")
            .dynamic
            .clear();
        if app.input.is_action_just_pressed("toggle_heat_view") {
            self.pilot_view.heat_view = !self.pilot_view.heat_view;
        }
        for index in 0..play_ui::HEAT_LABELS {
            if let Some(node) = scene.ui_node(&play_ui::heat_label_key(index)) {
                node.set_active(false);
            }
        }
        if !self.pilot_view.heat_view
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
                .ui_node(&play_ui::heat_label_key(index))
                .filter(|_| index < play_ui::HEAT_LABELS)
            else {
                continue;
            };
            node.set_active(true);
            node.transform.x = x - label_width / 2.0;
            node.transform.y = y + radius + 2.0;
            node.transform.rect.left = node.transform.x;
            node.transform.rect.top = node.transform.y;
            node.transform.rect.right = node.transform.x + label_width;
            node.transform.rect.bottom = node.transform.y + label_height;
            if let Some(label) = node.as_label_mut() {
                label.set_text(&text);
            }
        }
    }

    /// Which way the sun in the sky is - the "sun" node's position, read as
    /// a direction the way the renderer does (see App's lighting update).
    pub(super) fn sun_direction(scene: &Scene) -> Option<Vector3<f32>> {
        crate::game::ext::drawn::get(scene, "sun")
            .and_then(|sun| sun.instance.transform.position.try_normalize(1e-6))
    }

    /// Every aircraft that's just ended: an explosion where it hit the
    /// ground, a splash where it went into the sea (on the water's
    /// surface) - and the ones already playing kept going.
    pub(super) fn show_crashes(&mut self, scene: &mut Scene, app: &mut App) {
        let names: Vec<String> = self
            .flight_manager
            .map()
            .aircraft()
            .map(|(_, placement)| placement.name.clone())
            .collect();
        for name in names {
            let Some(crash) = scene
                .content
                .nodes
                .get_mut(&name)
                .and_then(|node| node.get_behavior_mut::<Plane>())
                .and_then(|plane| plane.crash.take())
            else {
                continue;
            };
            let Some(mut at) = crate::game::ext::drawn::get(scene, &name)
                .map(|instance| instance.instance.transform.position)
            else {
                continue;
            };
            if crash == Crash::Water {
                at.y = app.particles.sea_level().unwrap_or(0.0);
            }
            self.crash_effects.spawn(scene, app, crash, at);
        }
        self.crash_effects.update(scene, app, app.time.delta_time);
    }
}
