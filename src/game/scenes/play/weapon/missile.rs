//! A missile in flight - a point mass flown from its `WeaponSpec`: the
//! motor's thrust along its velocity (and the propellant burnt off), drag by
//! Mach on its body tube (and the induced drag of whatever turn it's
//! pulling), gravity, and - homing on a target its seeker can
//! see - proportional navigation: it turns at N times the rate the line of
//! sight to the target turns, as hard as the G it can pull at its speed
//! allows. An infrared one sees heat (see play::weapon::heat): something
//! clearly brighter in its seeker's view than what it's on pulls it off -
//! another jet, or the sun, which it then just flies at. Its fuze goes off
//! when `proximity_radius` reaches one of an aircraft's colliders, or when
//! its path enters one (see play::weapon::fuze) - never the aircraft that
//! launched it. It self-destructs after `max_flight_time`, and ends in the sea.
//!
//! Plain numbers in, plain numbers out - nothing of the scene's - so it can
//! be tested on its own (see the tests below); play::weapon::missiles flies
//! the scene's missiles with it.

use std::collections::HashSet;

use nalgebra::{UnitQuaternion, Vector3};

use super::fuze::{self, FuzeCollider};
use super::heat;
use super::weapon_spec::{Seeker, WeaponSpec, SPEED_OF_SOUND};
use crate::game::scenes::play::world_collision::WorldCollision;

/// The same air the aircraft fly in (see plane::physics::wings::wing).
const AIR_DENSITY: f32 = 1.225;
const GRAVITY: Vector3<f32> = Vector3::new(0.0, -9.81, 0.0);
/// The longest step it's flown in (s) - a long frame is split into these.
const MAX_STEP: f32 = 1.0 / 240.0;
/// How strongly its fins and body push it toward where it points, per
/// radian of angle of attack (normal force coefficient slope, on its
/// cross-section) - what bends its path toward its nose.
const NORMAL_FORCE_SLOPE: f32 = 30.0;
/// How quickly its nose swings into the airflow (s) - fins at the back
/// weathervane it toward where it's going.
const WEATHERVANE_SECONDS: f32 = 0.12;
/// Flying at the sun, it turns its path onto it this quickly (s).
const PURSUIT_SECONDS: f32 = 1.0;
/// An IR seeker doesn't count what's this close (m) - the jet that just
/// fired it, a pass about to set the fuze off anyway.
const SEEKER_MIN_RANGE: f32 = 150.0;

/// Where a target is, how it's moving, and how hot it is (see
/// play::weapon::heat) - what the missile homes on.
#[derive(Clone, Debug)]
pub struct TargetTrack {
    pub name: String,
    pub position: Vector3<f32>,
    pub velocity: Vector3<f32>,
    pub heat: f32,
    /// A flare, not an aircraft - what a seeker's `flare_resistance` is
    /// about.
    pub decoy: bool,
    /// How the aircraft is turned. Held through a fuze step (it translates
    /// with `velocity`).
    pub rotation: UnitQuaternion<f32>,
    /// Fuselage and wing colliders, in the aircraft's frame. Empty on a
    /// flare - nothing for the fuze to hit.
    pub colliders: Vec<FuzeCollider>,
    /// How far those colliders reach from the aircraft's origin (m). The
    /// fuze skips the precise test past this plus its proximity radius.
    pub reach: f32,
}

/// What a missile's seeker is on.
#[derive(Clone, Debug, PartialEq)]
pub enum Track {
    /// An aircraft, by name.
    Aircraft(String),
    /// The sun - an IR seeker pulled off its target by it.
    Sun,
}

/// What's around a missile that it can run into.
pub struct Surroundings<'a> {
    /// Every aircraft still flying - its target among them. Any of them
    /// whose colliders the fuze reaches sets it off, except `shooter`.
    pub aircraft: &'a [TargetTrack],
    /// The aircraft that launched this missile, by name. Its colliders
    /// never set the fuze off.
    pub shooter: &'a str,
    /// The land and runway - `None`: none.
    pub ground: Option<&'a WorldCollision>,
    /// Where the water is.
    pub sea_level: f32,
    /// Which way the sun is (a unit direction) - `None`: no sun.
    pub sun: Option<Vector3<f32>>,
}

impl<'a> Surroundings<'a> {
    /// Just these aircraft over the sea at `sea_level`, no sun.
    pub fn open(aircraft: &'a [TargetTrack], sea_level: f32) -> Self {
        Self { aircraft, ground: None, sea_level, sun: None, shooter: "" }
    }
}

/// What steers it home: an intercept on a moving target (proportional
/// navigation - the line of sight to it and its velocity), or straight at
/// something infinitely far (the sun - a direction).
enum Homing {
    Intercept(Vector3<f32>, Vector3<f32>),
    Pursue(Vector3<f32>),
}

