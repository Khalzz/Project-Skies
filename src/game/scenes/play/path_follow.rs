//! Nodes following the map's paths (see play::map's `PathSpec`): what a
//! running animation's path bar has them follow (see play::animations), or
//! a map node's own path (`NodeSpec::follow`) whenever nothing else has it
//! on another.
//!
//! A follow is either:
//! - positioned: the node's placed along the path by time, at its pace,
//!   turned along it - its body (if it has one) held still meanwhile, and
//!   let go at the speed it was going when the follow ends;
//! - flown: an aircraft's autopilot flies the path for real (see
//!   plane::autopilot), as well as its pilot's skill flies - when it ends,
//!   back to the aircraft's own flight plan.

use std::collections::HashMap;
use std::sync::mpsc::Sender;

use nalgebra::Vector3;

use crate::engine::game_nodes::game_object::Physics;
use crate::engine::physics::physics_handler::PhysicsCommand;
use crate::engine::scene_manager::scene::Scene;
use crate::engine::scene_manager::scene_nodes::facing;

use super::flight_manager::cruise_speed;
use super::map::{Difficulty, FollowMode, MapSpec, Pace, PathSpec};
use super::pilot_skill::PilotSkill;
use super::plane::autopilot::Speed;
use super::plane::plane::Plane;

/// Knots to m/s.
const KT_TO_MS: f32 = 0.514_444;
/// Each smooth path's leg is cut into this many straight pieces to follow.
const SMOOTH_STEPS: usize = 16;

/// A path as it's followed: straight pieces (a smooth path's curve cut
/// fine), each point's distance along it, its length.
pub struct SampledPath {
    points: Vec<Vector3<f32>>,
    distances: Vec<f32>,
    length: f32,
    looped: bool,
}

impl SampledPath {
    /// `None` for a path with no points.
    pub fn new(path: &PathSpec) -> Option<SampledPath> {
        let mut corners: Vec<Vector3<f32>> = path.points.iter().map(|&(x, y, z)| Vector3::new(x, y, z)).collect();
        if corners.is_empty() {
            return None;
        }
        let looped = path.looped && corners.len() > 1;
        let points = if path.smooth && corners.len() > 2 {
            let count = corners.len();
            let at = |index: isize| -> Vector3<f32> {
                if looped {
                    corners[index.rem_euclid(count as isize) as usize]
                } else {
                    corners[index.clamp(0, count as isize - 1) as usize]
                }
            };
            let legs = if looped { count } else { count - 1 };
            let mut points = Vec::with_capacity(legs * SMOOTH_STEPS + 1);
            for leg in 0..legs as isize {
                let (a, b, c, d) = (at(leg - 1), at(leg), at(leg + 1), at(leg + 2));
                for step in 0..SMOOTH_STEPS {
                    let t = step as f32 / SMOOTH_STEPS as f32;
                    points.push(catmull_rom(a, b, c, d, t));
                }
            }
            points.push(if looped { corners[0] } else { corners[count - 1] });
            points
        } else {
            if looped {
                corners.push(corners[0]);
            }
            corners
        };
        let mut distances = Vec::with_capacity(points.len());
        let mut length = 0.0;
        for (index, point) in points.iter().enumerate() {
            if index > 0 {
                length += (point - points[index - 1]).magnitude();
            }
            distances.push(length);
        }
        Some(SampledPath { points, distances, length, looped })
    }

    pub fn length(&self) -> f32 {
        self.length
    }

    /// Where it is `distance` meters along (round again past the end for a
    /// looped one, held at the end otherwise), and which way it's heading
    /// there - `None` for the heading where it isn't going anywhere.
    pub fn at(&self, distance: f32) -> (Vector3<f32>, Option<Vector3<f32>>) {
        if self.points.len() == 1 || self.length <= 1e-3 {
            return (self.points[0], None);
        }
        let distance = if self.looped { distance.rem_euclid(self.length) } else { distance.clamp(0.0, self.length) };
        let index = self.distances.partition_point(|&along| along <= distance).clamp(1, self.points.len() - 1);
        let (a, b) = (self.points[index - 1], self.points[index]);
        let piece = (self.distances[index] - self.distances[index - 1]).max(1e-6);
        let t = ((distance - self.distances[index - 1]) / piece).clamp(0.0, 1.0);
        (a.lerp(&b, t), (b - a).try_normalize(1e-6))
    }

