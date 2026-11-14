//! F10's editor - the live map editor docked round the world
//! (`update_flight_editor`), what it's told about the scene
//! (`fill_editor_scene`), the telemetry of what the camera's on and the
//! editor's lines over the world. Its state is `EditorState`, the scene's
//! `editor`.
//!
//! Everything of the editor's is under it:
//! - `flight_editor`: the editor's panels - the hierarchy, the inspector,
//!   the timeline, the flight plans;
//! - `gizmos`: what it draws over the world (camera views, paths, axes).

pub mod flight_editor;
pub mod gizmos;

use super::*;

pub(super) struct EditorState {
    // What used to live on the game's own App (the engine's has no room for
    // a game's state - a scene's is its own):
    // The live map editor, docked round the world in F10's editor (see
    // update_flight_editor) - drawn in debug_ui.
    pub(super) flight_editor: FlightEditor,
    // Whether the flight editor (F10's editor view) was open last frame - see
    // update_flight_editor.
    pub(super) flight_editor_was_open: bool,
    // The editor's map didn't validate last frame - its error is showing.
    pub(super) flight_editor_map_invalid: bool,
    // F10's editor - the world seen through a free editor camera, the
    // editor's panels round it.
    pub(super) editor_active: bool,
    // The telemetry's last look at what the camera's on: its id and
    // velocity - for a missile's G.
    pub(super) telemetry_last: Option<(String, Vector3<f32>)>,
    // The debug view (F5/F9) while spectating - what the camera's on.
    pub(super) spectate_telemetry: Option<Telemetry>,
}