/// How a step of flight ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Flying,
    /// The proximity fuze went off passing the aircraft `target` - at `at`.
    Detonated { at: Vector3<f32>, target: String },
    /// Flew into the ground at `at`.
    HitGround { at: Vector3<f32> },
    /// Out of flight time - blown up at `at`.
    SelfDestructed { at: Vector3<f32> },
    /// Into the sea at `at`.
    HitWater { at: Vector3<f32> },
}

pub struct MissileFlight {
    pub position: Vector3<f32>,
    pub velocity: Vector3<f32>,
    /// Seconds since launch.
    pub time: f32,
    /// What its seeker is on - lost when it leaves the seeker's view
    /// (`gimbal_limit_deg`).
    pub target: Option<Track>,
    /// How it's turned - its nose is +Z. Starts as its aircraft was, then
    /// swings into the airflow without snapping (and keeps its roll).
    pub attitude: UnitQuaternion<f32>,
    /// Flares its seeker has seen through (see `flare_resistance`) - by
    /// name, never looked at again.
    ignored: HashSet<String>,
}

impl MissileFlight {
    /// Off the rail at `position`, moving with the aircraft (`velocity`),
    /// turned like it (`attitude`), homing on `target` if it has one.
    pub fn launch(position: Vector3<f32>, velocity: Vector3<f32>, attitude: UnitQuaternion<f32>, target: Option<String>) -> Self {
        Self { position, velocity, time: 0.0, target: target.map(Track::Aircraft), attitude, ignored: HashSet::new() }
    }

    /// The aircraft it's homing on, by name - none on the sun or nothing.
    pub fn target_aircraft(&self) -> Option<&str> {
        match &self.target {
            Some(Track::Aircraft(name)) => Some(name),
            _ => None,
        }
    }

    /// An IR seeker's look round what it's on: the brightest other source
    /// in its field of view (see play::weapon::heat) - another aircraft, a
    /// flare, or the sun - takes it over if it's clearly brighter. A flare
    /// gets one roll against `flare_resistance` (0..1): resisted, it's
    /// ignored from then on.
    fn look_for_brighter(&mut self, surroundings: &Surroundings, flare_resistance: f32) {
        let (look, current) = match &self.target {
            Some(Track::Aircraft(name)) => {
                let Some(track) = surroundings.aircraft.iter().find(|track| track.name == *name) else { return };
                let line_of_sight = track.position - self.position;
                (line_of_sight, heat::apparent(track.heat, line_of_sight.magnitude()))
            }
            Some(Track::Sun) => match surroundings.sun {
                Some(sun) => (sun, heat::SUN_APPARENT),
                None => return,
            },
            None => return,
        };
        let in_view = |direction: &Vector3<f32>| look.angle(direction).to_degrees() <= heat::SEEKER_FIELD_DEG;

        // (what, how bright, a flare)
        let mut brightest: Option<(Track, f32, bool)> = None;
        for track in surroundings.aircraft {
            let line_of_sight = track.position - self.position;
            let distance = line_of_sight.magnitude();
            if self.target_aircraft() == Some(&track.name) || self.ignored.contains(&track.name) || distance < SEEKER_MIN_RANGE || !in_view(&line_of_sight) {
                continue;
            }
            let brightness = heat::apparent(track.heat, distance);
            if brightest.as_ref().is_none_or(|(_, best, _)| brightness > *best) {
                brightest = Some((Track::Aircraft(track.name.clone()), brightness, track.decoy));
            }
        }
        let sun_in_view = surroundings.sun.is_some_and(|sun| self.target != Some(Track::Sun) && in_view(&sun));
        if sun_in_view && brightest.as_ref().is_none_or(|(_, best, _)| heat::SUN_APPARENT > *best) {
            brightest = Some((Track::Sun, heat::SUN_APPARENT, false));
        }
        let Some((track, brightness, decoy)) = brightest else { return };
        if brightness <= current * heat::SWITCH_RATIO {
            return;
        }
        if decoy && rand::random::<f32>() < flare_resistance {
            // Seen through.
            if let Track::Aircraft(name) = track {
                self.ignored.insert(name);
            }
            return;
        }
        self.target = Some(track);
    }

    /// Where its nose points.
    pub fn heading(&self) -> Vector3<f32> {
        self.attitude * Vector3::z()
    }

    /// Whether the motor's still burning.
    pub fn burning(&self, spec: &WeaponSpec) -> bool {
        self.time < spec.burn_time()
    }

