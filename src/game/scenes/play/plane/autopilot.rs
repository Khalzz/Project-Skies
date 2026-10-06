//! A bot's pilot: flies the aircraft through its path's points (see
//! play::map's paths), strictly in order - round and round, or once and on
//! straight - by moving the same stick and throttle a player would
//! (`PlaneControls`), so the flight physics fly it, fly-by-wire and all.
//!
//! How it steers: it tracks the line between the point it came from and the
//! one it's flying to - aiming at a spot a few seconds ahead of it on that
//! line (the closer it is to the line, the more that spot's just ahead of it
//! on it; well off it, the spot pulls it back on), not straight at the point
//! - so it flies the path as drawn instead of wherever a turn left it.
//!
//! It flies like a fighter pilot: works out how hard it wants to turn
//! toward that spot (the further off it is, the harder - up to its skill's
//! G), adds what holding against gravity takes, rolls so the lift points
//! that way and pulls that G - a descent too, rolled all the way over
//! until the lift points down the path, not stopped at the bank it uses
//! for a level turn. It holds 1 G until the wings are there, then pulls,
//! rather than pushing. Lighter while it's still rolling, all of it
//! once it's lined up. A level leg rolls the wings back upright, and keeps
//! that roll going through inverted so it doesn't sit there. It looks a
//! few points ahead and starts each turn early — by its turn radius, how
//! long the wings take to roll in, and how sharp the corner is — and it
//! slows before a corner the speed it's at can't make, so it arrives on
//! the points instead of swinging wide of them. It works the throttle
//! around military power: up into the afterburner when the speed is short, and down when a turn matters more
//! than holding that speed - or full power, flying flat out.
//! With the pitch fly-by-wire on (the default - see `Fcs`) the stick
//! commands G, which is exactly what it works out; the roll fly-by-wire
//! turns its aileron into a roll rate, and it commands the rate that
//! meets the bank it wants already stopped.
//!
//! How well it does all that is its pilot's skill (see play::pilot_skill):
//! how hard it banks and pulls, how early it turns for the next point, how
//! quick its hands are, and how much it wanders off its heading, height and
//! speed.

use nalgebra::{UnitQuaternion, Vector3};

use crate::game::scenes::play::map::Difficulty;

use super::controls::PlaneControls;
use super::flight_data::FlightData;
use super::physics::rolling_rate::{commanded_roll_rate_deg_s, RollRateParams};

// Within its skill's `corner_lead_m` of a point it counts as reached - on to
// the next. Otherwise a point counts as reached once the aircraft crosses its
// finish line: the plane through the point square to the leg flown to it (see
// `Autopilot::crossed`) - so a point is only ever passed by flying that leg,
// never skipped for being behind the nose, and one it can't turn tightly
// enough to hit doesn't keep it circling.

/// How far ahead along the leg it aims - this many seconds at its speed,
/// no nearer or farther than these (m).
const LOOKAHEAD_SECONDS: f32 = 4.0;
const MIN_LOOKAHEAD: f32 = 250.0;
const MAX_LOOKAHEAD: f32 = 2500.0;
/// How fast it wants to turn toward where it's going, per radian it's off,
/// per unit of its skill's turn eagerness (rad/s) - an eager pilot reefs
/// in hard for a small error, a timid one eases round.
const TURN_RATE_PER_ERROR: f32 = 0.35;
/// Least G it pulls (the most is its skill's).
const MIN_G: f32 = -1.0;
/// The most G it puts into climbing or diving toward a point's height, on
/// top of holding itself up - height's chased gently, turns hard.
const MAX_CLIMB_PULL_G: f32 = 2.0;
/// How hard it plans its turns, as a share of its skill's G - leading a
/// corner by the radius it turns at with this much pull, it has some left
/// to tighten up with.
const PLANNED_G_SHARE: f32 = 0.8;
/// It leads a corner by no more than this many turn radii (a U-turn's lead
/// would be endless).
const MAX_LEAD_RADII: f32 = 1.5;
/// The pitch fly-by-wire's stick range - stick centered is +1 G, full back
/// +9, full forward -2 (see physics::pitch_flcs).
const STICK_CENTER_G: f32 = 1.0;
const STICK_MAX_G: f32 = 9.0;
const STICK_MIN_G: f32 = -2.0;
/// How long it means to take to roll onto the bank it wants, times its
/// hand speed (`control_rate`, per second). A slower pilot takes longer,
/// and the wings arrive as the hands do instead of going past. The stick
/// commands the roll rate that meets that bank already stopped (see
/// `Autopilot::aileron_for`); this is the most aileron it uses.
const BANK_CAPTURE_PER_HAND: f32 = 4.0;
const MAX_AILERON: f32 = 0.8;
/// Throttle per m/s short of its cruise speed. Military power (the
/// afterburner's gate - see `Autopilot::military`) is the lever when the
/// speed is right and it's flying level: this much slow, it goes up through
/// that gate into the afterburner, and this much fast, it comes back down.
const THROTTLE_PER_SPEED_ERROR: f32 = 0.02;
/// How far below military it sets the lever when a turn has the priority
/// over speed, and how many G past 1 that takes. A hard corner is allowed
/// to bleed speed and tighten, instead of the engine holding the speed
/// through it.
const TURN_THROTTLE_DROP: f32 = 0.45;
const TURN_PRIORITY_G: f32 = 3.0;
/// Below this airspeed (m/s) it isn't flying yet (a takeoff roll): full
/// throttle, wings level, stick centered.
const FLYING_SPEED: f32 = 60.0;
/// Below this altitude (m), and below the path it's flying, it won't dive -
/// climbing at least gently. A path drawn lower than this is flown at its
/// own height; the floor only stops a dive under a higher one.
const MIN_ALTITUDE: f32 = 150.0;
const MIN_ALTITUDE_CLIMB_DEG: f32 = 5.0;
/// How far a pilot with no precision at all wanders off what it wants:
/// its heading (deg), its climb angle (deg) and its speed (m/s) - less the
/// more precise it is, none at all for a perfect one.
const WANDER_HEADING_DEG: f32 = 10.0;
const WANDER_FLIGHT_PATH_DEG: f32 = 4.0;
const WANDER_SPEED: f32 = 20.0;
/// A path descending no steeper than this (deg) is flown upright. Steeper,
/// it rolls over until the lift points down the path.
const LEVEL_FLIGHT_DEG: f32 = 10.0;
/// While the nose is at least this far (deg) above a descending path, the
/// roll goes to inverted so a pull can bring it down. A heading error is
/// kept from holding the wings near 90° until that pitch change is underway.
const PITCH_FIRST_DEG: f32 = 30.0;
/// How many corners ahead it plans, and how long before one it will already
/// be slowing if the speed it's at can't make the turn.
const PLAN_STEPS: usize = 3;
const PLAN_HORIZON_SECONDS: f32 = 12.0;
/// Steepest it will aim (deg). The path's own slope, up to this — a shallower
/// cap leaves it short of a point it could have reached by starting sooner.
const MAX_AIM_DEG: f32 = 50.0;

