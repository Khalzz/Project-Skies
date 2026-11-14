//! What the pilot sees happen to them - the G-effects on the view
//! (`update_g_vision`), and crashing and the wreck view (`check_crash`,
//! `show_crashes`). Its state is `PilotView`, the scene's `pilot_view`.

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
}

impl Play {
    /// Once the plane has crashed into the water (see Plane::check_wreck -
    /// the physics keep running: it gets dragged to a stop, floats, sinks),
    /// the camera backs out into a free orbit around the wreck. No death
    /// screen for now (play_ui::open_death_screen) - restart from the pause
    /// menu.
    ///
    /// When the crash kills the pilot the view blacks out first (see
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
        if self.debug.heat_view
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

    /// Which way the sun in the sky is - the "sun" node's position, read as
    /// a direction the way the renderer does (see App's lighting update).
    pub(super) fn sun_direction(scene: &Scene) -> Option<Vector3<f32>> {
        crate::ext::drawn::get(scene, "sun")
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
            let Some(mut at) = crate::ext::drawn::get(scene, &name)
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