    /// Flies `delta_time` seconds through `surroundings` - its target (if
    /// it has one and it's still there) found among their aircraft, each
    /// assumed to carry on straight through the step.
    pub fn step(&mut self, spec: &WeaponSpec, delta_time: f32, surroundings: &Surroundings) -> Outcome {
        // Gone (destroyed, removed; no sun) - nothing to home on.
        let there = match &self.target {
            Some(Track::Aircraft(name)) => surroundings.aircraft.iter().any(|track| track.name == *name),
            Some(Track::Sun) => surroundings.sun.is_some(),
            None => true,
        };
        if !there {
            self.target = None;
        }
        // Heat: something brighter in view takes an IR seeker over.
        if spec.guidance.seeker == Seeker::InfraRed {
            self.look_for_brighter(surroundings, spec.guidance.flare_resistance);
        }
        let target = self.target_aircraft().and_then(|name| surroundings.aircraft.iter().find(|track| track.name == name));
        let steps = (delta_time / MAX_STEP).ceil().max(1.0) as usize;
        let h = delta_time / steps as f32;
        let mut elapsed = 0.0;
        for _ in 0..steps {
            let target_now = target.map(|track| track.position + track.velocity * elapsed);
            let start = self.position;
            self.advance(spec, h, target.zip(target_now), surroundings.sun);
            let step_start = elapsed;
            elapsed += h;

            // The fuze, once armed: the earliest detonation along this
            // step, against any aircraft that isn't the one that fired it.
            // A collider the sphere reaches, or a path that enters one.
            if spec.fuze.collider_proximity && self.time >= spec.fuze.arm_delay {
                let nearest = surroundings.aircraft.iter()
                    .filter(|track| track.name != surroundings.shooter && !track.colliders.is_empty())
                    .filter_map(|track| {
                        let track_start = track.position + track.velocity * step_start;
                        let track_end = track.position + track.velocity * elapsed;
                        // Every collider sits inside `reach` of the aircraft.
                        // Farther than that plus the fuze, this step cannot
                        // touch one, so the precise test is skipped. A close
                        // pass or a hit still runs it.
                        let relative_start = start - track_start;
                        let relative_end = self.position - track_end;
                        let (gap, _) = closest_approach(relative_start, relative_end, start, self.position);
                        if gap > track.reach + spec.fuze.proximity_radius {
                            return None;
                        }
                        let t = fuze::detonation_t(&track.rotation, track_start, track_end, start, self.position, &track.colliders, spec.fuze.proximity_radius)?;
                        let at = start + (self.position - start) * t;
                        Some((t, at, track))
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0));
                if let Some((_, at, track)) = nearest {
                    return Outcome::Detonated { at, target: track.name.clone() };
                }
            }
            if let Some(at) = surroundings.ground.and_then(|ground| ground.segment_hit(start, self.position)) {
                return Outcome::HitGround { at };
            }
            if self.position.y <= surroundings.sea_level {
                let at = Vector3::new(self.position.x, surroundings.sea_level, self.position.z);
                return Outcome::HitWater { at };
            }
            if self.time >= spec.fuze.max_flight_time {
                return Outcome::SelfDestructed { at: self.position };
            }
        }
        Outcome::Flying
    }

    /// One step of `h` seconds - `target`: its track and where it is now;
    /// `sun`: which way the sun is.
    fn advance(&mut self, spec: &WeaponSpec, h: f32, target: Option<(&TargetTrack, Vector3<f32>)>, sun: Option<Vector3<f32>>) {
        let heading = self.heading();
        let speed = self.velocity.magnitude();
        let along = if speed > 1.0 { self.velocity / speed } else { heading };

        let mass = spec.mass_at(self.time);
        let dynamic_pressure = 0.5 * AIR_DENSITY * speed * speed;
        let drag = dynamic_pressure * spec.drag_coefficient(speed / SPEED_OF_SOUND) * spec.reference_area();
        // The motor pushes along its nose; drag along its path.
        let mut acceleration = heading * spec.thrust_at(self.time) / mass - along * drag / mass + GRAVITY;
        // Pointing off its path (angle of attack): the airflow on its fins
        // and body pushes the path round toward the nose.
        let off_path = heading - along * along.dot(&heading);
        acceleration += off_path * dynamic_pressure * spec.reference_area() * NORMAL_FORCE_SLOPE / mass;

        // Guided (after it's cleared the aircraft): homing on its target if
        // its seeker has one, and holding its path up against gravity.
        let guiding = spec.guidance.seeker != Seeker::None && self.time >= spec.guidance.guidance_delay;
        if guiding {
            // Where it's homing: its target aircraft, or the sun - lost once
            // it's out of the seeker's view.
            let line_of_sight = match &self.target {
                Some(Track::Aircraft(_)) => target.map(|(track, target_position)| Homing::Intercept(target_position - self.position, track.velocity)),
                Some(Track::Sun) => sun.map(Homing::Pursue),
                None => None,
            };
            let homing = line_of_sight.filter(|homing| {
                let direction = match homing { Homing::Intercept(line_of_sight, _) => *line_of_sight, Homing::Pursue(direction) => *direction };
                along.angle(&direction).to_degrees() <= spec.guidance.gimbal_limit_deg
            });
            if homing.is_none() {
                self.target = None;
            }
            let turn = self.steering(spec, along, speed, homing);
            acceleration += turn;
            // What turning costs: to pull `turn` it flies at an angle of
            // attack (its lift / (q S slope)), so that lift leans back by
            // as much - induced drag, lift x angle. Near free holding itself
            // up fast, dear in a hard turn, dearer still slow.
            let lift = turn.magnitude() * mass;
            let lift_per_radian = dynamic_pressure * spec.reference_area() * NORMAL_FORCE_SLOPE;
            if lift_per_radian > 1.0 {
                let induced_drag = lift * lift / lift_per_radian;
                acceleration -= along * induced_drag / mass;
            }
        }

        self.velocity += acceleration * h;
        self.position += self.velocity * h;
        self.time += h;

        // The nose swings toward where it's going - smoothly, by the
        // shortest way round, so it keeps its roll.
        if let Some(direction) = self.velocity.try_normalize(1.0) {
            let swing = 1.0 - (-h / WEATHERVANE_SECONDS).exp();
            if let Some(turn) = UnitQuaternion::rotation_between(&heading, &direction) {
                self.attitude = UnitQuaternion::identity().slerp(&turn, swing) * self.attitude;
            }
        }
    }

