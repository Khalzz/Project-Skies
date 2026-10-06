//! Nodes' animations running (see play::map's `AnimationDef`): each on its
//! own clock - started with the level (`on_start`), by another's event, or
//! from the editor; round again if `looped`. While one runs its node follows
//! its path bars (see play::path_follow), its camera keys put the camera
//! where they say, and its events fire as it passes them.

use std::collections::HashSet;
use std::sync::mpsc::Sender;

use nalgebra::{UnitQuaternion, Vector3};

use crate::engine::physics::physics_handler::PhysicsCommand;
use crate::engine::scene_manager::properties::{rotation_from_degrees, Transform3D};
use crate::engine::scene_manager::scene::Scene;
use crate::engine::scene_manager::scene_nodes::facing;
use crate::engine::utils::lerps::{lerp, smoothstep};

use super::map::{AnimAction, AnimationDef, CameraKey, FollowMode, MapSpec, Pace, Transition, WaveOn};
use super::path_follow::{FollowRequest, PathFollower};

/// Where an animation's camera is, and its lens.
#[derive(Clone, Copy, Debug)]
pub struct CameraPose {
    pub position: Vector3<f32>,
    pub rotation: UnitQuaternion<f32>,
    pub fovy: f32,
}

/// What the running animations did this frame, for the scene to carry out.
#[derive(Default)]
pub struct AnimationFrame {
    /// The events passed (other than starting and stopping animations -
    /// done here).
    pub events: Vec<AnimAction>,
    /// Where the camera is, while one with camera keys runs - the one
    /// started last.
    pub camera: Option<CameraPose>,
    /// The player's controls are locked.
    pub locks_input: bool,
    /// The HUD's hidden.
    pub hides_hud: bool,
    /// Nodes an animation's playing on - their pilots feel no G meanwhile.
    pub calm: HashSet<String>,
}

/// One animation on its clock.
pub struct Running {
    pub node: String,
    pub name: String,
    pub time: f32,
    /// Its clock running - otherwise held where it is (scrubbed in the
    /// editor), still driving its node and camera.
    pub playing: bool,
    // Events fired up to here - None: none yet (one right at 0 still fires).
    events_up_to: Option<f32>,
}

#[derive(Default)]
pub struct Animations {
    /// Running or held, oldest first - a later one's camera wins.
    running: Vec<Running>,
    follower: PathFollower,
    /// Seconds the camera wave has been moving - keeps going while an
    /// animation is held, so the drift shows as you set it up.
    wave_time: f32,
}

/// Where camera key `key` puts the camera now - an offset from its
/// `follow` (turned with it when `follow_rotation` is on; not there:
/// nowhere), turned to its `look_at`.
pub fn key_pose(key: &CameraKey, scene: &Scene) -> Option<CameraPose> {
    let local = Transform3D {
        position: Vector3::new(key.position.0, key.position.1, key.position.2),
        rotation: rotation_from_degrees(Vector3::new(key.rotation.0, key.rotation.1, key.rotation.2)),
        scale: Vector3::new(1.0, 1.0, 1.0),
    };
    let mut world = match &key.follow {
        Some(follow) => scene.world_transform(follow)?.compose(&local, key.follow_rotation),
        None => local,
    };
    if let Some(target) = key.look_at.as_ref().and_then(|target| scene.world_transform(target)) {
        if let Some(rotation) = facing(target.position - world.position) {
            world.rotation = rotation;
        }
    }
    Some(CameraPose { position: world.position, rotation: world.rotation, fovy: key.fov })
}

/// Where `animation`'s camera keys put the camera at `time` - the last key
/// reached held (a cut), or blended toward the next when that one comes in
/// smoothly. `None` before its first key.
pub fn camera_pose(animation: &AnimationDef, time: f32, wave_time: f32, scene: &Scene) -> Option<CameraPose> {
    let mut keys: Vec<&CameraKey> = animation.camera.iter().collect();
    keys.sort_by(|a, b| a.time.total_cmp(&b.time));
    let index = keys.iter().rposition(|key| key.time <= time)?;
    let from = key_pose(keys[index], scene)?;
    let Some(next) = keys.get(index + 1).filter(|next| next.transition == Transition::Smooth) else {
        return Some(apply_wave(from, wave_time, wave_setup(keys[index], &from, scene)));
    };
    let Some(to) = key_pose(next, scene) else {
        return Some(apply_wave(from, wave_time, wave_setup(keys[index], &from, scene)));
    };
    let t = smoothstep(((time - keys[index].time) / (next.time - keys[index].time).max(1e-4)).clamp(0.0, 1.0));
    let pose = CameraPose {
        position: from.position.lerp(&to.position, t),
        rotation: from.rotation.slerp(&to.rotation, t),
        fovy: lerp(from.fovy, to.fovy, t),
    };
    let from_wave = wave_setup(keys[index], &from, scene);
    let to_wave = wave_setup(next, &to, scene);
    let setup = WaveSetup {
        amount: lerp(from_wave.amount, to_wave.amount, t),
        speed: lerp(from_wave.speed, to_wave.speed, t),
        on: if t < 0.5 { from_wave.on } else { to_wave.on },
        objective: from_wave.objective.lerp(&to_wave.objective, t),
    };
    Some(apply_wave(pose, wave_time, setup))
}

