//! M's tactical map - opening and closing it, the camera into its view,
//! dragging and following, its lines and its buttons (`update_tactical_map`,
//! `fill_tactical_ui`). Its state is `TacticalState`, the scene's
//! `tactical`.

use super::*;

/// Which of the scene's frame plans is up (see game::frame_plans) - flying's,
/// the M map's lines, or its Chart (the world seen straight down).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ShownPlan {
    Flight,
    Map,
    Chart,
}

pub(super) struct TacticalState {
    // F10's tactical map - see update_tactical_map.
    pub(super) tactical_map: TacticalMap,
    // M's map (see tactical_map) is up - drawn instead of the world (see
    // game::frame_plans::tactical_map).
    pub(super) tactical_map_active: bool,
    // The tactical map's buttons, selection and flight plan window.
    pub(super) tactical_ui: TacticalUi,
    // Which of the scene's frame plans is up - the M map's or flying's (see
    // game::frame_plans) - so it's swapped only when the map opens or closes.
    pub(super) shown_plan: ShownPlan,
}

/// Where F10's map is on screen (logical px: x, y, width, height) - the
/// editor view's middle, between its panels, or the whole screen.
pub(super) fn tactical_viewport(app: &App) -> [f32; 4] {
    crate::game::frame_plans::tactical_viewport().unwrap_or([
        0.0,
        0.0,
        app.window.size.width as f32,
        app.window.size.height as f32,
    ])
}

