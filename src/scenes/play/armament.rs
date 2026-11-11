//! The player's weapons - the master mode, radar and seeker
//! (`fire_control`), the weapon picked, locking, firing and the launch
//! hold, the countermeasures and bingo cues (`update_weapons`) - and the
//! missiles' flight and blasts, the bots' defenses against them. Its state
//! is `Armament`, the scene's `weapons`.

use super::*;

pub(super) struct Armament {
    // The player's master mode, radar and seeker - see update_weapons.
    pub(super) fire_control: FireControl,
    // Every weapon's data read so far, by id.
    pub(super) weapon_specs: HashMap<String, WeaponSpec>,
    // Which side the player last fired from (+1 left, -1 right) - the next
    // goes from the other.
    pub(super) last_fired_side: f32,
    // How long fire_weapon has been held toward a launch (s), for which
    // weapon, and whether this press already fired.
    pub(super) launch_hold: f32,
    pub(super) launch_hold_weapon: Option<String>,
    pub(super) launch_spent: bool,
    // The weapons line on the HUD - see update_weapons.
    pub(super) weapons_status: String,
    // The countermeasures line on the data box. The column is what's left
    // and the release setting - the HUD itself only flashes the setting
    // when it changes (see cms_cue).
    pub(super) countermeasures_status: String,
    pub(super) countermeasures_column: Option<(u32, u32, Release)>,
    // The setting just chosen, and the game time it was chosen - the HUD
    // blinks it, holds it, then fades it.
    pub(super) cms_cue: Option<(String, f64)>,
    // Fuel has already crossed bingo, so the four-second BINGO cue doesn't
    // start again every frame. Cleared once the tanks are above bingo or empty.
    pub(super) bingo_latched: bool,
    // Game time until which the WARN text reads BINGO.
    pub(super) bingo_warn_until: Option<f64>,
    // With a lock: the range scale's numbers - see update_hud.
    pub(super) range_cue: Option<RangeCue>,
    // The seeker's symbol this frame (its diamond, or the box on what it's
    // locked) - set by update_weapons, drawn by update_hud.
    pub(super) hud_symbol: Option<HudSymbol>,
    // Where the pilot looks while the helmet display is up (a world
    // direction) - its cross; None otherwise. Set by update_hud (after the
    // camera has moved), read by the next update_weapons to point the
    // seeker in the Fox-2 mode (see FireControl::cue).
    pub(super) hmd_cue: std::cell::Cell<Option<Vector3<f32>>>,
    // The vertical lock is on: the first enemy through the strip over the
    // nose gets locked (see fire_control::in_vertical_scan) - for as long as
    // the lock key is held, see update_weapons.
    pub(super) vertical_scan: bool,
    // How long the lock key has been held (s), and whether this hold already
    // went into the vertical lock - so letting go isn't a tap too.
    pub(super) lock_held: f32,
    pub(super) lock_hold_used: bool,
    // The weapon picked from the list (a weapon id) - None: OFF, nothing
    // armed. See update_weapons.
    pub(super) selected_weapon: Option<String>,
    // The HUD's weapon list as it should read: each entry's text, and
    // whether it's the selected one - see play_ui::build_weapon_list.
    pub(super) weapon_list: Vec<(String, bool)>,
}

/// How a weapon reads on the HUD - its folder name, "missiles/aim-9x" ->
/// "AIM-9X".
pub(super) fn weapon_label(id: &str) -> String {
    id.rsplit('/').next().unwrap_or(id).to_uppercase()
}