/// How far in front of a camera with nothing to look at its objective sits (m).
const OBJECTIVE_DISTANCE: f32 = 40.0;

struct WaveSetup {
    amount: f32,
    speed: f32,
    on: WaveOn,
    objective: Vector3<f32>,
}

fn wave_setup(key: &CameraKey, pose: &CameraPose, scene: &Scene) -> WaveSetup {
    let objective = key.look_at.as_ref().and_then(|target| scene.world_transform(target)).map(|target| target.position).unwrap_or_else(|| pose.position + pose.rotation * Vector3::z() * OBJECTIVE_DISTANCE);
    WaveSetup {
        amount: if key.wave { key.wave_amount.max(0.0) } else { 0.0 },
        speed: key.wave_speed.max(0.0),
        on: key.wave_on,
        objective,
    }
}

/// The camera's wave: `setup.amount` meters of travel at `setup.speed`
/// (1 is a swing of a few seconds). On the camera, it drifts in its own
/// frame. On the objective, that point drifts and the camera turns to follow.
fn apply_wave(mut pose: CameraPose, time: f32, setup: WaveSetup) -> CameraPose {
    if setup.amount < 1e-3 || setup.speed < 1e-3 {
        return pose;
    }
    let t = time * setup.speed;
    let sway = |freq: f32, phase: f32| (t * freq + phase).sin();
    let lateral = (0.8 * sway(2.1, 0.4) + 0.2 * sway(3.7, 1.2)) * setup.amount;
    let vertical = (0.7 * sway(1.7, 2.0) + 0.3 * sway(2.9, 0.3)) * setup.amount;
    match setup.on {
        WaveOn::Position => {
            let dolly = 0.35 * sway(1.3, 1.5) * setup.amount;
            pose.position += pose.rotation * Vector3::new(lateral, vertical, dolly);
        }
        WaveOn::Objective => {
            let point = setup.objective + pose.rotation * Vector3::new(lateral, vertical, 0.0);
            if let Some(rotation) = facing(point - pose.position) {
                pose.rotation = rotation;
            }
        }
    }
    pose
}

impl Animations {
    /// The level's starting: every animation set to play `on_start` does.
    pub fn start_level(&mut self, map: &MapSpec) {
        for (node, animations) in &map.animations {
            for animation in animations.iter().filter(|animation| animation.on_start) {
                self.start(node, &animation.name);
            }
        }
    }

    fn index(&self, node: &str, name: &str) -> Option<usize> {
        self.running.iter().position(|running| running.node == node && running.name == name)
    }

    pub fn get(&self, node: &str, name: &str) -> Option<&Running> {
        self.index(node, name).map(|index| &self.running[index])
    }

    /// Node `node` has an animation playing (not merely held).
    pub fn playing_on(&self, node: &str) -> bool {
        self.running.iter().any(|running| running.node == node && running.playing)
    }

    /// Ends node `node`'s animations that are held. True when one was.
    pub fn stop_paused(&mut self, node: &str) -> bool {
        let before = self.running.len();
        self.running.retain(|running| !(running.node == node && !running.playing));
        self.running.len() != before
    }

    /// Plays node `node`'s animation `name` from its start.
    pub fn start(&mut self, node: &str, name: &str) {
        self.stop(node, name);
        self.running.push(Running { node: node.to_owned(), name: name.to_owned(), time: 0.0, playing: true, events_up_to: None });
    }

    /// Plays it on from where it's held - from its start if it isn't.
    pub fn resume(&mut self, node: &str, name: &str, length: f32) {
        match self.index(node, name) {
            Some(index) => {
                let running = &mut self.running[index];
                if running.time >= length {
                    running.time = 0.0;
                }
                running.events_up_to = (running.time > 0.0).then_some(running.time);
                running.playing = true;
            }
            None => self.start(node, name),
        }
    }