/// How it holds its speed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Speed {
    /// This airspeed (m/s).
    Hold(f32),
    /// As fast as the plane goes - full throttle.
    Max,
}

pub struct Autopilot {
    path: Vec<Vector3<f32>>,
    /// Round and round - otherwise once, then on straight ahead.
    looped: bool,
    /// The point it's flying to - an index into `path`.
    next: usize,
    /// Flown once through its last point - flying on straight.
    finished: bool,
    /// Where the leg to `next` started - the point before it, or where the
    /// aircraft was when it set off for its first. `None` until it's flown.
    leg_start: Option<Vector3<f32>>,
    speed: Speed,
    /// Its pilot's skill level - its profile read each frame (see
    /// play::pilot_skill), so a change in the editor shows straight away.
    skill: Difficulty,
    /// Military power, 0..1 - where the afterburner lights (1: none). The
    /// lever it holds in level flight at the speed it wants; above it is
    /// the afterburner.
    military: f32,
    /// Seconds it's been flying - the clock its wandering runs on - and
    /// where in it this pilot starts, so two don't wander alike.
    clock: f32,
    phase: f32,
    /// Which way it's rolling back to wings level (+ increases bank, left
    /// wing down), once a level leg has caught it inverted. Kept until the
    /// wings are upright: the bank reading flips at 180°, and following
    /// that flip reverses the stick and leaves it sitting there.
    level_roll: Option<f32>,
}

impl Autopilot {
    pub fn new(path: &[(f32, f32, f32)], speed: Speed, skill: Difficulty, looped: bool) -> Self {
        // Any number that differs between paths will do for the phase.
        let phase = path.iter().map(|point| point.0 * 0.013 + point.2 * 0.007).sum::<f32>().rem_euclid(100.0);
        Self {
            path: path.iter().map(|&(x, y, z)| Vector3::new(x, y, z)).collect(),
            looped,
            next: 0,
            finished: false,
            leg_start: None,
            speed,
            skill,
            // No afterburner until the plane says where military power is.
            military: 1.0,
            clock: 0.0,
            phase,
            level_roll: None,
        }
    }

    /// Military power is `gate` (the engine's afterburner gate, 1 when it
    /// has none) - the lever it holds when the speed is right.
    pub fn with_military(mut self, gate: f32) -> Self {
        self.military = gate.clamp(0.01, 1.0);
        self
    }

    /// The point it's flying to - an index into its path. `None` without a
    /// path, or once it's flown a path that doesn't loop.
    pub fn next_point(&self) -> Option<usize> {
        (!self.path.is_empty() && !self.finished).then_some(self.next)
    }

    /// It's flown a path that doesn't loop all the way through.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Where the point it's flying to is - `None` without a path, or once
    /// it's flown a path that doesn't loop.
    pub fn waypoint(&self) -> Option<Vector3<f32>> {
        self.next_point().map(|next| self.path[next])
    }

    /// The points it flies, which it's flying to, and whether round and
    /// round - for the editor to draw.
    pub fn route(&self) -> (&[Vector3<f32>], Option<usize>, bool) {
        (&self.path, self.next_point(), self.looped)
    }

    /// A smooth, slow, never quite repeating wander in -1..1 - `channel`
    /// keeps its heading, climb and speed wandering apart.
    fn wander(&self, channel: f32) -> f32 {
        let t = self.clock + self.phase + channel * 37.0;
        0.6 * (t * 0.23 + channel).sin() + 0.4 * (t * 0.61 + 2.1 * channel).sin()
    }