    /// Past its end - a path that doesn't loop, followed all the way.
    #[cfg(test)]
    pub fn finished(&self, distance: f32) -> bool {
        !self.looped && distance >= self.length
    }
}

fn catmull_rom(a: Vector3<f32>, b: Vector3<f32>, c: Vector3<f32>, d: Vector3<f32>, t: f32) -> Vector3<f32> {
    let t2 = t * t;
    let t3 = t2 * t;
    (b * 2.0 + (c - a) * t + (a * 2.0 - b * 5.0 + c * 4.0 - d) * t2 + (b * 3.0 - a - c * 3.0 + d) * t3) * 0.5
}

/// How fast `pace` goes along a path `length` long, flown with `skill`
/// (m/s).
pub fn pace_speed(pace: Pace, length: f32, skill: &PilotSkill) -> f32 {
    match pace {
        Pace::Speed(knots) => knots.max(0.0) * KT_TO_MS,
        Pace::Duration(seconds) => length / seconds.max(0.01),
        Pace::BySkill => skill.cruise_kt * KT_TO_MS,
    }
}

/// How long a body that was placed stays off the physics' pose after it's
/// let go (s) - until the physics has reported it from where it was let go.
const RELEASE_GRACE_SECONDS: f32 = 0.25;
/// The fastest a placed body's let go at (m/s) - a short timed path can
/// mean a silly speed, not one to hand the physics.
const MAX_RELEASE_SPEED: f32 = 350.0;

/// How far along `points` (straight legs, back to the first when `looped`)
/// each point is (m), and the whole length.
pub fn corner_distances(points: &[(f32, f32, f32)], looped: bool) -> (Vec<f32>, f32) {
    let point = |&(x, y, z): &(f32, f32, f32)| Vector3::new(x, y, z);
    let mut distances = Vec::with_capacity(points.len());
    let mut along = 0.0;
    for (index, corner) in points.iter().enumerate() {
        if index > 0 {
            along += (point(corner) - point(&points[index - 1])).magnitude();
        }
        distances.push(along);
    }
    let total = match (looped && points.len() > 1, points.first(), points.last()) {
        (true, Some(first), Some(last)) => along + (point(first) - point(last)).magnitude(),
        _ => along,
    };
    (distances, total)
}

/// What's left to fly of `points` from `distance` along them: the points
/// still ahead, in order - for a looped path all of them, starting with
/// the next one.
fn points_ahead(points: &[(f32, f32, f32)], looped: bool, distance: f32) -> Vec<(f32, f32, f32)> {
    let (distances, total) = corner_distances(points, looped);
    let distance = if looped && total > 0.0 { distance.rem_euclid(total) } else { distance };
    match distances.iter().position(|&along| along > distance + 1.0) {
        Some(next) if looped => points[next..].iter().chain(points[..next].iter()).copied().collect(),
        Some(next) => points[next..].to_vec(),
        None if looped => points.to_vec(),
        None => Vec::new(),
    }
}

/// Holds node `id` still for something else to move: the physics' pose
/// isn't written onto it, and its body (if it has one) goes kinematic.
fn hold_node(scene: &mut Scene, id: &str, physics_command_tx: Option<&Sender<PhysicsCommand>>) {
    scene.content.kinematic.insert(id.to_owned());
    let has_body = scene.content.nodes.get(id).is_some_and(|node| node.get_property::<Physics>().is_some());
    if let (true, Some(tx)) = (has_body, physics_command_tx) {
        let _ = tx.send(PhysicsCommand::SetKinematic { name: id.to_owned(), kinematic: true });
    }
}