impl Play {
    /// The flight editor (see play::editor::flight_editor), docked in F10's editor
    /// view: starts it from the world as it is when that opens, carries out
    /// what it was asked to (save, reload, respawn...), and applies its map
    /// to the world - every frame it's up (see flight_manager::sync).
    pub(super) fn update_flight_editor(&mut self, scene: &mut Scene, app: &mut App) {
        // Docked in F10's editor view - up while that is.
        let open = self.editor.editor_active;
        if open != self.editor.flight_editor_was_open {
            self.editor.flight_editor_was_open = open;
            if open {
                // Starts from the world as it is.
                self.editor.flight_editor.map = self.flight_manager.map().clone();
                self.editor.flight_editor.planes = AircraftSpec::available();
                // What each plane's loadout can fill, and with what.
                self.editor.flight_editor.pylons = self
                    .editor
                    .flight_editor
                    .planes
                    .iter()
                    .filter_map(|plane| {
                        Some((plane.clone(), AircraftSpec::load(plane).ok()?.pylons))
                    })
                    .collect();
                // And how many flares and chaff it carries.
                self.editor.flight_editor.countermeasures = self
                    .editor
                    .flight_editor
                    .planes
                    .iter()
                    .filter_map(|plane| {
                        let countermeasures = AircraftSpec::load(plane).ok()?.countermeasures?;
                        Some((
                            plane.clone(),
                            (countermeasures.flares, countermeasures.chaff),
                        ))
                    })
                    .collect();
                self.editor.flight_editor.fuel_capacity = self
                    .editor
                    .flight_editor
                    .planes
                    .iter()
                    .filter_map(|plane| {
                        let capacity = AircraftSpec::load(plane).ok()?.fuel.capacity_kg;
                        (capacity > 0.0).then_some((plane.clone(), capacity))
                    })
                    .collect();
                self.editor.flight_editor.weapons = WeaponSpec::available()
                    .into_iter()
                    .filter_map(|id| Some((id.clone(), WeaponSpec::read(&id).ok()?.category)))
                    .collect();
                self.editor.flight_editor.status = None;
                self.editor.flight_editor.requests.clear();
            } else {
                self.editor.flight_editor.typing = false;
            }
            // The pause menu owns the cursor while it's up - see
            // play_ui::close_pause_menu.
            if !app.time.is_paused() {
                app.window.set_cursor_locked(!self.mouse_free());
            }
        }
        if !open {
            return;
        }

        let mut editor = std::mem::take(&mut self.editor.flight_editor);
        let mut respawn_all = false;
        let mut respawn = Vec::new();
        for request in std::mem::take(&mut editor.requests) {
            match request {
                EditorRequest::Save => {
                    editor.status = Some(match editor.map.save(LEVEL_FOLDER) {
                        Ok(()) => (
                            format!("Saved to {}", MapSpec::path(LEVEL_FOLDER).display()),
                            false,
                        ),
                        Err(error) => (error, true),
                    });
                }
                EditorRequest::ReloadFromFile => {
                    editor.status = Some(match MapSpec::load(LEVEL_FOLDER) {
                        Ok(map) => {
                            editor.map = map;
                            ("Reloaded map.ron".to_owned(), false)
                        }
                        Err(error) => (error, true),
                    });
                }
                EditorRequest::Timeline(request) => match request {
                    TimelineRequest::Play(node, name) => {
                        let length = editor
                            .map
                            .animation(&node, &name)
                            .map(|animation| animation.length)
                            .unwrap_or(0.0);
                        self.animations.resume(&node, &name, length);
                    }
                    TimelineRequest::Start(node, name) => self.animations.start(&node, &name),
                    TimelineRequest::Pause(node, name) => self.animations.pause(&node, &name),
                    TimelineRequest::Stop(node, name) => self.animations.stop(&node, &name),
                    TimelineRequest::Reset(node, name) => {
                        self.animations.stop(&node, &name);
                        if let Some(held) = scene.content.nodes.get_mut(&node) {
                            held.remove_property::<self::path_follow::Held>();
                        }
                        if editor.map.find(&node).is_some() {
                            respawn.push(node);
                        } else if let Some(spec) = editor.map.node(&node).cloned() {
                            if let Some(held) = scene.content.nodes.get_mut(&node) {
                                held.remove_property::<self::path_follow::Held>();
                            }
                            if let Some(placed) = scene.content.nodes.get_mut(&node) {
                                placed.set_property(Transform3D {
                                    position: spec.position(),
                                    rotation: spec.rotation(),
                                    scale: Vector3::new(1.0, 1.0, 1.0),
                                });
                            }
                        }
                    }
                    TimelineRequest::Seek(node, name, time) => {
                        let length = editor
                            .map
                            .animation(&node, &name)
                            .map(|animation| animation.length)
                            .unwrap_or(time);
                        self.animations.seek(&node, &name, time.min(length));
                    }
                },
                EditorRequest::Focus(id) => {
                    let distance = if self.flight_manager.map().find(&id).is_some() {
                        camera::EDITOR_FOCUS_AIRCRAFT
                    } else {
                        camera::EDITOR_FOCUS_NODE
                    };
                    self.camera.editor_focus_on(scene, &id, distance);
                }
                EditorRequest::RespawnAll => respawn_all = true,
                EditorRequest::Respawn(name) => respawn.push(name),
                EditorRequest::ResetLoadout(name) => {
                    self.flight_manager.reset_loadout(scene, app, &name)
                }
                EditorRequest::SetProfile {
                    aircraft,
                    label,
                    on,
                } => {
                    if let Some(node) = scene.content.nodes.get_mut(&aircraft) {
                        node.push_physics_event(ProfileSwitch { label, on });
                    }
                }
                EditorRequest::CaptureCurrent(name) => {
                    // Into both copies of the map, so nothing teleports.
                    if let Some((position, rotation)) =
                        self.flight_manager.capture_current(scene, &name)
                    {
                        if let Some(placement) = editor.map.find_mut(&name) {
                            placement.position = position;
                            placement.rotation = rotation;
                        }
                    }
                }
            }
        }

        match self.flight_manager.sync(scene, app, &editor.map) {
            Ok(()) => {
                if self.editor.flight_editor_map_invalid {
                    editor.status = None;
                }
                self.editor.flight_editor_map_invalid = false;
            }
            Err(error) => {
                editor.status = Some((error, true));
                self.editor.flight_editor_map_invalid = true;
            }
        }
        let respawned = if respawn_all {
            self.flight_manager.respawn_all(scene, app)
        } else {
            respawn
                .iter()
                .map(|name| self.flight_manager.respawn(scene, app, name))
                .collect()
        };
        if let Err(error) = respawned {
            editor.status = Some((error, true));
        }

        editor.live = self
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
        editor.profiles = self
            .flight_manager
            .map()
            .aircraft()
            .filter_map(|(_, aircraft)| {
                let profiles = scene
                    .content
                    .nodes
                    .get(&aircraft.name)?
                    .physics_state::<AircraftState>()?
                    .profiles
                    .clone();
                Some((aircraft.name.clone(), profiles))
            })
            .collect();
        self.fill_editor_scene(scene, &mut editor);
        self.editor.flight_editor = editor;

        // The player flying again (respawned, or switched to a plane that
        // isn't wrecked) - out of the crash view.
        if self.is_dead {
            let player_wrecked = scene
                .content
                .nodes
                .get(self.flight_manager.player_id())
                .and_then(|node| node.get_behavior::<Plane>())
                .is_none_or(|plane| plane.wrecked);
            if !player_wrecked {
                self.is_dead = false;
                self.pilot_view.crash_time = 0.0;
                if let Some(camera) = CameraControl::player_camera_mut(scene) {
                    camera.leave_wreck_view();
                }
            }
        }
    }