    /// Moves `controls` toward what flies it to its next point, for this
    /// frame - `position`/`rotation` where the aircraft is now. Leaves them
    /// alone without a path.
    pub fn fly(&mut self, controls: &mut PlaneControls, position: Vector3<f32>, rotation: UnitQuaternion<f32>, flight_data: &FlightData, delta_time: f32) {
        if self.path.is_empty() {
            return;
        }
        self.clock += delta_time;
        let skill = self.skill.skill();
        let sloppiness = (1.0 - skill.precision).clamp(0.0, 1.0);
        let velocity = flight_data.velocity;
        let speed = velocity.magnitude();

        // On to the next point - only this one, in order - once it's
        // reached or its finish line is crossed. Past the last of a path
        // that doesn't loop: on straight, the way it's going.
        let leg_start = *self.leg_start.get_or_insert(position);
        let point = self.path[self.next];
        // How long the wings take to roll onto a bank. The turn has to
        // start that early, or it arrives at the corner still wings-level.
        let capture = BANK_CAPTURE_PER_HAND / skill.control_rate.max(0.1);
        // A corner a few points ahead that this speed can't make: the speed
        // that fits it. Looked up before the point is counted as reached,
        // so the slowdown is underway by the time the turn starts.
        let fit_speed = match self.speed {
            Speed::Hold(_) => self.speed_for_corners(position, speed, skill.max_g),
            Speed::Max => None,
        };
        // Started early by its turn radius at this corner's angle (see the
        // module's doc comment) - or by its skill's own corner lead.
        let lead = skill.corner_lead_m.max(50.0).max(self.turn_lead(leg_start, speed, skill.max_g));
        if !self.finished && ((point - position).magnitude() < lead || Self::crossed(leg_start, point, position)) {
            if self.next + 1 == self.path.len() && !self.looped {
                self.finished = true;
            } else {
                self.next = (self.next + 1) % self.path.len();
                // The next leg starts at the point just reached - or, for a
                // one-point path (the same point again), where it is now.
                self.leg_start = Some(if self.path[self.next] == point { position } else { point });
            }
        }
        let to_point = if self.finished {
            // Straight on, level.
            Vector3::new(velocity.x, 0.0, velocity.z).try_normalize(1e-3).unwrap_or_else(|| rotation * Vector3::z()) * 5000.0
        } else {
            self.aim_spot(position, speed, skill.max_g, capture)
        };

        // Where it wants to go - with its wander, never too steep, never
        // diving down low.
        let mut wanted = to_point.try_normalize(1e-3).unwrap_or_else(|| rotation * Vector3::z());
        let wander_heading = (WANDER_HEADING_DEG * sloppiness * self.wander(1.0)).to_radians();
        wanted = UnitQuaternion::from_axis_angle(&Vector3::y_axis(), wander_heading) * wanted;
        let max_climb = MAX_AIM_DEG.to_radians();
        let mut climb = (wanted.y.clamp(-1.0, 1.0).asin() + (WANDER_FLIGHT_PATH_DEG * sloppiness * self.wander(2.0)).to_radians()).clamp(-max_climb, max_climb);
        // A path at 30 m is flown at 30 m. The floor only catches a dive
        // that has fallen below both 150 m and the leg it's been given.
        let floor = MIN_ALTITUDE.min(self.path_altitude());
        if position.y < floor {
            climb = climb.max(MIN_ALTITUDE_CLIMB_DEG.to_radians());
        }
        let flat = Vector3::new(wanted.x, 0.0, wanted.z).try_normalize(1e-6).unwrap_or_else(|| Vector3::new(velocity.x, 0.0, velocity.z).try_normalize(1e-6).unwrap_or_else(Vector3::z));
        let wanted = (flat * climb.cos() + Vector3::y() * climb.sin()).normalize();

        // Its bank, + left wing down (+X is the left wing, +Y up) - against
        // wings level about where it's going.
        let forward = velocity.try_normalize(1e-3).unwrap_or_else(|| rotation * Vector3::z());
        let level_up = (Vector3::y() - forward * forward.y).try_normalize(1e-3).unwrap_or_else(|| rotation * Vector3::y());
        let level_left = level_up.cross(&forward);
        let up = rotation * Vector3::y();
        let bank = (up.dot(&level_left)).atan2(up.dot(&level_up));

        let (elevator, aileron, throttle) = if speed < FLYING_SPEED {
            // Still rolling: full power, wings level, stick centered.
            (0.0, Self::aileron_for(0.0, bank, capture, speed, position.y, flight_data.aoa_y), 1.0)
        } else {
            // How hard it wants to turn, sideways and up or down - each
            // from how far off it is that way, faster the further off (and
            // the eagerer the pilot).
            let rate = TURN_RATE_PER_ERROR * skill.turn_eagerness;
            let heading = forward.x.atan2(forward.z);
            let wanted_heading = wanted.x.atan2(wanted.z);
            // + toward its left (world +X when flying +Z, like its left wing).
            let heading_error = wrap_angle(wanted_heading - heading);
            let flight_path = forward.y.clamp(-1.0, 1.0).asin();
            let climb_error = wanted.y.clamp(-1.0, 1.0).asin() - flight_path;
            let sideways = heading_error * rate * speed;
            // Height is still chased more gently than a flat turn, but a
            // path that climbs or drops uses more of the G it has — a 2 G
            // cap arrives at the point still on the old flight path.
            let climb_g = (skill.max_g * 0.55).max(MAX_CLIMB_PULL_G);
            let vertical = (climb_error * rate * speed).clamp(-climb_g * GRAVITY, climb_g * GRAVITY);

            // The lift that takes, against wings level: up - the climb it
            // wants plus holding up against gravity - and sideways.
            let lift_up = vertical + GRAVITY * flight_path.cos();
            let lift_left = sideways;
            let max_bank = skill.max_bank_deg.to_radians();
            let max_lift = skill.max_g * GRAVITY;
            // Level, or a shallow path: upright. The lift it wants can
            // still point a little down when it's a touch high, and that
            // used to hold the inverted bank.
            let leveling = lift_up >= 0.0 || climb > -LEVEL_FLIGHT_DEG.to_radians();
            let (wanted_bank, wanted_g) = if leveling {
                let holding = lift_up.max(GRAVITY * flight_path.cos().max(0.25));
                let mut bank = lift_left.atan2(holding).clamp(-max_bank, max_bank);
                // More than it can pull: still holding its height, turning
                // as hard as its G allows - banked just enough.
                if lift_up >= 0.0 && (lift_up * lift_up + lift_left * lift_left).sqrt() > max_lift {
                    bank = lift_left.signum() * (lift_up / max_lift).clamp(-1.0, 1.0).acos().min(max_bank);
                }
                // Banked less than the turn wants, it turns wider rather
                // than climbing away: only the G that holds its height.
                // A touch high on a level leg: unload, and let the nose
                // settle, once the wings are back up.
                let g = if lift_up >= 0.0 { lift_up / bank.cos().max(0.15) / GRAVITY } else { 0.0 };
                (bank, g)
            } else {
                // The lift it wants points down the path. Roll until a pull
                // aims there - fully inverted, if that's where it points -
                // and then pull. The skill's turning bank doesn't apply:
                // that limit is for a level turn, and stopping short leaves
                // the lift aimed above the path. Until the wings are there
                // it holds 1 G, so it rolls all the way instead of pushing.
                //
                // The heading demand is in m/s² with no ceiling, and the
                // descent's is capped, so a miss to the side used to win
                // and park the bank near 90° - knife-edge, unloaded, the
                // pull never getting to drop the nose. While the nose is
                // still well above the path, that sideways share fades out
                // and the roll goes to inverted. The sign of the turn is
                // kept, so it inverts the way it's already rolling.
                let pitch_first = ((-climb_error).max(0.0) / PITCH_FIRST_DEG.to_radians()).clamp(0.0, 1.0);
                let aimed = sideways.clamp(-max_lift, max_lift) * (1.0 - pitch_first);
                let mut ideal = if aimed.abs() < 1.0 {
                    let sign = if bank.abs() > 5.0_f32.to_radians() {
                        bank.signum()
                    } else if sideways.abs() > 1.0 {
                        sideways.signum()
                    } else if bank < 0.0 {
                        -1.0
                    } else {
                        1.0
                    };
                    sign * (std::f32::consts::PI - 1.0e-3)
                } else {
                    aimed.atan2(lift_up)
                };
                // Nearly inverted is the same attitude either way around.
                // A few degrees of bank already started must not flip the
                // target onto the other 180 and roll all the way back.
                if bank.abs() > 5.0_f32.to_radians() && ideal.abs() > 150.0_f32.to_radians() && ideal.signum() != bank.signum() {
                    ideal = bank.signum() * ideal.abs();
                }
                let wanted_bank = bank + wrap_angle(ideal - bank);
                let wanted_g = if wanted_bank.cos() < -0.2 {
                    (lift_up / wanted_bank.cos() / GRAVITY).max(0.0)
                } else {
                    0.0
                };
                (wanted_bank, wanted_g)
            };
            // Lighter while it's still rolling there.
            let rolled = wrap_angle(wanted_bank - bank).cos().max(0.0);
            let wanted_g = if wanted_g > 1.0 { 1.0 + (wanted_g - 1.0) * rolled } else { wanted_g };
            let mut g = wanted_g.clamp(MIN_G, skill.max_g);
            // Near the angle of attack it's willing to fly: no harder than
            // it's pulling now.
            if flight_data.aoa_y > skill.max_aoa_deg {
                g = g.min(flight_data.g_meter.max(1.0));
            }

            let throttle = match self.speed {
                Speed::Max => 1.0,
                Speed::Hold(cruise_speed) => {
                    let mut wanted_speed = cruise_speed + WANDER_SPEED * sloppiness * self.wander(3.0);
                    if let Some(fit) = fit_speed {
                        wanted_speed = wanted_speed.min(fit);
                    }
                    // A sloppier pilot works the throttle more lazily.
                    let gain = THROTTLE_PER_SPEED_ERROR * (0.4 + 0.6 * skill.precision);
                    // On speed and level: military. Short of the speed: up
                    // through the gate, which lights the afterburner. A hard
                    // turn takes the lever back down so the corner can bleed
                    // speed, and it never adds power to do that.
                    let for_speed = (self.military + (wanted_speed - speed) * gain).clamp(0.0, 1.0);
                    let for_turn = (self.military - TURN_THROTTLE_DROP).max(0.0);
                    let turn_priority = ((g - 1.0) / TURN_PRIORITY_G).clamp(0.0, 1.0);
                    if for_turn < for_speed { for_speed + (for_turn - for_speed) * turn_priority } else { for_speed }
                }
            };
            // Inverted on a level leg: keep rolling the way the wings
            // already lean until they're upright. Following the bank
            // reading through 180° reverses the stick each frame.
            let aileron = if leveling && bank.cos() < 0.15 {
                let dir = *self.level_roll.get_or_insert(if bank < 0.0 { 1.0 } else { -1.0 });
                Self::aileron_for_error(dir * std::f32::consts::PI, capture, speed, position.y, flight_data.aoa_y)
            } else {
                self.level_roll = None;
                Self::aileron_for(wanted_bank, bank, capture, speed, position.y, flight_data.aoa_y)
            };
            (Self::elevator_for(g), aileron, throttle)
        };

        let ease = (delta_time * skill.control_rate).clamp(0.0, 1.0);
        controls.elevator += (elevator - controls.elevator) * ease;
        controls.aileron += (aileron - controls.aileron) * ease;
        controls.throttle += (throttle - controls.throttle) * ease;
        controls.rudder = 0.0;
        // Brakes off - it's going places.
        controls.parking_brake = false;
        controls.wheel_brake = 0.0;
    }

