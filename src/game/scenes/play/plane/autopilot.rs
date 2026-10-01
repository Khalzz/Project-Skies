//! A bot's pilot: flies the aircraft through its path's points (see
//! play::map's `path`), strictly in order, round and round - by moving the same
//! stick and throttle a player would (`PlaneControls`), so the flight
//! physics fly it, fly-by-wire and all.
//!
//! How it steers: it banks toward the next point (more bank the further off
//! its heading the point is), pulls the G that banked turn needs to hold its
//! climb or descent toward the point, and works the throttle to hold its
//! cruise speed. With the pitch fly-by-wire on (the default - see `Fcs`) the
//! stick commands G, which is exactly what it works out; the roll
//! fly-by-wire turns its aileron into a roll rate.

use nalgebra::{UnitQuaternion, Vector3};

use super::controls::PlaneControls;
use super::flight_data::FlightData;

/// Within this far of a point (m) it counts as reached - on to the next.
/// Otherwise a point counts as reached once the aircraft crosses its finish
/// line: the plane through the point square to the leg flown to it (see
/// `Autopilot::crossed`) - so a point is only ever passed by flying that
/// leg, never skipped for being behind the nose, and one it can't turn
/// tightly enough to hit doesn't keep it circling.
const REACH_RADIUS: f32 = 300.0;
/// Steepest bank it turns with (deg), and how much bank per degree it's off
/// its heading toward the point.
const MAX_BANK_DEG: f32 = 60.0;
const BANK_PER_HEADING_ERROR: f32 = 1.5;
/// Steepest climb or dive it flies toward a point (deg).
const MAX_FLIGHT_PATH_DEG: f32 = 25.0;
/// Extra G per radian of climb/dive angle short of where it wants to be.
const G_PER_FLIGHT_PATH_ERROR: f32 = 5.0;
/// Least and most G it pulls.
const MIN_G: f32 = -1.0;
const MAX_G: f32 = 5.0;
/// The pitch fly-by-wire's stick range - stick centered is +1 G, full back
/// +9, full forward -2 (see physics::pitch_flcs).
const STICK_CENTER_G: f32 = 1.0;
const STICK_MAX_G: f32 = 9.0;
const STICK_MIN_G: f32 = -2.0;
/// Aileron per radian of bank short of the bank it wants, and the most it
/// uses.
const AILERON_PER_BANK_ERROR: f32 = 1.5;
const MAX_AILERON: f32 = 0.8;
/// Throttle per m/s short of its cruise speed (on top of half throttle).
const THROTTLE_PER_SPEED_ERROR: f32 = 0.02;
/// Below this airspeed (m/s) it isn't flying yet (a takeoff roll): full
/// throttle, wings level, stick centered.
const FLYING_SPEED: f32 = 60.0;
/// Below this altitude (m) it won't dive - climbing at least gently.
const MIN_ALTITUDE: f32 = 150.0;
const MIN_ALTITUDE_CLIMB_DEG: f32 = 5.0;
/// How fast its hands move to the new stick/throttle position (per second)
/// - smooth, not a jump every frame.
const CONTROL_RATE: f32 = 4.0;

pub struct Autopilot {
    path: Vec<Vector3<f32>>,
    /// The point it's flying to - an index into `path`.
    next: usize,
    /// Where the leg to `next` started - the point before it, or where the
    /// aircraft was when it set off for its first. `None` until it's flown.
    leg_start: Option<Vector3<f32>>,
    /// The airspeed it holds (m/s).
    cruise_speed: f32,
}

impl Autopilot {
    pub fn new(path: &[(f32, f32, f32)], cruise_speed: f32) -> Self {
        Self {
            path: path.iter().map(|&(x, y, z)| Vector3::new(x, y, z)).collect(),
            next: 0,
            leg_start: None,
            cruise_speed,
        }
    }

    /// The point it's flying to - an index into its path. `None` without a
    /// path.
    pub fn next_point(&self) -> Option<usize> {
        (!self.path.is_empty()).then_some(self.next)
    }