impl Play {
    /// The weapons, every frame the game runs: the master mode keys (1 NAV,
    /// 2 air-to-air - again for DOGFIGHT/BVR - 3 air-to-ground), the seeker
    /// and its lock key (see play::weapon::fire_control), the trigger, the
    /// missiles' plumes (they're flown by fly_missiles), and the HUD's symbols and
    /// weapons line. `has_control`: the player can fly (not in a scripted
    /// sequence, not typing in the editor).
    pub(super) fn update_weapons(&mut self, scene: &mut Scene, app: &mut App, has_control: bool) {
        let delta_time = app.time.delta_time;
        let player_id = self.flight_manager.player_id().to_owned();
        let aircraft = self.aircraft_tracks(scene, true);

        // The data of every weapon hung anywhere - read once.
        for (_, placement) in self.flight_manager.map().aircraft() {
            for weapon in placement.loadout.values() {
                if !self.weapons.weapon_specs.contains_key(weapon) {
                    if let Ok(spec) = WeaponSpec::read(weapon) {
                        self.weapons.weapon_specs.insert(weapon.clone(), spec);
                    }
                }
            }
        }

        // Plumes: off on the pylons, lit on the missiles burning.
        let mut model_files: Vec<String> = self
            .weapons
            .weapon_specs
            .values()
            .map(|spec| crate::asset_relative(&spec.model.model))
            .collect();
        model_files.sort_unstable();
        model_files.dedup();
        self.missiles.update_plumes(scene, &model_files, delta_time);

        // The player's jet.
        let player_alive = scene
            .content
            .nodes
            .get(&player_id)
            .and_then(|node| node.get_behavior::<Plane>())
            .is_some_and(|plane| !plane.wrecked && !plane.pilot.is_unconscious());
        let own = aircraft
            .iter()
            .find(|track| track.name == player_id)
            .and_then(|track| {
                let rotation = crate::ext::drawn::get(scene, &player_id)?
                    .instance
                    .transform
                    .rotation;
                Some(OwnShip {
                    position: track.position,
                    rotation,
                    velocity: track.velocity,
                })
            });
        let player_coalition = self
            .flight_manager
            .coalition_of(&player_id)
            .map(str::to_owned);
        let contacts: Vec<Contact> = aircraft
            .iter()
            .filter(|track| {
                track.name != player_id
                    && self
                        .flight_manager
                        .coalition_of(&track.name)
                        .map(str::to_owned)
                        != player_coalition
            })
            .map(|track| Contact {
                name: track.name.clone(),
                position: track.position,
                velocity: track.velocity,
                falling: self.falling(scene, &track.name),
            })
            .collect();

        // The weapon list: OFF, then each kind of weapon still on the jet,
        // in pylon order - with how many are left.
        let hung = self.flight_manager.hung_weapons(&player_id);
        let mut kinds: Vec<(String, usize)> = Vec::new();
        for (_, weapon, _) in &hung {
            match kinds.iter_mut().find(|(kind, _)| kind == weapon) {
                Some((_, count)) => *count += 1,
                None => kinds.push((weapon.clone(), 1)),
            }
        }
        // The last one fired: back to OFF.
        if self
            .weapons
            .selected_weapon
            .as_ref()
            .is_some_and(|selected| !kinds.iter().any(|(kind, _)| kind == selected))
        {
            self.weapons.selected_weapon = None;
        }
        // The next one down the list, round to OFF again.
        if has_control && player_alive && app.input.is_action_just_pressed("next_weapon") {
            let current = self
                .weapons
                .selected_weapon
                .as_ref()
                .and_then(|selected| kinds.iter().position(|(kind, _)| kind == selected));
            let next = match current {
                None => 0,
                Some(index) => index + 1,
            };
            self.weapons.selected_weapon = kinds.get(next).map(|(kind, _)| kind.clone());
            self.weapons.fire_control.drop_locks();
        }
        self.weapons.weapon_list = kinds
            .iter()
            .map(|(kind, count)| {
                (
                    format!("{} x{count}", weapon_label(kind)),
                    self.weapons.selected_weapon.as_ref() == Some(kind),
                )
            })
            .collect();

        // The selected weapon's data, and every pylon carrying one.
        let selected = self.weapons.selected_weapon.as_ref().and_then(|weapon| {
            Some((
                weapon.clone(),
                self.weapons.weapon_specs.get(weapon)?.clone(),
            ))
        });
        let ready: Vec<(String, Vector3<f32>)> = selected
            .as_ref()
            .map(|(weapon, _)| {
                hung.iter()
                    .filter(|(_, hung_weapon, _)| hung_weapon == weapon)
                    .map(|(pylon, _, offset)| (pylon.clone(), *offset))
                    .collect()
            })
            .unwrap_or_default();
        // What its seeker looks with - a guided missile's.
        let seeker = selected
            .as_ref()
            .filter(|(_, spec)| {
                spec.category == WeaponCategory::Missile && spec.guidance.seeker != Seeker::None
            })
            .map(|(_, spec)| spec.guidance.clone());

        self.weapons.range_cue = None;
        let Some(own) = own.filter(|_| player_alive) else {
            self.weapons.fire_control.drop_locks();
            self.weapons.weapons_status = String::new();
            self.weapons.countermeasures_status = String::new();
            self.weapons.countermeasures_column = None;
            self.weapons.bingo_latched = false;
            self.weapons.bingo_warn_until = None;
            self.weapons.hud_symbol = None;
            self.weapons.vertical_scan = false;
            return;
        };

        // Countermeasures: the key choosing flares, chaff or both, one
        // release, the burst program - and the program's releases due now.
        let mut released = Vec::new();
        let (status, column) = match self.flight_manager.countermeasures_mut(&player_id) {
            Some(set) => {
                if has_control && app.input.is_action_just_pressed("countermeasure_mode") {
                    set.release = set.release.next();
                    self.weapons.cms_cue = Some((set.release.label().to_owned(), self.game_time));
                }
                if has_control && app.input.is_action_just_pressed("countermeasure_single") {
                    released.extend(set.release_one());
                }
                if has_control && app.input.is_action_just_pressed("countermeasure_program") {
                    released.extend(set.start_program());
                }
                released.extend(set.update(delta_time));
                (
                    format!(
                        "FLR {}  CHF {}  {}",
                        set.flares,
                        set.chaff,
                        set.release.label()
                    ),
                    Some((set.flares, set.chaff, set.release)),
                )
            }
            None => (String::new(), None),
        };
        self.weapons.countermeasures_status = status;
        self.weapons.countermeasures_column = column;
        let (fuel_kg, bingo_kg) = scene
            .content
            .nodes
            .get(&player_id)
            .and_then(|node| node.get_behavior::<Plane>())
            .map(|plane| (plane.fuel().0, plane.bingo_kg))
            .unwrap_or((0.0, None));
        let in_bingo = bingo_kg.is_some_and(|bingo| fuel_kg <= bingo && fuel_kg > 0.0);
        if in_bingo && !self.weapons.bingo_latched {
            self.weapons.bingo_warn_until = Some(self.game_time + 4.0);
        }
        if !in_bingo || fuel_kg <= 0.0 {
            self.weapons.bingo_warn_until = None;
        }
        self.weapons.bingo_latched = in_bingo;
        for release in released {
            self.countermeasures.release(
                scene,
                app,
                release,
                own.position,
                own.rotation,
                own.velocity,
            );
        }
        // The Fox-2 mode (a heat-seeker selected): the helmet display, while
        // it's up, points the seeker where the pilot looks - see draw_hmd.
        let fox2 = selected.as_ref().is_some_and(|(_, spec)| {
            spec.category == WeaponCategory::Missile && spec.guidance.seeker == Seeker::InfraRed
        });
        self.weapons.fire_control.cue = if fox2 {
            self.weapons.hmd_cue.get()
        } else {
            None
        };
        // The lock key: lets the lock go, or locks the seeker on what's where
        // it rests (in the diamond - the nose, or the helmet's cross).
        // Held past VERTICAL_SCAN_HOLD_SECONDS instead, it's the vertical
        // lock, for as long as it stays held - letting go ends it, and the
        // key is back to its taps (a lock it caught stays). So a tap acts
        // when it's let go, and a hold isn't a tap too.
        const VERTICAL_SCAN_HOLD_SECONDS: f32 = 0.5;
        if has_control && app.input.is_action_pressed("toggle_lock") {
            self.weapons.lock_held += delta_time;
            if !self.weapons.lock_hold_used
                && self.weapons.lock_held >= VERTICAL_SCAN_HOLD_SECONDS
                && seeker.is_some()
            {
                self.weapons.lock_hold_used = true;
                self.weapons.fire_control.drop_locks();
                self.weapons.vertical_scan = true;
            }
        } else {
            if self.weapons.lock_held > 0.0 && !self.weapons.lock_hold_used {
                if self.weapons.fire_control.seeker_target.is_some() {
                    self.weapons.fire_control.drop_locks();
                } else if let Some(seeker) = &seeker {
                    self.weapons.fire_control.lock(&own, &contacts, seeker);
                }
            }
            self.weapons.vertical_scan = false;
            self.weapons.lock_held = 0.0;
            self.weapons.lock_hold_used = false;
        }
        // The vertical lock: the first enemy through the strip over the nose,
        // in range, is locked, and that's it done for this hold. Without a
        // seeker (another weapon, or none) there's nothing to lock with.
        match &seeker {
            Some(seeker) if self.weapons.vertical_scan => {
                if let Some(name) = fire_control::in_vertical_scan(&own, &contacts, seeker) {
                    self.weapons.fire_control.seeker_target = Some(name);
                    self.weapons.vertical_scan = false;
                }
            }
            _ => self.weapons.vertical_scan = false,
        }
        self.weapons
            .fire_control
            .update(&own, &contacts, seeker.as_ref(), delta_time);
        let locked = self
            .weapons
            .fire_control
            .locked(&contacts, &own)
            .map(str::to_owned);
        let locked_contact = locked
            .as_ref()
            .and_then(|name| contacts.iter().find(|contact| contact.name == *name));
        let shoot = match (locked_contact, &selected) {
            (Some(contact), Some((_, spec))) => {
                fire_control::in_range(&own, contact, spec.guidance.lock_range)
            }
            _ => false,
        };
        self.weapons.range_cue = match (locked_contact, &selected) {
            (Some(contact), Some((_, spec))) => Some(RangeCue {
                range: (contact.position - own.position).magnitude(),
                min_range: fire_control::MIN_RANGE,
                max_range: fire_control::max_range(&own, contact, spec.guidance.lock_range),
                scale_top: spec.guidance.lock_range,
                in_range: shoot,
            }),
            _ => None,
        };

        // The trigger: hold it for the missile's `hold_to_launch`, then the
        // next one off - from the other side than the last, outboard first -
        // at whatever the seeker's locked on (or nothing: it flies on
        // straight). Releasing early cancels. One launch per press.
        let selected_id = selected.as_ref().map(|(weapon, _)| weapon.clone());
        let holding = has_control && player_alive && app.input.is_action_pressed("fire_weapon");
        if !holding || self.weapons.launch_hold_weapon.as_ref() != selected_id.as_ref() {
            self.weapons.launch_hold = 0.0;
            self.weapons.launch_spent = false;
            self.weapons.launch_hold_weapon = holding.then(|| selected_id.clone()).flatten();
        } else if !self.weapons.launch_spent {
            self.weapons.launch_hold += delta_time;
        }
        let hold_for = selected
            .as_ref()
            .map(|(_, spec)| spec.hold_to_launch)
            .unwrap_or(0.0);
        if holding && !self.weapons.launch_spent && self.weapons.launch_hold >= hold_for {
            self.weapons.launch_spent = true;
            let outboard = |a: &&(String, Vector3<f32>), b: &&(String, Vector3<f32>)| {
                a.1.x.abs().total_cmp(&b.1.x.abs())
            };
            let other_side = ready
                .iter()
                .filter(|(_, offset)| offset.x.signum() != self.weapons.last_fired_side)
                .max_by(outboard);
            let next = other_side
                .or_else(|| ready.iter().max_by(outboard))
                .cloned();
            match (next, &selected) {
                (Some((pylon, offset)), Some((_, spec)))
                    if spec.category == WeaponCategory::Missile =>
                {
                    if let Some((position, rotation)) =
                        self.flight_manager.release(scene, app, &player_id, &pylon)
                    {
                        self.weapons.last_fired_side = offset.x.signum();
                        // Its lock - or, without one, whatever its seeker
                        // sees in the diamond coming off the rail.
                        let target = locked.clone().or_else(|| {
                            seeker.as_ref().and_then(|seeker| {
                                fire_control::in_seeker_view(
                                    &own,
                                    &contacts,
                                    seeker,
                                    self.weapons.fire_control.rest_direction(&own, seeker),
                                )
                            })
                        });
                        self.missiles.launch(
                            scene,
                            app,
                            &player_id,
                            spec.clone(),
                            position,
                            rotation,
                            own.velocity,
                            target,
                        );
                    }
                }
                (_, Some((_, spec))) => println!(
                    "{} can't be fired yet - only missiles fly so far",
                    weapon_label(&spec.id)
                ),
                (_, None) => println!("No weapon selected"),
            }
        }

        // The HUD's weapons line.
        self.weapons.weapons_status = match &selected {
            Some((weapon, _)) => {
                let state = if shoot {
                    "SHOOT"
                } else if locked.is_some() {
                    "LOCK"
                } else {
                    ""
                };
                format!("{}  {state}", weapon_label(weapon))
            }
            None => "WPN OFF".to_owned(),
        };

        // The HUD's symbol: locked, a square on the target (flashing while
        // it's in range - fire!) and nothing else; unlocked, the seeker's
        // diamond out ahead where it looks - the nose. Drawn by update_hud,
        // once the camera has moved.
        self.weapons.hud_symbol = None;
        if seeker.is_none() || self.pilot_view.hud_hidden_by_g {
            return;
        }
        let up = own.rotation * Vector3::y();
        let target = self
            .weapons
            .fire_control
            .seeker_target
            .as_ref()
            .and_then(|name| contacts.iter().find(|contact| contact.name == *name));
        match target {
            Some(target) => {
                let flash_off = shoot && (self.game_time * 4.0).fract() > 0.5;
                if !flash_off {
                    self.weapons.hud_symbol = Some(HudSymbol {
                        point: target.position,
                        track: Some(target.name.clone()),
                        up,
                        half_size: 18.0,
                        diamond: false,
                    });
                }
            }
            None => {
                // The HUD's diamond is the boresight - fixed on the nose. While
                // the helmet display is up in the Fox-2 mode, the seeker looks
                // where the pilot does instead, shown on the helmet (see
                // draw_hmd).
                let nose_point = own.position + own.boresight() * 3000.0;
                self.weapons.hud_symbol = Some(HudSymbol {
                    point: nose_point,
                    track: None,
                    up,
                    half_size: 11.0,
                    diamond: true,
                });
            }
        }
    }