    pub fn pause(&mut self, node: &str, name: &str) {
        if let Some(index) = self.index(node, name) {
            self.running[index].playing = false;
        }
    }

    /// Ends it - everything it held let go.
    pub fn stop(&mut self, node: &str, name: &str) {
        self.running.retain(|running| !(running.node == node && running.name == name));
    }

    /// Holds it at `time` (the editor's scrubbing) - its node and camera
    /// shown as they are then, no events.
    pub fn seek(&mut self, node: &str, name: &str, time: f32) {
        let time = time.max(0.0);
        match self.index(node, name) {
            Some(index) => {
                let running = &mut self.running[index];
                running.time = time;
                running.playing = false;
                running.events_up_to = Some(time);
            }
            None => self.running.push(Running { node: node.to_owned(), name: name.to_owned(), time, playing: false, events_up_to: Some(time) }),
        }
    }

    /// An animation playing right now has the player's controls locked.
    pub fn locks_input(&self, map: &MapSpec) -> bool {
        self.running.iter().any(|running| running.playing && map.animation(&running.node, &running.name).is_some_and(|animation| animation.lock_input))
    }

    /// One frame - see `AnimationFrame`. Call after the behaviors (see
    /// SceneBehaviour::late_update).
    pub fn update(&mut self, delta_time: f32, scene: &mut Scene, map: &MapSpec, physics_command_tx: Option<&Sender<PhysicsCommand>>) -> AnimationFrame {
        let mut frame = AnimationFrame::default();
        self.wave_time = (self.wave_time + delta_time).rem_euclid(10_000.0);
        let wave_time = self.wave_time;
        self.running.retain(|running| map.animation(&running.node, &running.name).is_some());

        let mut finished = Vec::new();
        let mut started = Vec::new();
        let mut stopped = Vec::new();
        let mut requests = Vec::new();
        for running in self.running.iter_mut() {
            let Some(animation) = map.animation(&running.node, &running.name) else { continue };
            let length = animation.length.max(0.01);
            let mut ended = false;
            if running.playing {
                running.time += delta_time;
                let mut passed: Vec<&AnimAction> = animation.events.iter()
                    .filter(|event| running.events_up_to.is_none_or(|up_to| event.time > up_to) && event.time <= running.time)
                    .map(|event| &event.action)
                    .collect();
                running.events_up_to = Some(running.time);
                if running.time >= length {
                    if animation.looped {
                        running.time -= length;
                        // Round again: what's at its start fires again.
                        passed.extend(animation.events.iter().filter(|event| event.time <= running.time).map(|event| &event.action));
                        running.events_up_to = Some(running.time);
                    } else {
                        running.time = length;
                        ended = true;
                    }
                }
                for action in passed {
                    match action {
                        AnimAction::StartAnimation { node, animation } => started.push((node.clone(), animation.clone())),
                        AnimAction::StopAnimation { node, animation } => stopped.push((node.clone(), animation.clone())),
                        other => frame.events.push(other.clone()),
                    }
                }
                frame.locks_input |= animation.lock_input;
                frame.hides_hud |= animation.hide_hud;
                frame.calm.insert(running.node.clone());
            }
            if ended {
                finished.push((running.node.clone(), running.name.clone()));
                continue;
            }

            let time = running.time;
            for clip in &animation.paths {
                if time < clip.start || time >= clip.end.max(clip.start + 0.01) {
                    continue;
                }
                let pace = match clip.mode {
                    // The whole path from its start to its end.
                    FollowMode::Positioned => Pace::Duration((clip.end - clip.start).max(0.01)),
                    FollowMode::Flown => clip.speed_kt.map(Pace::Speed).unwrap_or(Pace::BySkill),
                };
                requests.push((running.node.clone(), FollowRequest {
                    path: clip.path.clone(),
                    mode: clip.mode,
                    pace,
                    skill: clip.skill,
                    orient: clip.orient,
                    elapsed: time - clip.start,
                    key: (format!("{}/{}", running.node, running.name), clip.start),
                    held: !running.playing,
                }));
            }
            if let Some(pose) = camera_pose(animation, time, wave_time, scene) {
                frame.camera = Some(pose);
            }
        }
        for (node, name) in finished {
            self.stop(&node, &name);
        }
        for (node, name) in stopped {
            self.stop(&node, &name);
        }
        for (node, name) in started {
            if map.animation(&node, &name).is_some() {
                self.start(&node, &name);
            }
        }
        self.follower.update(delta_time, scene, map, requests, physics_command_tx);
        frame
    }
}