impl Play {
    /// F10's editor and M's map (see play::tactical_map): opens/closes them
    /// (one at a time) - for the map, the
    /// camera into its orbit view and the renderer onto its lines - keeps
    /// every aircraft's trail, and while it's open builds this frame's
    /// lines.
    pub(super) fn update_tactical_map(&mut self, scene: &mut Scene, app: &mut App) {
        // F10 - the editor; not with the map up (one closes the other).
        if app.input.is_action_just_pressed("toggle_editor") {
            let open = !self.editor.editor_active;
            if open && self.tactical.tactical_map.open {
                self.tactical.tactical_map.open = false;
                self.tactical.tactical_map_active = false;
                if let Some(camera) = CameraControl::player_camera_mut(scene) {
                    camera.set_tactical_view(false);
                }
            }
            self.editor.editor_active = open;
            // Starts where the view is now.
            let active = scene.cameras.active();
            let from = (
                active.camera.position().coords,
                active.camera.calc_forward_direction(),
            );
            if let Some(camera) = CameraControl::player_camera_mut(scene) {
                camera.set_editor_view(open, from);
            }
            if !app.time.is_paused() {
                app.window.set_cursor_locked(!self.mouse_free());
            }
        }
        if app.input.is_action_just_pressed("toggle_tactical_map")
            && !self.editor.flight_editor.typing
        {
            if self.editor.editor_active {
                self.editor.editor_active = false;
                if let Some(camera) = CameraControl::player_camera_mut(scene) {
                    camera.set_editor_view(false, (Vector3::zeros(), Vector3::z()));
                }
            }
            self.tactical.tactical_map.open = !self.tactical.tactical_map.open;
            self.tactical.tactical_map_active = self.tactical.tactical_map.open;
            if let Some(camera) = CameraControl::player_camera_mut(scene) {
                camera.set_tactical_view(self.tactical.tactical_map.open);
            }
            // A pointer while it's up, to click with - see App::mouse_free.
            if !app.time.is_paused() {
                app.window.set_cursor_locked(!self.mouse_free());
            }
            // Opens with something selected - what the camera follows - so its
            // flight plan's up straight away.
            if self.tactical.tactical_map.open && self.tactical.tactical_ui.selected.is_none() {
                let followed = if self.camera.focus().is_empty() {
                    self.flight_manager.player_id().to_owned()
                } else {
                    self.camera.focus().to_owned()
                };
                self.tactical.tactical_ui.selected = Some(followed);
            }
        }

        let live: HashMap<String, LiveAircraft> = self
            .flight_manager
            .map()
            .aircraft()
            .filter_map(|(_, aircraft)| {
                Some((
                    aircraft.name.clone(),
                    self.flight_manager.live(scene, &aircraft.name)?,
                ))
            })
            .collect();
        if !app.time.is_paused() {
            self.tactical
                .tactical_map
                .record(&live, app.time.delta_time);
        }
        if !self.tactical.tactical_map.open {
            app.renderer
                .render_sets
                .lines("tactical_lines")
                .dynamic
                .clear();
            return;
        }

        // A flight plan changed in its window: into the map, applied now (a
        // bot flies the new path) - and into the editor's copy too, so it
        // doesn't put the old one back.
        // A flight plan point dragged on the map: across under the pointer,
        // at its own height - or, with Shift, up and down. Into the path like
        // an edit in its window (below).
        if let Some(drag) = self.tactical.tactical_ui.point_drag.take() {
            // The flight plan shown - an aircraft's or a map path's.
            let shown = self
                .tactical
                .tactical_ui
                .plan
                .as_ref()
                .map(|plan| (plan.target.clone(), plan.points.clone()));
            if let Some((target, mut path)) = shown {
                if let Some(point) = path.get_mut(drag.index) {
                    let active = scene.cameras.active();
                    let [left, top, width, height] = tactical_viewport(app);
                    if drag.vertical {
                        // A pixel's worth of height at the point's distance.
                        let distance = (Vector3::new(point.0, point.1, point.2)
                            - active.camera.position().coords)
                            .magnitude();
                        let meters_per_pixel =
                            2.0 * distance * (active.projection.fovy.to_radians() * 0.5).tan()
                                / height.max(1.0);
                        point.1 = (point.1 - drag.delta.y * meters_per_pixel).clamp(0.0, 20_000.0);
                    } else if let Some((origin, direction)) =
                        app.renderer.camera_resources.screen_ray(
                            active,
                            drag.pointer.x - left,
                            drag.pointer.y - top,
                            width as u32,
                            height as u32,
                        )
                    {
                        // Where the pointer's ray meets the flat plane at the
                        // point's height - not past the far horizon.
                        if direction.y.abs() > 1e-4 {
                            let distance = (point.1 - origin.y) / direction.y;
                            if distance > 0.0 && distance < 500_000.0 {
                                let hit = origin.coords + direction * distance;
                                point.0 = hit.x;
                                point.2 = hit.z;
                            }
                        }
                    }
                    self.tactical.tactical_ui.path_edit = Some((target, path));
                }
            }
        }

        // Into the map path it is (or the aircraft follows - every aircraft
        // on it flies the new one), or the aircraft's own.
        if let Some((target, path)) = self.tactical.tactical_ui.path_edit.take() {
            let apply = |map: &mut MapSpec| match &target {
                PlanTarget::Aircraft(name) => map.set_flight_plan(name, path.clone()),
                PlanTarget::Path(name) => {
                    if let Some(spec) = map.named_path_mut(name) {
                        spec.points = path.clone();
                    }
                }
            };
            let mut map = self.flight_manager.map().clone();
            apply(&mut map);
            if let Err(error) = self.flight_manager.sync(scene, app, &map) {
                eprintln!("F10 flight plan: {error}");
            }
            apply(&mut self.editor.flight_editor.map);
        }

        // Its center button: the camera kept on the selected aircraft -
        // otherwise free, dragged across the map.
        let ui = &mut self.tactical.tactical_ui;
        if ui
            .selected
            .as_ref()
            .is_some_and(|name| self.flight_manager.map().find(name).is_none())
        {
            ui.selected = None;
        }
        if ui.following {
            match &ui.selected {
                Some(name) => self.camera.set_focus(name),
                None => ui.following = false,
            }
        }
        let following = ui.following;
        let top_down = ui.top_down;
        let pan = std::mem::take(&mut ui.pan_delta);
        let [_, _, _, map_height] = tactical_viewport(app);
        if let Some(camera) = CameraControl::player_camera_mut(scene) {
            camera.set_tactical_follow(following);
            camera.set_tactical_top_down(top_down);
            if pan != (0.0, 0.0) {
                camera.pan_tactical(pan, map_height);
            }
        }

        let active = scene.cameras.active();
        let focus = crate::game::ext::drawn::get(scene, self.camera.focus())
            .map(|instance| instance.instance.transform.position)
            .unwrap_or_else(|| active.camera.position().coords);
        let to_pixels =
            app.window.pixel_size.height as f32 / (app.window.size.height as f32).max(1.0);
        let view = TacticalView {
            camera_position: active.camera.position().coords,
            fovy_deg: active.projection.fovy,
            screen_height: map_height * to_pixels,
            focus,
        };
        let sea_level = app.particles.sea_level().unwrap_or(0.0);
        let selected = self.tactical.tactical_ui.selected.clone();
        let hovered = self.tactical.tactical_ui.hovered.clone();
        // A map path picked in the editor is shown instead of the aircraft's
        // flight plan.
        let picked_path = self
            .tactical
            .tactical_ui
            .selected_path
            .as_deref()
            .and_then(|name| self.flight_manager.map().named_path(name))
            .cloned();
        let plan_of = if picked_path.is_some() {
            None
        } else {
            selected.as_deref()
        };
        app.renderer.render_sets.lines("tactical_lines").dynamic =
            self.tactical.tactical_map.lines(
                self.flight_manager.map(),
                &live,
                self.flight_manager.player_id(),
                plan_of,
                hovered.as_deref(),
                &view,
                sea_level,
            );
        if let Some(path) = &picked_path {
            tactical_map::flight_plan(
                &mut app.renderer.render_sets.lines("tactical_lines").dynamic,
                &path.points,
                path.looped,
                None,
                sea_level,
                &view,
            );
        }
        // The missiles in the air, in their shooter's colors.
        for missile in self.missiles.in_flight() {
            let rgb = match self.flight_manager.map().find(&missile.shooter) {
                Some((_, placement)) if placement.player => [1.0, 0.82, 0.30],
                Some((coalition, _)) => [coalition.color.0, coalition.color.1, coalition.color.2]
                    .map(|channel| channel as f32 / 255.0),
                None => [1.0, 1.0, 1.0],
            };
            tactical_map::missile_marker(
                &mut app.renderer.render_sets.lines("tactical_lines").dynamic,
                missile.position,
                missile.velocity,
                rgb,
                &view,
            );
        }
        self.fill_tactical_ui(scene, app, &live, selected.as_deref());
    }