    /// The altitude it's been told to fly (m) - the lower of the points on
    /// the leg it's flying, or the last point once a path that doesn't loop
    /// is finished. Not where the aircraft happens to be: a dive mustn't
    /// drag the floor down with it.
    fn path_altitude(&self) -> f32 {
        if self.path.is_empty() {
            return MIN_ALTITUDE;
        }
        if self.finished {
            return self.path.last().map(|point| point.y).unwrap_or(MIN_ALTITUDE);
        }
        let point = self.path[self.next].y;
        let previous = if self.next > 0 {
            self.path[self.next - 1].y
        } else if self.looped && self.path.len() > 1 {
            self.path[self.path.len() - 1].y
        } else {
            point
        };
        point.min(previous)
    }

    /// How early it starts the turn for the leg after the one it's on (m):
    /// at `speed`, pulling its planned share of `max_g`, its turn radius
    /// times the tangent of half the corner's angle - where a turn that
    /// size meets both legs. Nothing without a leg after (or a straight
    /// one).
    fn turn_lead(&self, leg_start: Vector3<f32>, speed: f32, max_g: f32) -> f32 {
        self.lead_at(self.next, leg_start, speed, max_g)
    }

    /// The same lead for the corner at `vertex`, arrived at from
    /// `arrive_from`.
    fn lead_at(&self, vertex: usize, arrive_from: Vector3<f32>, speed: f32, max_g: f32) -> f32 {
        let point = self.path[vertex];
        let Some(after) = self.after(vertex).map(|index| self.path[index]) else { return 0.0 };
        let (Some(into), Some(out)) = ((point - arrive_from).try_normalize(1.0), (after - point).try_normalize(1.0)) else { return 0.0 };
        let corner = into.dot(&out).clamp(-1.0, 1.0).acos();
        if corner < 1.0_f32.to_radians() {
            return 0.0;
        }
        let g = (max_g * PLANNED_G_SHARE).max(1.5);
        let radius = speed * speed / (GRAVITY * (g * g - 1.0).sqrt());
        (radius * (corner * 0.5).tan()).min(radius * MAX_LEAD_RADII)
    }

    /// Where it steers: a few seconds ahead on the leg, blended toward the
    /// leg leaving a corner it's already close enough to set up for. Looks
    /// up to `PLAN_STEPS` corners ahead, so a tight run of points is turned
    /// for before the first of them, with the roll started early enough
    /// that the wings are there when the bend is.
    fn aim_spot(&self, position: Vector3<f32>, speed: f32, max_g: f32, capture: f32) -> Vector3<f32> {
        let lookahead = (speed * LOOKAHEAD_SECONDS).clamp(MIN_LOOKAHEAD, MAX_LOOKAHEAD);
        let mut from = self.leg_start.unwrap_or(position);
        let mut cursor = position;
        let mut travelled = 0.0;
        // The furthest corner already inside its setup, and how far through
        // that setup the aircraft is (0 at the start, 1 at the corner).
        let mut plan: Option<(usize, f32)> = None;
        for step in 0..PLAN_STEPS {
            let Some(vertex) = self.ahead(step) else { break };
            let point = self.path[vertex];
            let leg = point - from;
            let len = leg.magnitude().max(1e-3);
            let along = (cursor - from).dot(&(leg / len)).clamp(0.0, len);
            let remaining = len - along;
            let distance = travelled + remaining;
            let lead = self.lead_at(vertex, from, speed, max_g);
            let roll_in = speed * capture;
            let setup = lead + roll_in;
            let in_setup = setup > 50.0 && distance <= setup;
            if in_setup {
                // The roll starts out in the roll-in, while the nose is
                // still on this leg. Once the turn radius says the bend
                // is here, the aim commits to the leg that leaves it.
                let through = if lead < 1.0 {
                    1.0
                } else if distance > lead {
                    let started = (1.0 - (distance - lead) / roll_in.max(1.0)).clamp(0.0, 1.0);
                    started * 0.2
                } else {
                    1.0
                };
                plan = Some((vertex, through));
            }
            let leave = self.after(vertex).map(|index| (self.path[index] - point).magnitude()).unwrap_or(0.0);
            // A leg long enough for its own bend is flown on its own. A
            // shorter one is part of the same plan, so the corner after
            // it is looked at too.
            if in_setup && leave > lead.max(50.0) {
                break;
            }
            if distance > (speed * PLAN_HORIZON_SECONDS).max(MIN_LOOKAHEAD) {
                break;
            }
            travelled += remaining;
            cursor = point;
            from = point;
        }

        let on_leg = self.carrot(position, lookahead);
        let Some((vertex, through)) = plan else { return on_leg - position };
        let Some(after) = self.after(vertex) else { return on_leg - position };
        let point = self.path[vertex];
        let out = self.path[after] - point;
        let Some(dir) = out.try_normalize(1.0) else { return on_leg - position };
        let exit = point + dir * lookahead.min(out.magnitude());
        // `through` is a taste of the next leg while the wings roll in, and
        // the whole of it once the turn itself is due.
        (on_leg * (1.0 - through) + exit * through) - position
    }