    /// With a lock on something the camera can't see (off screen, or
    /// behind it): an arrow near the screen's edge pointing the way to turn
    /// to it - in every view, not over F10's map. Drawn into the HUD's
    /// screen lines, after update_hud has filled them.
    pub(super) fn update_lock_arrow(&self, scene: &Scene, app: &mut App) {
        const COLOR: [f32; 4] = [0.0, 1.0, 0.3, 0.95];
        const WIDTH: f32 = 2.0;
        // How far in from the screen's edge it sits, and its size (px).
        const EDGE_INSET: f32 = 70.0;
        const LENGTH: f32 = 22.0;
        const HALF_WIDTH: f32 = 11.0;
        // With the HUD fixed to the jet, the HUD's own arrow (at its edge)
        // and the helmet display's (by its cross) point the way instead.
        if (self.tactical.tactical_map_active || self.editor.editor_active)
            || self.pilot_view.hud_hidden_by_g
            || self.hud_on_plate(scene)
        {
            return;
        }
        let Some(target) = self
            .weapons
            .fire_control
            .seeker_target
            .as_ref()
            .and_then(|name| crate::ext::drawn::get(scene, name))
            .map(|instance| instance.instance.transform.position)
        else {
            return;
        };
        let active = scene.cameras.active();
        let (width, height) = (app.window.size.width, app.window.size.height);
        if app
            .renderer
            .camera_resources
            .world_to_screen(active, Point3::from(target), width, height)
            .is_some()
        {
            // In view - its square shows where it is.
            return;
        }

        // Which way it is across the screen: the camera's own right and up.
        let forward =
            (active.camera.rotation_modifier * active.camera.calc_forward_direction()).normalize();
        let up = active.camera.rotation_modifier * active.camera.up;
        let Some(right) = forward.cross(&up).try_normalize(1e-6) else {
            return;
        };
        let up = right.cross(&forward);
        let to_target = target - active.camera.position().coords;
        // Screen's y runs down; straight behind, point down (turn round).
        let direction = nalgebra::Vector2::new(to_target.dot(&right), -to_target.dot(&up))
            .try_normalize(1e-6)
            .unwrap_or_else(|| nalgebra::Vector2::new(0.0, 1.0));

        // From the middle out to just inside the edge, that way.
        let (half_width, half_height) = (width as f32 * 0.5, height as f32 * 0.5);
        let reach_x = if direction.x.abs() > 1e-6 {
            (half_width - EDGE_INSET) / direction.x.abs()
        } else {
            f32::MAX
        };
        let reach_y = if direction.y.abs() > 1e-6 {
            (half_height - EDGE_INSET) / direction.y.abs()
        } else {
            f32::MAX
        };
        let center = nalgebra::Vector2::new(half_width, half_height)
            + direction * reach_x.min(reach_y).max(0.0);
        let across = nalgebra::Vector2::new(-direction.y, direction.x);
        let tip = center + direction * LENGTH * 0.5;
        let base = center - direction * LENGTH * 0.5;
        let corners = [tip, base + across * HALF_WIDTH, base - across * HALF_WIDTH];

        let to_pixels = app.window.pixel_size.height as f32 / (height as f32).max(1.0);
        for index in 0..3 {
            let (start, end) = (
                corners[index] * to_pixels,
                corners[(index + 1) % 3] * to_pixels,
            );
            app.renderer
                .render_sets
                .lines("hud_screen_lines")
                .dynamic
                .push(LineInstance {
                    start: [start.x, start.y, 0.0],
                    end: [end.x, end.y, 0.0],
                    color: COLOR,
                    width: WIDTH,
                });
        }
    }