/// Lets node `id` go again after `hold_node` - its body starting from where
/// it is now at `velocity`. True when it has a body (it stays off the
/// physics' pose RELEASE_GRACE_SECONDS longer); without one it's out
/// already.
fn release_node(scene: &mut Scene, id: &str, velocity: Vector3<f32>, physics_command_tx: Option<&Sender<PhysicsCommand>>) -> bool {
    let has_body = scene.content.nodes.get(id).is_some_and(|node| node.get_property::<Physics>().is_some());
    let velocity = if velocity.magnitude() > MAX_RELEASE_SPEED { velocity.normalize() * MAX_RELEASE_SPEED } else { velocity };
    match (has_body, physics_command_tx, scene.world_transform(id)) {
        (true, Some(tx), Some(world)) => {
            let _ = tx.send(PhysicsCommand::SetKinematic { name: id.to_owned(), kinematic: false });
            let _ = tx.send(PhysicsCommand::SetTransform { name: id.to_owned(), translation: world.position, rotation: world.rotation.into_inner(), linvel: velocity });
            true
        }
        _ => {
            scene.content.kinematic.remove(id);
            false
        }
    }
}

/// A path a node follows right now - see `PathFollower::update`.
pub struct FollowRequest {
    pub path: String,
    pub mode: FollowMode,
    pub pace: Pace,
    pub skill: Option<Difficulty>,
    pub orient: bool,
    /// Seconds since it started.
    pub elapsed: f32,
    /// Which follow it is - to tell a new one from the one already flying.
    pub key: (String, f32),
    /// Its animation's held (scrubbed), not playing: a flown path is shown
    /// where along it the plane would be by then, placed like a positioned
    /// one - and, played from there, flown on from that point.
    pub held: bool,
}

#[derive(Default)]
pub struct PathFollower {
    /// Nodes it's placing along a path now, and how fast each is going -
    /// for when it's let go.
    animated: HashMap<String, Vector3<f32>>,
    /// Nodes just let go, and how much longer they stay off the physics'
    /// pose.
    releasing: HashMap<String, f32>,
    /// Aircraft it has flying a path, and which follow.
    flown: HashMap<String, (String, f32)>,
    /// Seconds since the level started - map nodes' own paths run on it.
    level_time: f32,
}