    /// A spot `lookahead` metres ahead of `position` on the leg it's
    /// flying, or the point itself once that's nearer.
    fn carrot(&self, position: Vector3<f32>, lookahead: f32) -> Vector3<f32> {
        let point = self.path[self.next];
        let from = self.leg_start.unwrap_or(position);
        let leg = point - from;
        let length = leg.magnitude();
        if length > 1.0 {
            let along_leg = leg / length;
            let along = (position - from).dot(&along_leg).clamp(0.0, length);
            let ahead = along + lookahead;
            if ahead >= length { point } else { from + along_leg * ahead }
        } else {
            point
        }
    }

    /// The speed (m/s) an upcoming corner can be flown at, when the speed
    /// it's at now makes a turn too wide for the legs around it. `None`
    /// when the next few corners fit. Looked for out to
    /// `PLAN_HORIZON_SECONDS`, so the throttle is back before the turn.
    fn speed_for_corners(&self, position: Vector3<f32>, speed: f32, max_g: f32) -> Option<f32> {
        let horizon = (speed * PLAN_HORIZON_SECONDS).max(MIN_LOOKAHEAD);
        let mut from = self.leg_start.unwrap_or(position);
        let mut cursor = position;
        let mut travelled = 0.0;
        let mut fit: Option<f32> = None;
        for step in 0..PLAN_STEPS {
            let Some(vertex) = self.ahead(step) else { break };
            let point = self.path[vertex];
            let leg = point - from;
            let len = leg.magnitude().max(1e-3);
            let along = (cursor - from).dot(&(leg / len)).clamp(0.0, len);
            let distance = travelled + (len - along);
            if distance > horizon {
                break;
            }
            let leave = self.after(vertex).map(|index| (self.path[index] - point).magnitude()).unwrap_or(len);
            let room = len.min(leave) * 0.5;
            let lead = self.lead_at(vertex, from, speed, max_g);
            if lead > room.max(50.0) && room > 50.0 {
                // The lead grows with the square of the speed.
                let slower = speed * (room / lead).sqrt();
                fit = Some(fit.map_or(slower, |have| have.min(slower)));
            }
            travelled = distance;
            cursor = point;
            from = point;
        }
        fit.map(|slower| slower.clamp(FLYING_SPEED * 1.5, speed))
    }

    /// The point `steps` corners ahead of the one it's flying to, if the
    /// path has one.
    fn ahead(&self, steps: usize) -> Option<usize> {
        if self.finished || self.path.is_empty() {
            return None;
        }
        let index = self.next + steps;
        if index < self.path.len() {
            Some(index)
        } else if self.looped {
            Some(index % self.path.len())
        } else {
            None
        }
    }

    /// The point after `vertex`.
    fn after(&self, vertex: usize) -> Option<usize> {
        let index = vertex + 1;
        if index < self.path.len() {
            Some(index)
        } else if self.looped && self.path.len() > 1 {
            Some(0)
        } else {
            None
        }
    }

    /// Whether `position` is past `point` along the leg from `leg_start` -
    /// across the plane through `point` square to that leg.
    fn crossed(leg_start: Vector3<f32>, point: Vector3<f32>, position: Vector3<f32>) -> bool {
        let leg = point - leg_start;
        leg.norm_squared() > 1.0 && (position - point).dot(&leg) >= 0.0
    }

    /// The stick for `g` with the pitch fly-by-wire on. `elevator` is + for
    /// stick forward (nose down), so pulling is negative.
    fn elevator_for(g: f32) -> f32 {
        let pull = if g >= STICK_CENTER_G {
            (g - STICK_CENTER_G) / (STICK_MAX_G - STICK_CENTER_G)
        } else {
            (g - STICK_CENTER_G) / (STICK_CENTER_G - STICK_MIN_G)
        };
        -pull.clamp(-1.0, 1.0)
    }

    /// The aileron that rolls from `bank` toward `wanted_bank` (both + left
    /// wing down) in `capture_seconds`, stopped when it gets there.
    /// `aileron` is - for rolling left. The stick is that roll rate as a
    /// share of what full aileron rolls at this `airspeed` (m/s), `altitude`
    /// (m) and `aoa_deg` - the same schedule the roll fly-by-wire flies - so
    /// the same bank error isn't a harder roll just because it's going faster.
    fn aileron_for(wanted_bank: f32, bank: f32, capture_seconds: f32, airspeed: f32, altitude: f32, aoa_deg: f32) -> f32 {
        Self::aileron_for_error(wanted_bank - bank, capture_seconds, airspeed, altitude, aoa_deg)
    }

    /// The aileron for a bank error (radians, + needs more left wing down)
    /// closed in `capture_seconds`. See `aileron_for`.
    fn aileron_for_error(error: f32, capture_seconds: f32, airspeed: f32, altitude: f32, aoa_deg: f32) -> f32 {
        let available = commanded_roll_rate_deg_s(1.0, airspeed, altitude, aoa_deg.abs(), &RollRateParams::default()).to_radians();
        // Sitting still: nothing to roll with.
        if available < 1.0_f32.to_radians() {
            return 0.0;
        }
        let wanted_rate = (error / capture_seconds.max(1e-3)).clamp(-available, available);
        // +bank rate (left wing down) is negative aileron.
        -(wanted_rate / available).clamp(-MAX_AILERON, MAX_AILERON)
    }
}

const GRAVITY: f32 = 9.81;

/// `angle` (radians) wrapped into -PI..PI.
fn wrap_angle(angle: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    (angle + PI).rem_euclid(TAU) - PI
}

#[cfg(test)]
mod tests {
    use super::*;
    fn medium() -> Difficulty {
        Difficulty::Medium
    }

    #[test]
    fn a_path_that_doesnt_loop_is_flown_once() {
        let mut autopilot = Autopilot::new(&[(0.0, 1000.0, 100.0)], Speed::Hold(150.0), medium(), false);
        let mut controls = PlaneControls::new();
        let mut flight_data = FlightData::new();
        flight_data.velocity = Vector3::new(0.0, 0.0, 150.0);
        autopilot.fly(&mut controls, Vector3::new(0.0, 1000.0, 0.0), UnitQuaternion::identity(), &flight_data, 0.1);
        assert!(autopilot.is_finished());
        assert_eq!(autopilot.next_point(), None);
    }