    /// Every aircraft still flying - where it is, how it's moving, how hot
    /// it is (its engine and afterburner, or its fire - see
    /// play::weapon::heat) - and those shot down and falling, still hot
    /// (what heat seekers see), if `with_falling`.
    pub(super) fn aircraft_tracks(&self, scene: &Scene, with_falling: bool) -> Vec<TargetTrack> {
        self.flight_manager
            .map()
            .aircraft()
            .filter_map(|(_, placement)| {
                let live = self.flight_manager.live(scene, &placement.name)?;
                if live.wrecked && !(with_falling && self.falling(scene, &placement.name)) {
                    return None;
                }
                let node = scene.content.nodes.get(&placement.name);
                let state = node.and_then(|node| node.physics_state::<AircraftState>());
                let velocity = state
                    .map(|state| state.flight_data.velocity)
                    .unwrap_or_else(Vector3::zeros);
                let afterburner = state.map(|state| state.afterburner).unwrap_or(0.0);
                let plane = node.and_then(|node| node.get_behavior::<Plane>());
                let throttle = plane.map(|plane| plane.controls.throttle).unwrap_or(0.0);
                let burning = plane.is_some_and(|plane| plane.wrecked && !plane.in_water());
                let heat = heat::aircraft_heat(throttle, afterburner, burning);
                // A broken surface's slab is gone, so a missile can't fuze on it.
                let gone = state
                    .map(|state| {
                        state
                            .profiles
                            .iter()
                            .filter(|(_, hp)| *hp <= 0.0)
                            .map(|(label, _)| label.clone())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let colliders: Vec<FuzeCollider> = plane
                    .map(|plane| {
                        plane
                            .fuze_colliders()
                            .iter()
                            .filter(|collider| match collider {
                                ColliderType::ConvexHull {
                                    wing: Some(label), ..
                                } => !gone.iter().any(|gone| gone == label),
                                _ => true,
                            })
                            .filter_map(FuzeCollider::from_collider)
                            .collect()
                    })
                    .unwrap_or_default();
                let reach = fuze::reach_of(&colliders);
                Some(TargetTrack {
                    name: placement.name.clone(),
                    position: live.position,
                    velocity,
                    heat,
                    decoy: false,
                    rotation: live.rotation,
                    colliders,
                    reach,
                })
            })
            .collect()
    }

    /// The bots defending themselves: each one with flares bursts them once
    /// it's noticed a heat seeker homing on it - how quickly, and how often
    /// again, its pilot's difficulty says (see play::ai_defense) - and its
    /// bursts in progress go on. Once a frame the game runs.
    pub(super) fn update_ai_countermeasures(&mut self, scene: &mut Scene, app: &mut App) {
        let delta_time = app.time.delta_time;
        let homing = self.missiles.heat_seekers_homing();
        let bots: Vec<(String, Difficulty)> = self
            .flight_manager
            .map()
            .aircraft()
            .filter(|(_, placement)| !placement.player)
            .map(|(_, placement)| (placement.name.clone(), placement.difficulty))
            .collect();
        for (name, difficulty) in bots {
            let flying = scene
                .content
                .nodes
                .get(&name)
                .and_then(|node| node.get_behavior::<Plane>())
                .is_some_and(|plane| !plane.wrecked);
            if !flying {
                self.ai_defense.forget(&name);
                continue;
            }
            let incoming: Vec<String> = homing
                .iter()
                .filter(|(_, target)| *target == name)
                .map(|(id, _)| id.clone())
                .collect();
            let burst = self
                .ai_defense
                .update(&name, difficulty, &incoming, delta_time);
            let Some(set) = self.flight_manager.countermeasures_mut(&name) else {
                continue;
            };
            let mut released = Vec::new();
            if burst {
                set.release = Release::Flares;
                released.extend(set.start_program());
            }
            released.extend(set.update(delta_time));
            if released.is_empty() {
                continue;
            }
            let Some(transform) = crate::ext::drawn::get(scene, &name)
                .map(|instance| instance.instance.transform)
            else {
                continue;
            };
            let velocity = scene
                .content
                .nodes
                .get(&name)
                .and_then(|node| node.physics_state::<AircraftState>())
                .map(|state| state.flight_data.velocity)
                .unwrap_or_else(Vector3::zeros);
            for release in released {
                self.countermeasures.release(
                    scene,
                    app,
                    release,
                    transform.position,
                    transform.rotation,
                    velocity,
                );
            }
        }
    }

    /// Flies every missile one frame and wrecks what they hit - early in
    /// the frame, before the camera reads where they are (see `update`), so
    /// a camera riding one moves with it, not a frame behind.
    pub(super) fn fly_missiles(&mut self, scene: &mut Scene, app: &mut App) {
        // Falling wrecks too: a missile chasing one keeps on its heat. And
        // the flares burning - heat sources it can be pulled onto.
        self.countermeasures.update(scene, app, app.time.delta_time);
        let mut aircraft = self.aircraft_tracks(scene, true);
        aircraft.extend(self.countermeasures.heat_tracks());
        let sun = Self::sun_direction(scene);
        let events = self.missiles.update(
            scene,
            app,
            &aircraft,
            &self.world_collision,
            sun,
            app.time.delta_time,
        );
        self.deliver_blasts(scene, &events.blasts);
    }

    /// Every blast onto every aircraft still flying. The physics half spends
    /// wing HP and notices a seat inside the sphere; a wing hit does not
    /// shot-down the jet.
    pub(super) fn deliver_blasts(&mut self, scene: &mut Scene, blasts: &[BlastEvent]) {
        let names: Vec<String> = self
            .flight_manager
            .map()
            .aircraft()
            .map(|(_, placement)| placement.name.clone())
            .collect();
        for blast in blasts {
            for name in &names {
                let Some(node) = scene.content.nodes.get_mut(name) else {
                    continue;
                };
                if node
                    .get_behavior::<Plane>()
                    .is_some_and(|plane| plane.wrecked)
                {
                    continue;
                }
                if let Some(plane) = node.get_behavior_mut::<Plane>() {
                    plane.note_blast(&blast.shooter, &blast.weapon);
                }
                node.push_physics_event(AircraftEvent::Blast {
                    at: blast.at,
                    radius: blast.radius,
                    damage: blast.damage,
                    impulse: blast.impulse,
                });
            }
        }
    }
}