    /// What the tactical map's egui half needs this frame (see play::
    /// tactical_ui): where every aircraft is on screen (to click), and the
    /// selected one's flight plan and its points' numbers.
    pub(super) fn fill_tactical_ui(
        &mut self,
        scene: &Scene,
        app: &mut App,
        live: &HashMap<String, LiveAircraft>,
        selected: Option<&str>,
    ) {
        // On the map's part of the screen (see tactical_viewport).
        let [left, top, width, height] = tactical_viewport(app);
        let active = scene.cameras.active();
        let on_screen = |position: Vector3<f32>| {
            app.renderer
                .camera_resources
                .world_to_screen(active, Point3::from(position), width as u32, height as u32)
                .map(|point| egui::pos2(left + point.x as f32, top + point.y as f32))
        };

        let pickables = live
            .iter()
            .filter(|(_, aircraft)| !aircraft.wrecked)
            .filter_map(|(name, aircraft)| Some((name.clone(), on_screen(aircraft.position)?)))
            .collect();

        let mut labels = Vec::new();
        let mut point_pickables = Vec::new();
        let picked_path = self
            .tactical
            .tactical_ui
            .selected_path
            .as_deref()
            .and_then(|name| self.flight_manager.map().named_path(name));
        let path_plan = picked_path.map(|path| {
            for (index, &(x, altitude, z)) in path.points.iter().enumerate() {
                if let Some(at) = on_screen(Vector3::new(x, altitude, z)) {
                    labels.push((
                        at,
                        format!("{}  {:.0} m", index + 1, altitude),
                        egui::Color32::WHITE,
                    ));
                    point_pickables.push((index, at));
                }
            }
            // On from the last point the way the last leg goes (or north).
            let next_suggestion = match path.points.as_slice() {
                [.., a, b] => (
                    b.0 + (b.0 - a.0).signum() * 1000.0,
                    b.1,
                    b.2 + (b.2 - a.2).signum() * 1000.0,
                ),
                [only] => (only.0, only.1, only.2 + 1000.0),
                [] => {
                    let at = scene.cameras.active().camera.position();
                    (at.x, 1000.0, at.z)
                }
            };
            FlightPlan {
                target: PlanTarget::Path(path.name.clone()),
                path_name: Some(path.name.clone()),
                name: path.name.clone(),
                coalition: String::new(),
                color: egui::Color32::from_rgb(140, 200, 255),
                plane: format!(
                    "{} points · {}{}",
                    path.points.len(),
                    if path.looped { "looped" } else { "once" },
                    if path.smooth { " · smooth" } else { "" }
                ),
                is_player: false,
                live: None,
                points: path.points.clone(),
                next: None,
                next_suggestion,
            }
        });
        let plan = if path_plan.is_some() {
            path_plan
        } else {
            selected
                .and_then(|name| self.flight_manager.map().find(name))
                .map(|(coalition, placement)| {
                    let now = live.get(&placement.name).copied();
                    let next = now.and_then(|now| now.next_path_point);
                    let (points, _) = self.flight_manager.map().flight_plan(placement);
                    for (index, &(x, altitude, z)) in points.iter().enumerate() {
                        if let Some(at) = on_screen(Vector3::new(x, altitude, z)) {
                            let color = if next == Some(index) {
                                egui::Color32::from_rgb(255, 205, 40)
                            } else {
                                egui::Color32::WHITE
                            };
                            labels.push((at, format!("{}  {:.0} m", index + 1, altitude), color));
                            point_pickables.push((index, at));
                        }
                    }
                    let is_player = placement.name == self.flight_manager.player_id();
                    let color = if is_player {
                        egui::Color32::from_rgb(255, 210, 90)
                    } else {
                        egui::Color32::from_rgb(
                            coalition.color.0,
                            coalition.color.1,
                            coalition.color.2,
                        )
                    };
                    FlightPlan {
                        target: PlanTarget::Aircraft(placement.name.clone()),
                        path_name: placement
                            .follow
                            .clone()
                            .filter(|name| self.flight_manager.map().named_path(name).is_some()),
                        name: placement.name.clone(),
                        coalition: coalition.name.clone(),
                        color,
                        plane: placement.plane.clone(),
                        is_player,
                        live: now,
                        next_suggestion: next_path_point(placement, &points, now),
                        points,
                        next,
                    }
                })
        };

        let roster = self
            .flight_manager
            .map()
            .coalitions
            .iter()
            .map(|coalition| RosterCoalition {
                name: coalition.name.clone(),
                color: egui::Color32::from_rgb(
                    coalition.color.0,
                    coalition.color.1,
                    coalition.color.2,
                ),
                aircraft: coalition
                    .aircraft
                    .iter()
                    .map(|aircraft| RosterEntry {
                        name: aircraft.name.clone(),
                        plane: aircraft.plane.clone(),
                        is_player: aircraft.name == self.flight_manager.player_id(),
                        status: match live.get(&aircraft.name) {
                            None => AircraftStatus::Missing,
                            Some(now) if now.wrecked || now.pilot_dead => AircraftStatus::Destroyed,
                            Some(_) => AircraftStatus::Flying,
                        },
                    })
                    .collect(),
            })
            .collect();

        let ui = &mut self.tactical.tactical_ui;
        ui.pickables = pickables;
        ui.labels = labels;
        ui.point_pickables = point_pickables;
        ui.roster = roster;
        ui.plan = plan;
    }
}