    #[test]
    fn an_ace_rolls_into_a_turn_quicker_than_a_beginner() {
        // A hard turn - full aileron for both; the Ace's hands get there
        // sooner.
        let aileron_after = |difficulty| {
            let mut autopilot = Autopilot::new(&[(5000.0, 1000.0, 1000.0)], Speed::Hold(150.0), difficulty, true);
            let mut controls = PlaneControls::new();
            let mut flight_data = FlightData::new();
            flight_data.velocity = Vector3::new(0.0, 0.0, 150.0);
            autopilot.fly(&mut controls, Vector3::new(0.0, 1000.0, 0.0), UnitQuaternion::identity(), &flight_data, 0.05);
            controls.aileron
        };
        let (ace, beginner) = (aileron_after(Difficulty::Ace), aileron_after(Difficulty::Beginner));
        assert!(ace < 0.0 && beginner < 0.0, "both roll left: {ace}, {beginner}");
        assert!(ace.abs() > beginner.abs(), "{ace} vs {beginner}");
    }

    #[test]
    fn stick_centered_is_one_g_and_pulling_is_negative() {
        assert_eq!(Autopilot::elevator_for(1.0), 0.0);
        assert!((Autopilot::elevator_for(9.0) + 1.0).abs() < 1e-6);
        assert!(Autopilot::elevator_for(3.0) < 0.0);
        assert!(Autopilot::elevator_for(0.0) > 0.0);
    }

    #[test]
    fn it_banks_toward_a_point_off_its_left() {
        let mut autopilot = Autopilot::new(&[(5000.0, 1000.0, 5000.0)], Speed::Hold(150.0), medium(), true);
        let mut controls = PlaneControls::new();
        let mut flight_data = FlightData::new();
        // Flying +Z, level - world +X is off its left wing.
        flight_data.velocity = Vector3::new(0.0, 0.0, 150.0);
        autopilot.fly(&mut controls, Vector3::new(0.0, 1000.0, 0.0), UnitQuaternion::identity(), &flight_data, 1.0);
        assert!(controls.aileron < 0.0, "should roll left, aileron {}", controls.aileron);
    }

    #[test]
    fn it_moves_on_past_a_reached_point() {
        let mut autopilot = Autopilot::new(&[(0.0, 1000.0, 100.0), (0.0, 1000.0, 5000.0)], Speed::Hold(150.0), medium(), true);
        let mut controls = PlaneControls::new();
        let mut flight_data = FlightData::new();
        flight_data.velocity = Vector3::new(0.0, 0.0, 150.0);
        autopilot.fly(&mut controls, Vector3::new(0.0, 1000.0, 0.0), UnitQuaternion::identity(), &flight_data, 0.1);
        assert_eq!(autopilot.next_point(), Some(1));
    }

    #[test]
    fn a_point_behind_the_nose_isnt_skipped() {
        // Point 2 is close behind - the path turns back on itself. Reaching
        // point 1 has to lead on to point 2, not past it to point 3.
        let mut autopilot = Autopilot::new(&[(0.0, 1000.0, 1000.0), (0.0, 1000.0, 400.0), (0.0, 1000.0, 5000.0)], Speed::Hold(150.0), medium(), true);
        let mut controls = PlaneControls::new();
        let mut flight_data = FlightData::new();
        flight_data.velocity = Vector3::new(0.0, 0.0, 150.0);
        // Sets off, then reaches point 1 heading +Z (point 2 now behind it).
        autopilot.fly(&mut controls, Vector3::new(0.0, 1000.0, 0.0), UnitQuaternion::identity(), &flight_data, 0.1);
        autopilot.fly(&mut controls, Vector3::new(0.0, 1000.0, 950.0), UnitQuaternion::identity(), &flight_data, 0.1);
        assert_eq!(autopilot.next_point(), Some(1));
        // Still flying away from point 2, past where it was - not skipped.
        autopilot.fly(&mut controls, Vector3::new(0.0, 1000.0, 1100.0), UnitQuaternion::identity(), &flight_data, 0.1);
        autopilot.fly(&mut controls, Vector3::new(200.0, 1000.0, 1300.0), UnitQuaternion::identity(), &flight_data, 0.1);
        assert_eq!(autopilot.next_point(), Some(1), "point 2 has to be flown to, not skipped");
    }

    #[test]
    fn crossing_a_points_finish_line_moves_on() {
        // Missed point 1 wide (2 km off to the side), but went past it along
        // the leg - on to point 2, no circling back.
        let mut autopilot = Autopilot::new(&[(0.0, 1000.0, 3000.0), (0.0, 1000.0, 9000.0)], Speed::Hold(150.0), medium(), true);
        let mut controls = PlaneControls::new();
        let mut flight_data = FlightData::new();
        flight_data.velocity = Vector3::new(0.0, 0.0, 150.0);
        autopilot.fly(&mut controls, Vector3::new(0.0, 1000.0, 0.0), UnitQuaternion::identity(), &flight_data, 0.1);
        autopilot.fly(&mut controls, Vector3::new(2000.0, 1000.0, 2900.0), UnitQuaternion::identity(), &flight_data, 0.1);
        assert_eq!(autopilot.next_point(), Some(0), "not across yet");
        autopilot.fly(&mut controls, Vector3::new(2000.0, 1000.0, 3050.0), UnitQuaternion::identity(), &flight_data, 0.1);
        assert_eq!(autopilot.next_point(), Some(1));
    }

    #[test]
    fn off_the_line_it_turns_back_onto_it_not_straight_at_the_point() {
        // The leg runs up +Z from the origin; it's 2 km off to its left
        // (+X), flying parallel. Straight at the far point would be a gentle
        // turn - getting back on the line is a hard one.
        let mut autopilot = Autopilot::new(&[(0.0, 1000.0, 0.0), (0.0, 1000.0, 20000.0)], Speed::Hold(150.0), medium(), false);
        let mut controls = PlaneControls::new();
        let mut flight_data = FlightData::new();
        flight_data.velocity = Vector3::new(0.0, 0.0, 150.0);
        autopilot.fly(&mut controls, Vector3::new(0.0, 1000.0, 0.0), UnitQuaternion::identity(), &flight_data, 0.1);
        assert_eq!(autopilot.next_point(), Some(1));
        let mut controls = PlaneControls::new();
        autopilot.fly(&mut controls, Vector3::new(2000.0, 1000.0, 10000.0), UnitQuaternion::identity(), &flight_data, 1.0);
        // Rolling right, back onto the line - the rate that meets that bank,
        // not full stick.
        assert!(controls.aileron > 0.3, "aileron {}", controls.aileron);
    }