    /// What the editor shows of the scene this frame: every node (for the
    /// hierarchy), the shown animation's clock, and where the view is (for a
    /// new camera key).
    pub(super) fn fill_editor_scene(
        &self,
        scene: &Scene,
        editor: &mut self::flight_editor::FlightEditor,
    ) {
        let map = self.flight_manager.map();
        editor.nodes = scene
            .content
            .nodes
            .iter()
            .map(|node| {
                let kind = if map.find(&node.id).is_some() {
                    NodeKind::Aircraft
                } else if map.node(&node.id).is_some() {
                    NodeKind::MapNode
                } else if node.get_property::<SpawnedLive>().is_some()
                    || self.flight_manager.is_store(&node.id)
                    || node.id == ANIMATION_CAMERA
                {
                    NodeKind::Runtime
                } else {
                    NodeKind::Level
                };
                let parent = node.get_property::<Parent>();
                let world = node
                    .get_property::<Transform3D>()
                    .copied()
                    .unwrap_or_default();
                SceneNodeInfo {
                    id: node.id.clone(),
                    kind,
                    parent: parent.map(|parent| parent.id.clone()),
                    space: parent.map(|parent| parent.space).unwrap_or(Space::Local),
                    camera: node.get_property::<CameraLens>().is_some(),
                    animated: map
                        .animations
                        .get(&node.id)
                        .is_some_and(|animations| !animations.is_empty()),
                    world_position: world.position,
                    world_rotation: degrees_from_rotation(&world.rotation),
                }
            })
            .collect();
        editor.playback = editor
            .current_animation
            .as_ref()
            .and_then(|(node, name)| self.animations.get(node, name))
            .map(|running| Playback {
                time: running.time,
                playing: running.playing,
                engaged: true,
            })
            .unwrap_or_default();
        // The editor camera's pose - what a new camera key takes.
        let active = scene.cameras.active();
        editor.view_position = active.camera.position().coords;
        editor.view_rotation = Some(facing_with_up(
            active.camera.calc_forward_direction(),
            active.camera.up,
        ));
        editor.view_fov = active.projection.fovy;
        if editor
            .preview_camera
            .as_ref()
            .is_some_and(|camera| scene.cameras.get(camera).is_none())
        {
            editor.preview_camera = None;
        }
    }