impl PathFollower {
    /// One frame: `requests` - what each node follows now (the running
    /// animations') - and, for map nodes nothing else has on a path, their
    /// own (`NodeSpec::follow`); places, flies or lets go of each.
    pub fn update(&mut self, delta_time: f32, scene: &mut Scene, map: &MapSpec, requests: Vec<(String, FollowRequest)>, physics_command_tx: Option<&Sender<PhysicsCommand>>) {
        self.level_time += delta_time;
        self.releasing.retain(|id, remaining| {
            *remaining -= delta_time;
            if *remaining > 0.0 {
                return true;
            }
            scene.content.kinematic.remove(id);
            false
        });

        let mut active: HashMap<String, FollowRequest> = requests.into_iter().collect();
        for node in &map.nodes {
            let Some(follow) = &node.follow else { continue };
            active.entry(node.id.clone()).or_insert_with(|| FollowRequest {
                path: follow.path.clone(),
                mode: FollowMode::Positioned,
                pace: follow.pace,
                skill: None,
                orient: follow.orient,
                elapsed: self.level_time,
                key: (String::new(), 0.0),
                held: false,
            });
        }

        let mut still_animated = HashMap::new();
        let mut still_flown = HashMap::new();
        for (id, follow) in &active {
            let Some(path) = map.named_path(&follow.path) else { continue };
            let difficulty = follow.skill
                .or_else(|| map.find(id).map(|(_, placement)| placement.difficulty))
                .unwrap_or_default();
            let skill = difficulty.skill();
            let is_plane = scene.content.nodes.get(id).is_some_and(|node| node.get_behavior::<Plane>().is_some());

            // A flown path is flown as straight legs between its points.
            let flown = follow.mode == FollowMode::Flown && is_plane;
            let straight = PathSpec { smooth: false, ..path.clone() };
            let followed = if flown { &straight } else { path };

            if flown && !follow.held {
                if self.flown.get(id) != Some(&follow.key) {
                    let speed = match follow.pace {
                        Pace::BySkill if skill.max_performance => Speed::Max,
                        pace => Speed::Hold(pace_speed(pace, 0.0, &skill)),
                    };
                    let flying_speed = match speed {
                        Speed::Hold(speed) => speed,
                        Speed::Max => skill.cruise_kt * KT_TO_MS,
                    };
                    // The animation has the say over where it is: it starts
                    // where along the path it'd be by now (its first point at
                    // the bar's start), nose along it, already flying at the
                    // follow's speed - then its autopilot flies the rest.
                    let distance = flying_speed * follow.elapsed;
                    if let Some(sampled) = SampledPath::new(&straight) {
                        let (start, heading) = sampled.at(distance);
                        let heading = heading
                            .or_else(|| scene.world_transform(id).map(|world| world.rotation * Vector3::z()))
                            .unwrap_or_else(Vector3::z);
                        if let Some(mut world) = scene.world_transform(id) {
                            world.position = start;
                            world.rotation = facing(heading).unwrap_or(world.rotation);
                            scene.set_world_transform(id, world);
                            if let Some(tx) = physics_command_tx {
                                let _ = tx.send(PhysicsCommand::SetTransform { name: id.clone(), translation: start, rotation: world.rotation.into_inner(), linvel: heading * flying_speed });
                            }
                        }
                    }
                    let ahead = points_ahead(&path.points, path.looped, distance);
                    if let Some(plane) = scene.content.nodes.get_mut(id).and_then(|node| node.get_behavior_mut::<Plane>()) {
                        plane.set_path(&ahead, speed, difficulty, path.looped);
                        plane.scripted = true;
                    }
                }
                still_flown.insert(id.clone(), follow.key.clone());
                continue;
            }

            // Positioned - or a flown path held at a moment: where along it
            // the plane would be by then, at its flying speed.
            let Some(sampled) = SampledPath::new(followed) else { continue };
            let speed = pace_speed(follow.pace, sampled.length(), &skill);
            let distance = speed * follow.elapsed;
            let (position, heading) = sampled.at(distance);
            if !self.animated.contains_key(id) {
                self.releasing.remove(id);
                hold_node(scene, id, physics_command_tx);
            }
            let Some(mut world) = scene.world_transform(id) else { continue };
            world.position = position;
            if follow.orient {
                if let Some(rotation) = heading.and_then(facing) {
                    world.rotation = rotation;
                }
            }
            scene.set_world_transform(id, world);
            // Let go at the path's speed, the way it last went - flying on
            // past the end of a path that doesn't loop, not dropped.
            let velocity = heading.unwrap_or_else(Vector3::zeros) * speed;
            still_animated.insert(id.clone(), velocity);
        }

        // Let go of what isn't followed any more.
        for (id, velocity) in std::mem::take(&mut self.animated) {
            if !still_animated.contains_key(&id) && release_node(scene, &id, velocity, physics_command_tx) {
                self.releasing.insert(id, RELEASE_GRACE_SECONDS);
            }
        }
        self.animated = still_animated;
        for id in std::mem::take(&mut self.flown).into_keys() {
            if still_flown.contains_key(&id) {
                continue;
            }
            // Back to its own flight plan.
            let Some((_, placement)) = map.find(&id) else { continue };
            let (points, looped) = map.flight_plan(placement);
            if let Some(plane) = scene.content.nodes.get_mut(&id).and_then(|node| node.get_behavior_mut::<Plane>()) {
                plane.set_path(&points, Speed::Hold(cruise_speed(placement)), placement.difficulty, looped);
                plane.scripted = false;
            }
        }
        self.flown = still_flown;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(points: &[(f32, f32, f32)], looped: bool, smooth: bool) -> PathSpec {
        PathSpec { name: "test".to_owned(), points: points.to_vec(), looped, smooth }
    }

    #[test]
    fn it_goes_along_at_its_distance_and_holds_the_end() {
        let sampled = SampledPath::new(&path(&[(0.0, 0.0, 0.0), (100.0, 0.0, 0.0), (100.0, 0.0, 100.0)], false, false)).unwrap();
        assert!((sampled.length() - 200.0).abs() < 1e-3);
        let (halfway, heading) = sampled.at(50.0);
        assert!((halfway - Vector3::new(50.0, 0.0, 0.0)).magnitude() < 1e-3);
        assert!((heading.unwrap() - Vector3::x()).magnitude() < 1e-3);
        let (corner_on, _) = sampled.at(150.0);
        assert!((corner_on - Vector3::new(100.0, 0.0, 50.0)).magnitude() < 1e-3);
        let (end, _) = sampled.at(999.0);
        assert!((end - Vector3::new(100.0, 0.0, 100.0)).magnitude() < 1e-3);
        assert!(sampled.finished(999.0));
    }

    #[test]
    fn a_looped_path_comes_round_again() {
        let sampled = SampledPath::new(&path(&[(0.0, 0.0, 0.0), (100.0, 0.0, 0.0)], true, false)).unwrap();
        // There and back: 200 m round.
        assert!((sampled.length() - 200.0).abs() < 1e-3);
        let (again, _) = sampled.at(250.0);
        assert!((again - Vector3::new(50.0, 0.0, 0.0)).magnitude() < 1e-3);
        assert!(!sampled.finished(1e6));
    }

    #[test]
    fn a_smooth_path_still_passes_through_its_points() {
        let points = [(0.0, 0.0, 0.0), (100.0, 0.0, 0.0), (100.0, 0.0, 100.0), (0.0, 0.0, 100.0)];
        let sampled = SampledPath::new(&path(&points, false, true)).unwrap();
        for corner in points {
            let corner = Vector3::new(corner.0, corner.1, corner.2);
            let closest = (0..=400).map(|step| (sampled.at(sampled.length() * step as f32 / 400.0).0 - corner).magnitude()).fold(f32::MAX, f32::min);
            assert!(closest < 2.0, "{corner:?}: {closest}");
        }
    }

    #[test]
    fn flown_from_the_middle_goes_on_to_the_points_ahead() {
        let points = [(0.0, 0.0, 0.0), (100.0, 0.0, 0.0), (100.0, 0.0, 100.0), (0.0, 0.0, 100.0)];
        let (distances, total) = corner_distances(&points, true);
        assert_eq!(distances, vec![0.0, 100.0, 200.0, 300.0]);
        assert!((total - 400.0).abs() < 1e-3);
        // Halfway down the second leg: the third point's next.
        assert_eq!(points_ahead(&points, false, 150.0), points[2..].to_vec());
        // Looped, past the last: round to the first again, the rest after.
        assert_eq!(points_ahead(&points, true, 350.0), vec![points[0], points[1], points[2], points[3]]);
        assert_eq!(points_ahead(&points, true, 150.0), vec![points[2], points[3], points[0], points[1]]);
        // Once through and past the end: nothing left.
        assert!(points_ahead(&points, false, 1000.0).is_empty());
    }

    #[test]
    fn duration_spreads_the_path_over_its_seconds() {
        let skill = PilotSkill::default_for(Difficulty::Medium);
        assert!((pace_speed(Pace::Duration(10.0), 500.0, &skill) - 50.0).abs() < 1e-4);
        assert!((pace_speed(Pace::Speed(100.0), 0.0, &skill) - 51.4444).abs() < 1e-3);
        assert!(pace_speed(Pace::BySkill, 0.0, &PilotSkill::default_for(Difficulty::Ace)) > pace_speed(Pace::BySkill, 0.0, &PilotSkill::default_for(Difficulty::Beginner)));
    }
}