    #[test]
    fn the_same_bank_error_is_less_stick_when_its_going_faster() {
        let capture = 1.0;
        let bank_error = 20.0_f32.to_radians();
        let slow = Autopilot::aileron_for(bank_error, 0.0, capture, 120.0, 1000.0, 0.0);
        let fast = Autopilot::aileron_for(bank_error, 0.0, capture, 250.0, 1000.0, 0.0);
        assert!(slow < 0.0 && fast < 0.0, "both roll left: {slow}, {fast}");
        assert!(fast.abs() < slow.abs(), "{fast} vs {slow}");
    }

    #[test]
    fn level_at_the_speed_it_wants_is_military_power() {
        let mut autopilot = Autopilot::new(&[(0.0, 1000.0, 20000.0)], Speed::Hold(150.0), Difficulty::Ace, false).with_military(0.85);
        let mut controls = PlaneControls::new();
        let mut flight_data = FlightData::new();
        flight_data.velocity = Vector3::new(0.0, 0.0, 150.0);
        autopilot.fly(&mut controls, Vector3::new(0.0, 1000.0, 0.0), UnitQuaternion::identity(), &flight_data, 1.0);
        assert!((controls.throttle - 0.85).abs() < 0.05, "throttle {}", controls.throttle);
    }

    #[test]
    fn short_of_speed_it_lights_the_afterburner() {
        let mut autopilot = Autopilot::new(&[(0.0, 1000.0, 20000.0)], Speed::Hold(150.0), Difficulty::Ace, false).with_military(0.85);
        let mut controls = PlaneControls::new();
        let mut flight_data = FlightData::new();
        flight_data.velocity = Vector3::new(0.0, 0.0, 100.0);
        autopilot.fly(&mut controls, Vector3::new(0.0, 1000.0, 0.0), UnitQuaternion::identity(), &flight_data, 1.0);
        assert!(controls.throttle > 0.95, "throttle {}", controls.throttle);
    }

    #[test]
    fn a_hard_turn_comes_off_the_power() {
        // Already rolled into the turn, so the pull is on and the corner
        // matters more than the speed.
        let mut autopilot = Autopilot::new(&[(8000.0, 1000.0, 500.0)], Speed::Hold(150.0), Difficulty::Ace, false).with_military(0.85);
        let mut controls = PlaneControls::new();
        let mut flight_data = FlightData::new();
        flight_data.velocity = Vector3::new(0.0, 0.0, 150.0);
        let banked = UnitQuaternion::from_axis_angle(&Vector3::z_axis(), -80.0_f32.to_radians());
        autopilot.fly(&mut controls, Vector3::new(0.0, 1000.0, 0.0), banked, &flight_data, 1.0);
        assert!(controls.throttle < 0.55, "throttle {}", controls.throttle);
    }

    #[test]
    fn a_path_below_the_floor_is_flown_level() {
        let mut autopilot = Autopilot::new(&[(0.0, 30.0, 20000.0)], Speed::Hold(150.0), Difficulty::Ace, false);
        let mut controls = PlaneControls::new();
        let mut flight_data = FlightData::new();
        flight_data.velocity = Vector3::new(0.0, 0.0, 150.0);
        autopilot.fly(&mut controls, Vector3::new(0.0, 30.0, 0.0), UnitQuaternion::identity(), &flight_data, 1.0);
        assert!(controls.elevator.abs() < 0.05, "elevator {}", controls.elevator);
    }

    #[test]
    fn below_a_high_path_it_still_climbs() {
        // Flown off the end of a high path, then down low: straight on would
        // be level, and the floor still climbs it back up.
        let mut autopilot = Autopilot::new(&[(0.0, 1000.0, 0.0)], Speed::Hold(150.0), Difficulty::Ace, false);
        let mut controls = PlaneControls::new();
        let mut flight_data = FlightData::new();
        flight_data.velocity = Vector3::new(0.0, 0.0, 150.0);
        autopilot.fly(&mut controls, Vector3::new(0.0, 1000.0, 0.0), UnitQuaternion::identity(), &flight_data, 0.1);
        assert!(autopilot.is_finished());
        let mut controls = PlaneControls::new();
        autopilot.fly(&mut controls, Vector3::new(0.0, 40.0, 0.0), UnitQuaternion::identity(), &flight_data, 1.0);
        assert!(controls.elevator < -0.1, "elevator {}", controls.elevator);
    }

    #[test]
    fn a_descent_rolls_until_the_lift_points_down_the_path() {
        // Already well past vertical, the path still below it. The bank it
        // wants is inverted, so it keeps rolling - it doesn't stop at its
        // skill's turning bank, and it doesn't push.
        let mut autopilot = Autopilot::new(&[(0.0, -1000.0, 4000.0)], Speed::Hold(200.0), Difficulty::Ace, false);
        let mut controls = PlaneControls::new();
        let mut flight_data = FlightData::new();
        flight_data.velocity = Vector3::new(0.0, 0.0, 200.0);
        let banked = UnitQuaternion::from_axis_angle(&Vector3::z_axis(), -110.0_f32.to_radians());
        autopilot.fly(&mut controls, Vector3::new(0.0, 1000.0, 0.0), banked, &flight_data, 1.0);
        assert!(controls.aileron < -0.15, "aileron {}", controls.aileron);
        assert!(controls.elevator < 0.2, "elevator {}", controls.elevator);
    }

    #[test]
    fn a_level_leg_rolls_out_of_inverted() {
        // Inverted, the path level ahead. It rolls back to wings level,
        // and keeps that roll when the bank reading flips across 180°.
        let mut autopilot = Autopilot::new(&[(0.0, 1000.0, 8000.0)], Speed::Hold(200.0), Difficulty::Ace, false);
        let mut controls = PlaneControls::new();
        let mut flight_data = FlightData::new();
        flight_data.velocity = Vector3::new(0.0, 0.0, 200.0);
        let inverted = UnitQuaternion::from_axis_angle(&Vector3::z_axis(), -179.0_f32.to_radians());
        autopilot.fly(&mut controls, Vector3::new(0.0, 1000.0, 0.0), inverted, &flight_data, 1.0);
        let first = controls.aileron;
        assert!(first.abs() > 0.3, "aileron {first}");
        let flipped = UnitQuaternion::from_axis_angle(&Vector3::z_axis(), 179.0_f32.to_radians());
        autopilot.fly(&mut controls, Vector3::new(0.0, 1000.0, 0.0), flipped, &flight_data, 1.0);
        assert!(controls.aileron.signum() == first.signum(), "first {first} then {}", controls.aileron);
        assert!(controls.aileron.abs() > 0.3, "aileron {}", controls.aileron);
    }