    /// The turn it's steered into: toward what it's homing on (`homing` -
    /// proportional navigation onto a target, or turning its path straight
    /// at the sun), plus holding up against gravity (the guidance's g-bias)
    /// - square to its flight path, as hard as the G it can pull at `speed`
    /// allows. Homing on nothing, its seeker gives no command at all: it
    /// coasts on a ballistic arc, dropping more the slower it gets.
    fn steering(&self, spec: &WeaponSpec, along: Vector3<f32>, speed: f32, homing: Option<Homing>) -> Vector3<f32> {
        if homing.is_none() {
            return Vector3::zeros();
        }
        let navigation = match homing {
            Some(Homing::Intercept(line_of_sight, target_velocity)) => {
                let range_squared = line_of_sight.norm_squared().max(1.0);
                // How fast (and which way) the line of sight is turning.
                let rotation = line_of_sight.cross(&(target_velocity - self.velocity)) / range_squared;
                rotation.cross(&self.velocity) * spec.guidance.navigation_constant
            }
            Some(Homing::Pursue(direction)) => (direction - along * along.dot(&direction)) * speed / PURSUIT_SECONDS,
            None => Vector3::zeros(),
        };
        // Gravity's pull across the flight path, cancelled.
        let against_gravity = -(GRAVITY - along * along.dot(&GRAVITY));
        let command = navigation + against_gravity;
        let across = command - along * along.dot(&command);
        let limit = spec.max_g_at(speed) * 9.81;
        if across.magnitude() > limit { across.normalize() * limit } else { across }
    }
}