    /// What the camera's on, for the editor's Telemetry window - an
    /// aircraft or a missile (not the player's own jet): how fast, how high,
    /// the G, what it's doing.
    pub(super) fn telemetry(&mut self, scene: &Scene, delta_time: f32) -> Option<Telemetry> {
        let focus = self.camera.focus().to_owned();
        if focus.is_empty() || focus == self.flight_manager.player_id() {
            self.editor.telemetry_last = None;
            return None;
        }
        let row = |label: &str, value: String| (label.to_owned(), value, false);
        let warn = |label: &str, value: String| (label.to_owned(), value, true);
        let heading =
            |velocity: Vector3<f32>| (velocity.x.atan2(velocity.z).to_degrees() + 360.0) % 360.0;

        if let Some(missile) = missile::get(scene, &focus).map(Missile::telemetry) {
            // Its G: how its velocity changed since last frame, gravity out.
            let g = match &self.editor.telemetry_last {
                Some((id, last)) if *id == focus && delta_time > 1e-4 => {
                    let felt =
                        (missile.velocity - last) / delta_time - Vector3::new(0.0, -9.81, 0.0);
                    Some(felt.magnitude() / 9.81)
                }
                _ => None,
            };
            self.editor.telemetry_last = Some((focus.clone(), missile.velocity));
            let speed = missile.velocity.magnitude();
            let target_at = missile
                .target
                .as_deref()
                .and_then(|target| scene.world_transform(target))
                .map(|transform| transform.position);
            let mut rows = vec![
                row(
                    "Speed",
                    format!("{:.0} kt · M{:.2}", speed * 1.943_84, speed / 340.29),
                ),
                row("Altitude", format!("{:.0} m", missile.position.y)),
                g.map(|g| {
                    if g > 30.0 {
                        warn("G", format!("{g:.0} G"))
                    } else {
                        row("G", format!("{g:.1} G"))
                    }
                })
                .unwrap_or_else(|| row("G", "-".to_owned())),
                row("Heading", format!("{:.0}°", heading(missile.velocity))),
                row("Flying for", format!("{:.1} s", missile.time)),
                row(
                    "Motor",
                    if missile.burning {
                        "burning".to_owned()
                    } else {
                        "burnt out".to_owned()
                    },
                ),
            ];
            match (&missile.target, target_at) {
                (Some(target), Some(at)) => rows.push(row(
                    "Target",
                    format!(
                        "{target} · {:.1} km",
                        (at - missile.position).magnitude() / 1000.0
                    ),
                )),
                (Some(target), None) => rows.push(row("Target", target.clone())),
                (None, _) => rows.push(warn("Target", "none - lost it".to_owned())),
            }
            return Some(Telemetry {
                title: focus.clone(),
                subtitle: format!("{} · fired by {}", missile.weapon, missile.shooter),
                rows,
            });
        }

        self.editor.telemetry_last = None;
        let (coalition, placement) = self.flight_manager.map().find(&focus)?;
        let live = self.flight_manager.live(scene, &focus)?;
        let node = scene.content.nodes.get(&focus);
        let state = node.and_then(|node| node.physics_state::<AircraftState>());
        let plane = node.and_then(|node| node.get_behavior::<Plane>());
        let flight_data = state.map(|state| &state.flight_data);
        let mut rows = vec![
            row(
                "Speed",
                format!(
                    "{:.0} kt · M{:.2}",
                    live.speed_kt,
                    flight_data.map(|data| data.mach).unwrap_or(0.0)
                ),
            ),
            row("Altitude", format!("{:.0} m", live.altitude)),
        ];
        let g_text = format!("{:+.1} G (commanded {:+.1})", live.g, live.g_command);
        rows.push(if live.g > 9.0 || live.g < -3.0 {
            warn("G", g_text)
        } else {
            row("G", g_text)
        });
        rows.push(row(
            "Angle of attack",
            format!("{:.1}°", flight_data.map(|data| data.aoa_y).unwrap_or(0.0)),
        ));
        if let Some(plane) = plane {
            let afterburner = state.map(|state| state.afterburner).unwrap_or(0.0);
            rows.push(row(
                "Throttle",
                format!(
                    "{:.0}%{}",
                    plane.controls.throttle * 100.0,
                    if afterburner > 0.01 {
                        " · afterburner"
                    } else {
                        ""
                    }
                ),
            ));
        }
        rows.push(row(
            "Heading",
            format!(
                "{:.0}°",
                heading(
                    flight_data
                        .map(|data| data.velocity)
                        .unwrap_or_else(Vector3::z)
                )
            ),
        ));
        rows.push(row("Flown by", live.flown_by.to_owned()));
        if let Some(waypoint) = live.waypoint {
            rows.push(row(
                "Next point",
                format!("{:.1} km", (waypoint - live.position).magnitude() / 1000.0),
            ));
        }
        if live.wrecked {
            rows.push(warn("State", "wrecked".to_owned()));
        } else if live.pilot_dead {
            rows.push(warn("State", "pilot killed".to_owned()));
        }
        let who = if placement.player {
            "player".to_owned()
        } else {
            format!("{} pilot", placement.difficulty.label())
        };
        Some(Telemetry {
            title: focus.clone(),
            subtitle: format!("{} · {} · {who}", placement.plane, coalition.name),
            rows,
        })
    }