    #[test]
    fn a_few_degrees_of_bank_doesnt_reverse_the_inversion() {
        // Already rolled a little left. The lift it wants is a hair the
        // other way, which used to read as -180 and roll it all the way
        // right. It finishes the roll it started.
        let mut autopilot = Autopilot::new(&[( -40.0, 0.0, 5000.0)], Speed::Hold(200.0), Difficulty::Ace, false);
        let mut controls = PlaneControls::new();
        let mut flight_data = FlightData::new();
        flight_data.velocity = Vector3::new(0.0, 0.0, 200.0);
        let banked = UnitQuaternion::from_axis_angle(&Vector3::z_axis(), -20.0_f32.to_radians());
        autopilot.fly(&mut controls, Vector3::new(0.0, 2000.0, 0.0), banked, &flight_data, 1.0);
        assert!(controls.aileron < -0.15, "aileron {}", controls.aileron);
    }

    #[test]
    fn it_starts_turning_before_the_corner_is_reached() {
        // A left turn a kilometre ahead. That's inside the setup (the turn
        // radius, plus the time the wings take to roll in) and still short
        // of where the point counts as reached, so the roll has already
        // started.
        let mut autopilot = Autopilot::new(&[(0.0, 1000.0, 3000.0), (3000.0, 1000.0, 3000.0)], Speed::Hold(200.0), medium(), false);
        let mut controls = PlaneControls::new();
        let mut flight_data = FlightData::new();
        flight_data.velocity = Vector3::new(0.0, 0.0, 200.0);
        autopilot.fly(&mut controls, Vector3::new(0.0, 1000.0, 2100.0), UnitQuaternion::identity(), &flight_data, 1.0);
        assert_eq!(autopilot.next_point(), Some(0));
        assert!(controls.aileron < -0.05, "aileron {}", controls.aileron);
    }

    #[test]
    fn a_far_corner_is_not_turned_for_yet() {
        let mut autopilot = Autopilot::new(&[(0.0, 1000.0, 8000.0), (3000.0, 1000.0, 8000.0)], Speed::Hold(200.0), medium(), false);
        let mut controls = PlaneControls::new();
        let mut flight_data = FlightData::new();
        flight_data.velocity = Vector3::new(0.0, 0.0, 200.0);
        autopilot.fly(&mut controls, Vector3::new(0.0, 1000.0, 0.0), UnitQuaternion::identity(), &flight_data, 1.0);
        assert!(controls.aileron.abs() < 0.05, "aileron {}", controls.aileron);
    }

    #[test]
    fn a_corner_the_speed_cant_make_is_slowed_for_ahead_of_time() {
        // The leg after the corner is short, so the turn at this speed
        // can't sit on it. A couple of kilometres out, the throttle is
        // already coming back.
        let mut autopilot = Autopilot::new(&[(0.0, 1000.0, 2000.0), (400.0, 1000.0, 2000.0)], Speed::Hold(220.0), Difficulty::Ace, false);
        let mut controls = PlaneControls::new();
        let mut flight_data = FlightData::new();
        flight_data.velocity = Vector3::new(0.0, 0.0, 220.0);
        autopilot.fly(&mut controls, Vector3::new(0.0, 1000.0, 0.0), UnitQuaternion::identity(), &flight_data, 1.0);
        assert_eq!(autopilot.next_point(), Some(0));
        assert!(controls.throttle < 0.5, "throttle {}", controls.throttle);
    }

    #[test]
    fn an_offset_descent_rolls_past_knife_edge() {
        // Nose still climbing, the next point far below and off to the
        // side. The sideways miss used to pin the bank near 90° and unload.
        // The roll keeps going toward inverted, including once it's already
        // there at knife-edge, and it is not pushing to get the nose down.
        let mut autopilot = Autopilot::new(&[(0.0, 4500.0, 7000.0), (0.0, 500.0, 10000.0)], Speed::Hold(220.0), Difficulty::Ace, false);
        let mut flight_data = FlightData::new();
        flight_data.velocity = Vector3::new(0.0, 1.0, 1.0).normalize() * 220.0;
        let mut controls = PlaneControls::new();
        let nose_up = UnitQuaternion::from_axis_angle(&Vector3::x_axis(), -45.0_f32.to_radians());
        autopilot.fly(&mut controls, Vector3::new(0.0, 4500.0, 7000.0), nose_up, &flight_data, 0.05);
        assert_eq!(autopilot.next_point(), Some(1));
        let mut controls = PlaneControls::new();
        autopilot.fly(&mut controls, Vector3::new(800.0, 4000.0, 7800.0), nose_up, &flight_data, 1.0);
        let started = controls.aileron;
        assert!(started.abs() > 0.4, "aileron {started}");
        assert!(controls.elevator < 0.2, "elevator {}", controls.elevator);
        let knife = nose_up * UnitQuaternion::from_axis_angle(&Vector3::z_axis(), if started > 0.0 { 90.0_f32.to_radians() } else { -90.0_f32.to_radians() });
        let mut controls = PlaneControls::new();
        autopilot.fly(&mut controls, Vector3::new(800.0, 4000.0, 7800.0), knife, &flight_data, 1.0);
        assert!(controls.aileron.signum() == started.signum(), "started {started} knife {}", controls.aileron);
        assert!(controls.aileron.abs() > 0.2, "aileron {}", controls.aileron);
        assert!(controls.elevator < 0.2, "elevator {}", controls.elevator);
    }

    #[test]
    fn a_beginner_descent_also_rolls_toward_inverted() {
        // Wings level, the path dropping away. Even a beginner is told to
        // roll all the way over, not just to its 45° turning bank.
        let mut autopilot = Autopilot::new(&[(0.0, -1000.0, 4000.0)], Speed::Hold(200.0), Difficulty::Beginner, false);
        let mut controls = PlaneControls::new();
        let mut flight_data = FlightData::new();
        flight_data.velocity = Vector3::new(0.0, 0.0, 200.0);
        autopilot.fly(&mut controls, Vector3::new(0.0, 1000.0, 0.0), UnitQuaternion::identity(), &flight_data, 1.0);
        assert!(controls.aileron.abs() > 0.1, "aileron {}", controls.aileron);
        assert!(controls.elevator < 0.4, "elevator {}", controls.elevator);
    }

    #[test]
    fn angles_wrap() {
        use std::f32::consts::PI;
        assert!((wrap_angle(1.5 * PI) + 0.5 * PI).abs() < 1e-5);
        assert!((wrap_angle(-1.5 * PI) - 0.5 * PI).abs() < 1e-5);
    }
}