/// The closest a point moving from `relative_start` to `relative_end`
/// (relative to the target) comes to it - and where the missile is then
/// (between `start` and `end`).
fn closest_approach(relative_start: Vector3<f32>, relative_end: Vector3<f32>, start: Vector3<f32>, end: Vector3<f32>) -> (f32, Vector3<f32>) {
    let motion = relative_end - relative_start;
    let t = if motion.norm_squared() > 1e-9 { (-relative_start.dot(&motion) / motion.norm_squared()).clamp(0.0, 1.0) } else { 0.0 };
    ((relative_start + motion * t).magnitude(), start + (end - start) * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Turned to point `direction`.
    fn facing(direction: Vector3<f32>) -> UnitQuaternion<f32> {
        UnitQuaternion::rotation_between(&Vector3::z(), &direction).unwrap_or_else(|| UnitQuaternion::from_axis_angle(&Vector3::y_axis(), std::f32::consts::PI))
    }

    fn aim9() -> WeaponSpec {
        WeaponSpec::read("missiles/aim-9x").expect("the AIM-9X reads")
    }

    /// Flies the missile at 60 steps a second for up to `seconds`, the
    /// target moving straight from `target_start` at `target_velocity`.
    fn fly(missile: &mut MissileFlight, spec: &WeaponSpec, target_start: Vector3<f32>, target_velocity: Vector3<f32>, seconds: f32) -> (Outcome, f32) {
        let dt = 1.0 / 60.0;
        let mut time = 0.0;
        while time < seconds {
            let track = fuselage("bandit", target_start + target_velocity * time, target_velocity);
            let outcome = missile.step(spec, dt, &Surroundings::open(std::slice::from_ref(&track), -1000.0));
            time += dt;
            if outcome != Outcome::Flying {
                return (outcome, time);
            }
        }
        (Outcome::Flying, time)
    }

    #[test]
    fn it_accelerates_past_mach_2_on_its_motor() {
        let spec = aim9();
        let mut missile = MissileFlight::launch(Vector3::new(0.0, 5000.0, 0.0), Vector3::new(0.0, 0.0, 250.0), UnitQuaternion::identity(), None);
        while missile.burning(&spec) {
            missile.step(&spec, 1.0 / 60.0, &Surroundings::open(&[], -1000.0));
        }
        let speed = missile.velocity.magnitude();
        assert!(speed > 2.0 * SPEED_OF_SOUND, "burnout speed {speed:.0} m/s");
        // And slows down after, drag winning.
        for _ in 0..600 {
            missile.step(&spec, 1.0 / 60.0, &Surroundings::open(&[], -1000.0));
        }
        assert!(missile.velocity.magnitude() < speed);
    }

    #[test]
    fn it_hits_a_target_ahead() {
        let mut spec = aim9();
        spec.fuze.collider_proximity = true;
        let mut missile = MissileFlight::launch(Vector3::new(0.0, 3000.0, 0.0), Vector3::new(0.0, 0.0, 250.0), UnitQuaternion::identity(), Some("bandit".into()));
        let (outcome, time) = fly(&mut missile, &spec, Vector3::new(200.0, 3100.0, 3000.0), Vector3::new(0.0, 0.0, 200.0), 20.0);
        assert!(matches!(outcome, Outcome::Detonated { .. }), "{outcome:?} after {time:.1} s");
    }

    #[test]
    fn it_hits_a_target_crossing_its_nose() {
        // 2 km ahead, crossing left to right at 250 m/s - it has to lead it.
        let mut spec = aim9();
        spec.fuze.collider_proximity = true;
        let mut missile = MissileFlight::launch(Vector3::new(0.0, 3000.0, 0.0), Vector3::new(0.0, 0.0, 250.0), UnitQuaternion::identity(), Some("bandit".into()));
        let (outcome, time) = fly(&mut missile, &spec, Vector3::new(600.0, 3000.0, 2000.0), Vector3::new(-250.0, 0.0, 0.0), 20.0);
        assert!(matches!(outcome, Outcome::Detonated { .. }), "{outcome:?} after {time:.1} s");
    }

    #[test]
    fn a_target_behind_it_is_lost() {
        let spec = aim9();
        let mut missile = MissileFlight::launch(Vector3::new(0.0, 3000.0, 0.0), Vector3::new(0.0, 0.0, 250.0), UnitQuaternion::identity(), Some("bandit".into()));
        let (outcome, _) = fly(&mut missile, &spec, Vector3::new(0.0, 3000.0, -2000.0), Vector3::new(0.0, 0.0, -200.0), 1.0);
        assert_eq!(outcome, Outcome::Flying);
        assert_eq!(missile.target, None, "past its gimbal limit");
    }

    #[test]
    fn it_ends_in_the_sea_or_at_its_flight_time() {
        let spec = aim9();
        // Fired straight down: the sea.
        let mut missile = MissileFlight::launch(Vector3::new(0.0, 500.0, 0.0), Vector3::new(0.0, -100.0, 0.0), facing(-Vector3::y()), None);
        let mut outcome = Outcome::Flying;
        for _ in 0..600 {
            outcome = missile.step(&spec, 1.0 / 60.0, &Surroundings::open(&[], 0.0));
            if outcome != Outcome::Flying { break; }
        }
        assert!(matches!(outcome, Outcome::HitWater { .. }), "{outcome:?}");
        // Fired straight up: it runs out of time first.
        let mut missile = MissileFlight::launch(Vector3::new(0.0, 500.0, 0.0), Vector3::new(0.0, 100.0, 0.0), facing(Vector3::y()), None);
        let mut outcome = Outcome::Flying;
        for _ in 0..(70 * 60) {
            outcome = missile.step(&spec, 1.0 / 60.0, &Surroundings::open(&[], -100_000.0));
            if outcome != Outcome::Flying { break; }
        }
        assert!(matches!(outcome, Outcome::SelfDestructed { .. }), "{outcome:?}");
    }

    #[test]
    fn it_leaves_the_rail_without_snapping() {
        // Fired banked 90 degrees, its path 6 degrees under its nose (the
        // jet's angle of attack).
        let spec = aim9();
        let attitude = UnitQuaternion::from_axis_angle(&Vector3::z_axis(), std::f32::consts::FRAC_PI_2);
        let velocity = UnitQuaternion::from_axis_angle(&Vector3::x_axis(), 6f32.to_radians()) * Vector3::new(0.0, 0.0, 250.0);
        let mut missile = MissileFlight::launch(Vector3::new(0.0, 3000.0, 0.0), velocity, attitude, None);
        missile.step(&spec, 1.0 / 60.0, &Surroundings::open(&[], -1000.0));
        assert!(missile.heading().angle(&Vector3::z()).to_degrees() < 1.5, "one frame in, the nose barely moves");
        for _ in 0..60 {
            missile.step(&spec, 1.0 / 60.0, &Surroundings::open(&[], -1000.0));
        }
        assert!(missile.heading().angle(&missile.velocity).to_degrees() < 0.5, "a second in, it points where it goes");
        let up = missile.attitude * Vector3::y();
        assert!(up.y.abs() < 0.2, "still banked: {up:?}");
    }

    #[test]
    fn unlocked_it_drops_on_a_ballistic_arc() {
        // Nothing to home on: no command, so gravity bends it down - about
        // as far as anything falls in 3 s (~44 m), the airflow on its fins
        // swinging its nose down after its path.
        let spec = aim9();
        let mut missile = MissileFlight::launch(Vector3::new(0.0, 3000.0, 0.0), Vector3::new(0.0, 0.0, 250.0), UnitQuaternion::identity(), None);
        for _ in 0..(3 * 60) {
            missile.step(&spec, 1.0 / 60.0, &Surroundings::open(&[], -1000.0));
        }
        let dropped = 3000.0 - missile.position.y;
        assert!((25.0..60.0).contains(&dropped), "dropped {dropped:.1} m");
        assert!(missile.velocity.y < -5.0, "heading down: {:?}", missile.velocity);
    }

    #[test]
    fn homing_it_holds_itself_up() {
        // A target dead level ahead: the g-bias keeps it from sagging.
        let spec = aim9();
        let mut missile = MissileFlight::launch(Vector3::new(0.0, 3000.0, 0.0), Vector3::new(0.0, 0.0, 250.0), UnitQuaternion::identity(), Some("bandit".into()));
        fly(&mut missile, &spec, Vector3::new(0.0, 3000.0, 6000.0), Vector3::new(0.0, 0.0, 200.0), 2.0);
        assert!((missile.position.y - 3000.0).abs() < 15.0, "{}", missile.position.y);
    }

    #[test]
    fn it_explodes_on_the_ground() {
        let spec = aim9();
        let mut ground = WorldCollision::default();
        // A flat square of land 2 km across, 100 m up.
        let vertices = [Vector3::new(-1.0, 0.1, -1.0), Vector3::new(1.0, 0.1, -1.0), Vector3::new(1.0, 0.1, 1.0), Vector3::new(-1.0, 0.1, 1.0)];
        ground.add_mesh(&vertices, &[[0, 1, 2], [0, 2, 3]], Vector3::zeros(), Vector3::new(1000.0, 1000.0, 1000.0));
        let surroundings = Surroundings { aircraft: &[], ground: Some(&ground), sea_level: 0.0, sun: None, shooter: "" };
        let down = UnitQuaternion::rotation_between(&Vector3::z(), &Vector3::new(0.0, -1.0, 1.0)).unwrap();
        let mut missile = MissileFlight::launch(Vector3::new(0.0, 400.0, 0.0), Vector3::new(0.0, -150.0, 150.0), down, None);
        let mut outcome = Outcome::Flying;
        for _ in 0..600 {
            outcome = missile.step(&spec, 1.0 / 60.0, &surroundings);
            if outcome != Outcome::Flying { break; }
        }
        match outcome {
            Outcome::HitGround { at } => assert!((at.y - 100.0).abs() < 0.01, "{at:?}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn any_aircraft_close_by_sets_it_off() {
        // Not its target - another jet in its way.
        let mut spec = aim9();
        spec.fuze.collider_proximity = true;
        let launch = || MissileFlight::launch(Vector3::new(0.0, 3000.0, 0.0), Vector3::new(0.0, 0.0, 250.0), UnitQuaternion::identity(), None);
        // Parked right where it'll be 3 s after launch.
        let mut path = launch();
        for _ in 0..(3 * 60) {
            path.step(&spec, 1.0 / 60.0, &Surroundings::open(&[], -1000.0));
        }
        let bystander = fuselage("wingman", path.position + Vector3::new(3.0, 0.0, 0.0), Vector3::zeros());
        let mut missile = launch();
        let mut outcome = Outcome::Flying;
        for _ in 0..600 {
            outcome = missile.step(&spec, 1.0 / 60.0, &Surroundings::open(std::slice::from_ref(&bystander), -1000.0));
            if outcome != Outcome::Flying { break; }
        }
        assert!(matches!(outcome, Outcome::Detonated { ref target, .. } if target == "wingman"), "{outcome:?}");
    }

    /// A missile flying along +Z on `target`, one frame, among `aircraft`
    /// with the sun `sun` - what it's on after.
    fn seen_after_a_frame(target: &str, aircraft: &[TargetTrack], sun: Option<Vector3<f32>>) -> Option<Track> {
        seen_by(&aim9(), target, aircraft, sun)
    }

    fn seen_by(spec: &WeaponSpec, target: &str, aircraft: &[TargetTrack], sun: Option<Vector3<f32>>) -> Option<Track> {
        let mut missile = MissileFlight::launch(Vector3::new(0.0, 3000.0, 0.0), Vector3::new(0.0, 0.0, 600.0), UnitQuaternion::identity(), Some(target.into()));
        let surroundings = Surroundings { sun, ..Surroundings::open(aircraft, -1000.0) };
        missile.step(spec, 1.0 / 60.0, &surroundings);
        missile.target
    }

    fn jet(name: &str, position: Vector3<f32>, heat: f32) -> TargetTrack {
        TargetTrack { name: name.into(), position, velocity: Vector3::zeros(), heat, decoy: false, rotation: UnitQuaternion::identity(), colliders: Vec::new(), reach: 0.0 }
    }

    /// An aircraft with a fuselage box about its origin - what the fuze
    /// used to treat as the whole jet, now a collider.
    fn fuselage(name: &str, position: Vector3<f32>, velocity: Vector3<f32>) -> TargetTrack {
        let colliders = vec![FuzeCollider::Cuboid { half_extents: Vector3::new(0.7, 0.7, 4.7), position: Vector3::new(0.0, 0.0, 1.5) }];
        let reach = fuze::reach_of(&colliders);
        TargetTrack {
            name: name.into(),
            position,
            velocity,
            heat: heat::MILITARY_HEAT,
            decoy: false,
            rotation: UnitQuaternion::identity(),
            colliders,
            reach,
        }
    }

    /// The AIM-9 with its fuze already armed.
    fn armed() -> WeaponSpec {
        let mut spec = aim9();
        spec.fuze.arm_delay = 0.0;
        spec.fuze.collider_proximity = true;
        spec
    }

    #[test]
    fn turning_hard_bleeds_its_speed() {
        // The same shot, one at a target dead ahead, one at a target 60
        // degrees off its nose (hard turn) - 2 s in, the turning one's slower.
        let spec = aim9();
        let speed_after = |target: Vector3<f32>| {
            let mut missile = MissileFlight::launch(Vector3::new(0.0, 3000.0, 0.0), Vector3::new(0.0, 0.0, 250.0), UnitQuaternion::identity(), Some("bandit".into()));
            fly(&mut missile, &spec, target, Vector3::zeros(), 2.0);
            missile.velocity.magnitude()
        };
        let straight = speed_after(Vector3::new(0.0, 3000.0, 8000.0));
        let turning = speed_after(Vector3::new(8000.0 * 60f32.to_radians().sin(), 3000.0, 8000.0 * 60f32.to_radians().cos()));
        // Still burning and fast, where turning's cheapest - without induced
        // drag the two would be the same (a pull square to its path doesn't
        // slow it).
        assert!(turning < straight - 10.0, "turning {turning:.0} m/s, straight {straight:.0} m/s");
    }

    #[test]
    fn its_flare_resistance_decides_whether_a_flare_pulls_it_off() {
        // A hot flare right beside the jet it's chasing.
        let scene = [
            jet("bandit", Vector3::new(0.0, 3000.0, 3000.0), heat::MILITARY_HEAT),
            TargetTrack { decoy: true, ..jet("flare:1", Vector3::new(20.0, 3000.0, 3000.0), 40.0) },
        ];
        let with_resistance = |resistance| {
            let mut spec = aim9();
            spec.guidance.flare_resistance = resistance;
            seen_by(&spec, "bandit", &scene, None)
        };
        assert_eq!(with_resistance(0.0), Some(Track::Aircraft("flare:1".into())), "falls for every one");
        assert_eq!(with_resistance(1.0), Some(Track::Aircraft("bandit".into())), "falls for none");
    }

    #[test]
    fn a_far_target_in_front_of_the_sun_loses_it_to_the_sun() {
        // Dead ahead and the sun right behind it: 4 km out at military
        // power it's dimmer than the sun; 1 km out it's brighter.
        let sun = Some(Vector3::z());
        let far = [jet("bandit", Vector3::new(0.0, 3000.0, 4000.0), heat::MILITARY_HEAT)];
        assert_eq!(seen_after_a_frame("bandit", &far, sun), Some(Track::Sun));
        let near = [jet("bandit", Vector3::new(0.0, 3000.0, 1000.0), heat::MILITARY_HEAT)];
        assert_eq!(seen_after_a_frame("bandit", &near, sun), Some(Track::Aircraft("bandit".into())));
        // The sun well off to the side: no matter how far.
        let aside = Some(Vector3::new(1.0, 1.0, 0.0).normalize());
        assert_eq!(seen_after_a_frame("bandit", &far, aside), Some(Track::Aircraft("bandit".into())));
    }

    #[test]
    fn a_hotter_jet_in_view_takes_it_over() {
        // Two jets side by side ahead, ~1 degree apart - the other on full
        // afterburner, this one at idle.
        let pair = [
            jet("bandit", Vector3::new(0.0, 3000.0, 3000.0), heat::IDLE_HEAT),
            jet("other", Vector3::new(50.0, 3000.0, 3000.0), heat::AFTERBURNER_HEAT),
        ];
        assert_eq!(seen_after_a_frame("bandit", &pair, None), Some(Track::Aircraft("other".into())));
        // About as hot: it stays on what it's on.
        let even = [
            jet("bandit", Vector3::new(0.0, 3000.0, 3000.0), heat::MILITARY_HEAT),
            jet("other", Vector3::new(50.0, 3000.0, 3000.0), heat::MILITARY_HEAT),
        ];
        assert_eq!(seen_after_a_frame("bandit", &even, None), Some(Track::Aircraft("bandit".into())));
    }

    #[test]
    fn on_the_sun_it_flies_at_it() {
        let spec = aim9();
        let sun = Vector3::new(0.0, 0.3, 1.0).normalize();
        let mut missile = MissileFlight::launch(Vector3::new(0.0, 3000.0, 0.0), Vector3::new(0.0, 0.0, 600.0), UnitQuaternion::identity(), None);
        missile.target = Some(Track::Sun);
        let surroundings = Surroundings { sun: Some(sun), ..Surroundings::open(&[], -1000.0) };
        for _ in 0..(3 * 60) {
            missile.step(&spec, 1.0 / 60.0, &surroundings);
        }
        assert!(missile.velocity.angle(&sun).to_degrees() < 3.0, "{:?}", missile.velocity);
    }

    #[test]
    fn a_wing_off_the_origin_sets_the_fuze_off() {
        // The aircraft's middle is 30 m from the path. Only a wing out there
        // is close, and that is enough.
        let spec = armed();
        let mut wing = jet("bandit", Vector3::zeros(), heat::MILITARY_HEAT);
        wing.colliders = vec![FuzeCollider::Cuboid { half_extents: Vector3::new(2.0, 0.3, 2.0), position: Vector3::new(30.0, 0.0, 0.0) }];
        wing.reach = fuze::reach_of(&wing.colliders);
        let mut missile = MissileFlight::launch(Vector3::new(30.0, 0.0, -5.0), Vector3::new(0.0, 0.0, 600.0), UnitQuaternion::identity(), None);
        let outcome = missile.step(&spec, 1.0 / 60.0, &Surroundings::open(std::slice::from_ref(&wing), -1000.0));
        assert!(matches!(outcome, Outcome::Detonated { ref target, .. } if target == "bandit"), "{outcome:?}");
    }

    #[test]
    fn passing_the_origin_without_a_collider_does_not() {
        // Close to where the aircraft sits, and nowhere near its only collider.
        let spec = armed();
        let mut away = jet("bandit", Vector3::zeros(), heat::MILITARY_HEAT);
        away.colliders = vec![FuzeCollider::Cuboid { half_extents: Vector3::new(1.0, 1.0, 1.0), position: Vector3::new(100.0, 0.0, 0.0) }];
        away.reach = fuze::reach_of(&away.colliders);
        let mut missile = MissileFlight::launch(Vector3::new(0.0, 0.0, -5.0), Vector3::new(0.0, 0.0, 300.0), UnitQuaternion::identity(), None);
        let outcome = missile.step(&spec, 1.0 / 60.0, &Surroundings::open(std::slice::from_ref(&away), -1000.0));
        assert_eq!(outcome, Outcome::Flying);
    }

    #[test]
    fn hitting_a_thin_wing_detonates_on_contact() {
        // One step clears the slab. The sphere is tiny, so only entering it
        // sets the fuze off, on the near face.
        let mut spec = armed();
        spec.fuze.proximity_radius = 0.01;
        let mut points = Vec::new();
        for x in [-2.0_f32, 2.0] {
            for y in [-1.0_f32, 1.0] {
                for z in [-0.1_f32, 0.1] {
                    points.push(Vector3::new(x, y, z));
                }
            }
        }
        let mut wing = jet("bandit", Vector3::zeros(), heat::MILITARY_HEAT);
        wing.colliders = vec![FuzeCollider::hull(points)];
        wing.reach = fuze::reach_of(&wing.colliders);
        let mut missile = MissileFlight::launch(Vector3::new(0.0, 0.0, -2.0), Vector3::new(0.0, 0.0, 800.0), UnitQuaternion::identity(), None);
        match missile.step(&spec, 1.0 / 240.0, &Surroundings::open(std::slice::from_ref(&wing), -1000.0)) {
            Outcome::Detonated { at, .. } => assert!((at.z + 0.1).abs() < 0.15, "{at:?}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_shooters_colliders_do_not_set_it_off() {
        let spec = armed();
        let shooter = fuselage("viper", Vector3::zeros(), Vector3::zeros());
        // Launched inside that fuselage.
        let mut missile = MissileFlight::launch(Vector3::zeros(), Vector3::new(0.0, 0.0, 50.0), UnitQuaternion::identity(), None);
        let surroundings = Surroundings { shooter: "viper", ..Surroundings::open(std::slice::from_ref(&shooter), -1000.0) };
        assert_eq!(missile.step(&spec, 1.0 / 60.0, &surroundings), Outcome::Flying);
        // The same pass at someone else does.
        let mut missile = MissileFlight::launch(Vector3::zeros(), Vector3::new(0.0, 0.0, 50.0), UnitQuaternion::identity(), None);
        let other = Surroundings { shooter: "other", ..Surroundings::open(std::slice::from_ref(&shooter), -1000.0) };
        assert!(matches!(missile.step(&spec, 1.0 / 60.0, &other), Outcome::Detonated { .. }));
    }

    #[test]
    fn the_fuze_catches_a_pass_between_frames() {
        // Missile and target crossing 5 m apart at a combined ~1300 m/s - a
        // whole frame would jump them past each other.
        let (closest, _) = closest_approach(Vector3::new(0.0, 5.0, -20.0), Vector3::new(0.0, 5.0, 20.0), Vector3::zeros(), Vector3::new(0.0, 0.0, 40.0));
        assert!((closest - 5.0).abs() < 1e-4);
    }
}