    /// F10's editor's lines over the world - see play::editor::gizmos.
    /// F10's editor's lines over the world (see play::editor::gizmos) - and,
    /// in the editor or the debug view (F5/F9) while spectating, the path
    /// of what's followed.
    pub(super) fn update_editor_gizmos(&self, scene: &Scene, app: &mut App) {
        let spectating = self.editor.flight_editor.telemetry.is_some();
        if !self.editor.editor_active && !spectating {
            app.renderer
                .render_sets
                .lines("editor_lines")
                .dynamic
                .clear();
            return;
        }
        let active = scene.cameras.active();
        let to_pixels =
            app.window.pixel_size.height as f32 / (app.window.size.height as f32).max(1.0);
        let view = TacticalView {
            camera_position: active.camera.position().coords,
            fovy_deg: active.projection.fovy,
            screen_height: app.window.size.height as f32 * to_pixels,
            focus: active.camera.position().coords,
        };
        let aspect = app.window.size.width as f32 / (app.window.size.height as f32).max(1.0);
        let editor = &self.editor.flight_editor;
        let selected = editor.selected_node();
        let mut highlighted: Vec<&str> = selected.into_iter().collect();
        if let Some(preview) = editor.preview_camera.as_deref() {
            highlighted.push(preview);
        }
        let mut lines = if self.editor.editor_active {
            self::gizmos::lines(
                scene,
                self.flight_manager.map(),
                scene.cameras.active_name(),
                selected,
                &highlighted,
                &view,
                app.particles.sea_level().unwrap_or(0.0),
                aspect,
            )
        } else {
            Vec::new()
        };
        // What the camera's on (see telemetry): an aircraft's path - the one
        // its autopilot's flying - with a line to the point it's making for;
        // a missile's trail and a line to its target.
        if spectating && !self.camera.focus().is_empty() {
            let sea_level = app.particles.sea_level().unwrap_or(0.0);
            if let Some(missile) = missile::get(scene, self.camera.focus()).map(Missile::telemetry) {
                for pair in missile.trail.windows(2).chain(std::iter::once(
                    [
                        *missile.trail.last().unwrap_or(&missile.position),
                        missile.position,
                    ]
                    .as_slice(),
                )) {
                    lines.push(LineInstance {
                        start: pair[0].into(),
                        end: pair[1].into(),
                        color: [1.0, 0.6, 0.2, 0.9],
                        width: 2.0,
                    });
                }
                if let Some(target) = missile
                    .target
                    .as_deref()
                    .and_then(|target| scene.world_transform(target))
                {
                    lines.push(LineInstance {
                        start: missile.position.into(),
                        end: target.position.into(),
                        color: [1.0, 0.3, 0.3, 0.6],
                        width: 1.5,
                    });
                }
            } else if let Some(mut live) = self.flight_manager.live(scene, self.camera.focus()) {
                let route = scene
                    .content
                    .nodes
                    .get(self.camera.focus())
                    .and_then(|node| node.get_behavior::<Plane>())
                    .and_then(Plane::route);
                let (points, looped) = match &route {
                    Some((points, next, looped)) => {
                        live.next_path_point = *next;
                        (
                            points
                                .iter()
                                .map(|point| (point.x, point.y, point.z))
                                .collect::<Vec<_>>(),
                            *looped,
                        )
                    }
                    None => self
                        .flight_manager
                        .map()
                        .find(self.camera.focus())
                        .map(|(_, placement)| self.flight_manager.map().flight_plan(placement))
                        .unwrap_or_default(),
                };
                tactical_map::flight_plan(
                    &mut lines,
                    &points,
                    looped,
                    Some(&live),
                    sea_level,
                    &view,
                );
            }
        }
        // The shown animation's camera keys, each where it puts the camera.
        if let Some(animation) = editor
            .current_animation
            .as_ref()
            .filter(|_| self.editor.editor_active)
            .and_then(|current| Some(current))
            .as_ref()
            .and_then(|(node, name)| self.flight_manager.map().animation(node, name))
        {
            let picked = editor.picked_camera_key();
            for (index, key) in animation.camera.iter().enumerate() {
                if let Some(pose) = animations::key_pose(key, scene) {
                    self::gizmos::camera_key(
                        &mut lines,
                        pose.position,
                        pose.rotation,
                        pose.fovy,
                        picked == Some(index),
                        &view,
                        aspect,
                    );
                }
            }
        }
        app.renderer.render_sets.lines("editor_lines").dynamic = lines;
    }
}