    /// Moves `controls` toward what flies it to its next point, for this
    /// frame - `position`/`rotation` where the aircraft is now. Leaves them
    /// alone without a path.
    pub fn fly(&mut self, controls: &mut PlaneControls, position: Vector3<f32>, rotation: UnitQuaternion<f32>, flight_data: &FlightData, delta_time: f32) {
        if self.path.is_empty() {
            return;
        }
        let velocity = flight_data.velocity;
        let speed = velocity.magnitude();

        // On to the next point - only this one, in order - once it's
        // reached or its finish line is crossed.
        let leg_start = *self.leg_start.get_or_insert(position);
        let point = self.path[self.next];
        if (point - position).magnitude() < REACH_RADIUS || Self::crossed(leg_start, point, position) {
            self.next = (self.next + 1) % self.path.len();
            // The next leg starts at the point just reached - or, for a
            // one-point path (the same point again), where it is now.
            self.leg_start = Some(if self.path[self.next] == point { position } else { point });
        }
        let to_point = self.path[self.next] - position;

        // Its bank, + left wing down (+X is the left wing, +Y up).
        let left_wing = rotation * Vector3::x();
        let up = rotation * Vector3::y();
        let bank = (-left_wing.y).atan2(up.y);

        let (elevator, aileron, throttle) = if speed < FLYING_SPEED {
            // Still rolling: full power, wings level, stick centered.
            (0.0, Self::aileron_for(0.0, bank), 1.0)
        } else {
            // Heading: + when the point is off to its left (world +X when
            // flying +Z, like its own left wing).
            let heading = velocity.x.atan2(velocity.z);
            let wanted_heading = to_point.x.atan2(to_point.z);
            let heading_error = wrap_angle(wanted_heading - heading);
            let max_bank = MAX_BANK_DEG.to_radians();
            let wanted_bank = (heading_error * BANK_PER_HEADING_ERROR).clamp(-max_bank, max_bank);

            // Climb/dive angle toward the point - never down low.
            let max_flight_path = MAX_FLIGHT_PATH_DEG.to_radians();
            let mut wanted_flight_path = (to_point.y / to_point.magnitude().max(1.0)).asin().clamp(-max_flight_path, max_flight_path);
            if position.y < MIN_ALTITUDE {
                wanted_flight_path = wanted_flight_path.max(MIN_ALTITUDE_CLIMB_DEG.to_radians());
            }
            let flight_path = (velocity.y / speed).asin();

            // The G a banked turn needs to hold its climb, plus a correction
            // toward the climb it wants.
            let level_g = flight_path.cos() / bank.cos().max(0.3);
            let g = (level_g + (wanted_flight_path - flight_path) * G_PER_FLIGHT_PATH_ERROR).clamp(MIN_G, MAX_G);

            let throttle = (0.5 + (self.cruise_speed - speed) * THROTTLE_PER_SPEED_ERROR).clamp(0.0, 1.0);
            (Self::elevator_for(g), Self::aileron_for(wanted_bank, bank), throttle)
        };

        let ease = (delta_time * CONTROL_RATE).clamp(0.0, 1.0);
        controls.elevator += (elevator - controls.elevator) * ease;
        controls.aileron += (aileron - controls.aileron) * ease;
        controls.throttle += (throttle - controls.throttle) * ease;
        controls.rudder = 0.0;
        // Brakes off - it's going places.
        controls.parking_brake = false;
        controls.wheel_brake = 0.0;
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
    /// wing down). `aileron` is - for rolling left.
    fn aileron_for(wanted_bank: f32, bank: f32) -> f32 {
        (-(wanted_bank - bank) * AILERON_PER_BANK_ERROR).clamp(-MAX_AILERON, MAX_AILERON)
    }
}

/// `angle` (radians) wrapped into -PI..PI.
fn wrap_angle(angle: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    (angle + PI).rem_euclid(TAU) - PI
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stick_centered_is_one_g_and_pulling_is_negative() {
        assert_eq!(Autopilot::elevator_for(1.0), 0.0);
        assert!((Autopilot::elevator_for(9.0) + 1.0).abs() < 1e-6);
        assert!(Autopilot::elevator_for(3.0) < 0.0);
        assert!(Autopilot::elevator_for(0.0) > 0.0);
    }

    #[test]
    fn it_banks_toward_a_point_off_its_left() {
        let mut autopilot = Autopilot::new(&[(5000.0, 1000.0, 5000.0)], 150.0);
        let mut controls = PlaneControls::new();
        let mut flight_data = FlightData::new();
        // Flying +Z, level - world +X is off its left wing.
        flight_data.velocity = Vector3::new(0.0, 0.0, 150.0);
        autopilot.fly(&mut controls, Vector3::new(0.0, 1000.0, 0.0), UnitQuaternion::identity(), &flight_data, 1.0);
        assert!(controls.aileron < 0.0, "should roll left, aileron {}", controls.aileron);
    }

    #[test]
    fn it_moves_on_past_a_reached_point() {
        let mut autopilot = Autopilot::new(&[(0.0, 1000.0, 100.0), (0.0, 1000.0, 5000.0)], 150.0);
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
        let mut autopilot = Autopilot::new(&[(0.0, 1000.0, 1000.0), (0.0, 1000.0, 400.0), (0.0, 1000.0, 5000.0)], 150.0);
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
        let mut autopilot = Autopilot::new(&[(0.0, 1000.0, 3000.0), (0.0, 1000.0, 9000.0)], 150.0);
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
    fn angles_wrap() {
        use std::f32::consts::PI;
        assert!((wrap_angle(1.5 * PI) + 0.5 * PI).abs() < 1e-5);
        assert!((wrap_angle(-1.5 * PI) - 0.5 * PI).abs() < 1e-5);
    }
}
