use std::collections::VecDeque;

use nalgebra::{Point3, UnitQuaternion, Vector3};
use rapier3d::prelude::RigidBody;

use super::flight_data::{FlightData, IMPACT_WINDOWS_SECONDS};

/// Computes everything the plane reports for HUD/debug display - lives on
/// the physics thread (inside `AircraftPhysics`), refreshed once per fixed
/// step straight from the rigidbody, and published back to the main thread
/// as `AircraftState::flight_data`. Grouped together since none of it is
/// control logic - it's purely "what does the plane currently show/report,"
/// as opposed to `Fcs` (what the plane is doing) or `PlaneControls` (what's
/// commanding it).
pub struct Instrumentation {
    pub flight_data: FlightData,
    previous_velocity: Option<Vector3<f32>>,
    /// The longest of IMPACT_WINDOWS_SECONDS of velocities, newest first -
    /// see `FlightData::impact_g`.
    velocity_history: VecDeque<Vector3<f32>>,
    /// See `AeroSpec::pilot_position`.
    pilot_position: Point3<f32>,
    previous_angvel: Option<Vector3<f32>>,
    /// Angular acceleration, eased over PILOT_ANGULAR_ACCEL_SMOOTHING - see
    /// where it's used in `update`.
    angular_acceleration: Vector3<f32>,
    /// Last step's body-frame velocity - see `update_g_meter`.
    previous_body_velocity: Option<Vector3<f32>>,
}

/// The G meter's x/z readings (see `update_g_meter`) are eased over this
/// long (s) - enough to stop them flickering step to step, short enough
/// that a real change still shows straight away.
const G_METER_SMOOTHING: f32 = 0.05;

/// Earth gravity - a plain world constant (matches the Rapier gravity
/// `Physics::new` sets up in physics_handler.rs).
const GRAVITY: Vector3<f32> = Vector3::new(0.0, -9.81, 0.0);

/// While rolling, the G meter's side reading gets a steady push against the
/// roll - this many G per 100 deg/s of roll rate, for as long as the roll
/// lasts (rolling right pushes left). See `update_g_meter`.
const ROLL_PUSH_G_PER_100_DEG_S: f32 = 0.25;

/// Roll is set straight to the commanded rate in one step (see
/// `AircraftPhysics::apply_kinematic_roll`), so its angular acceleration is a
/// one-step spike. Eased over this long (s) it becomes a short, readable
/// jolt instead - same total kick, just spread out.
const PILOT_ANGULAR_ACCEL_SMOOTHING: f32 = 0.1;

/// How much of the seat's swing around the center of mass counts, per
/// rotation (1.0 = all of it, the real physics). Rolling swings the seat
/// sideways and pitching swings it up/down - the seat sits far ahead of the
/// center of mass, so pitch inputs read as jolts - both turned down for the
/// feel. Yaw's swing is left as is.
const SEAT_ROLL_SWING: f32 = 0.1;
const SEAT_PITCH_SWING: f32 = 0.3;

impl Instrumentation {
    /// Where the pilot sits, in the plane's frame.
    pub fn pilot_position(&self) -> Vector3<f32> {
        self.pilot_position.coords
    }

    /// Moves the pilot's seat (data.ron edited mid-flight).
    pub fn set_pilot_position(&mut self, pilot_position: Vector3<f32>) {
        self.pilot_position = Point3::from(pilot_position);
    }

    pub fn new(pilot_position: Vector3<f32>) -> Self {
        Self {
            flight_data: FlightData::new(),
            previous_velocity: None,
            velocity_history: VecDeque::new(),
            pilot_position: Point3::from(pilot_position),
            previous_angvel: None,
            angular_acceleration: Vector3::zeros(),
            previous_body_velocity: None,
        }
    }

    /// Car-style G meter, forward/back (z) and side (x) only - the seat-axis
    /// (vertical) G is `body_g`, worked out separately in `update`.
    /// - forward/back: how fast the SPEED is changing. Only the speed - a
    ///   pull or turn at constant speed reads zero here, same as a car's
    ///   meter going round a corner at constant speed.
    /// - side: how fast the flight path is curving sideways, in the plane's
    ///   own axes - the sideslip speed changing, plus yaw rate x forward
    ///   speed (a car's lateral G: speed x how fast it's turning), minus roll
    ///   rate x vertical speed (rolling with the air coming from below/above
    ///   the nose swings that flow sideways) - with gravity's share taken out
    ///   (`body_gravity`): gravity bends the flight path downward, and as the
    ///   plane rolls, "downward" swings across its side axis, left, then
    ///   right, once per turn - a steady roll would read as a push rocking
    ///   side to side. Without it the reading only changes with what the
    ///   plane itself does. Then a steady push against the roll itself is
    ///   added (ROLL_PUSH_G_PER_100_DEG_S).
    /// `body_velocity`/`body_angvel`/`body_gravity` are this step's, in body
    /// axes (+X left wing, +Y up, +Z nose).
    fn update_g_meter(&mut self, body_velocity: Vector3<f32>, body_angvel: Vector3<f32>, body_gravity: Vector3<f32>, dt: f32) {
        let Some(previous) = self.previous_body_velocity.replace(body_velocity) else {
            return;
        };

        let speed_rate = (body_velocity.magnitude() - previous.magnitude()) / dt;
        let sideslip_rate = (body_velocity.x - previous.x) / dt;
        let path_curving = sideslip_rate + body_angvel.y * body_velocity.z - body_angvel.z * body_velocity.y;
        // Rolling right is a positive roll rate (the +X/left wing rising);
        // the push is to the left, so the acceleration it reads as is to the
        // right - negative.
        let roll_push = -body_angvel.z.to_degrees() / 100.0 * ROLL_PUSH_G_PER_100_DEG_S * 9.81;
        let lateral = path_curving - body_gravity.x + roll_push;

        let ease = 1.0 - (-dt / G_METER_SMOOTHING).exp();
        self.flight_data.longitudinal_g += (speed_rate / 9.81 - self.flight_data.longitudinal_g) * ease;
        self.flight_data.lateral_g += (lateral / 9.81 - self.flight_data.lateral_g) * ease;
    }

    /// Refreshes `flight_data` from this step's rigidbody state. `dt` is the
    /// fixed physics step - the velocity delta since the previous call is
    /// exactly one step's worth, so it's also what the g-meter divides by.
    pub fn update(&mut self, rigidbody: &RigidBody, dt: f32) {
        let linvel = *rigidbody.linvel();
        let rotation: UnitQuaternion<f32> = *rigidbody.rotation();

        let speed_ms = linvel.magnitude();
        self.flight_data.velocity = linvel;
        self.flight_data.speedometer = speed_ms * 1.94384;
        // See FlightData::mach's own doc comment for the sea-level-only
        // caveat on this constant.
        const SPEED_OF_SOUND_SEA_LEVEL_MS: f32 = 340.29;
        self.flight_data.mach = speed_ms / SPEED_OF_SOUND_SEA_LEVEL_MS;
        // Raw world Y - this game's own "sea level" is Y=0 (see e.g.
        // play::scene::spawn_world's "world"/water node), so this doubles as
        // height above water without needing the wave-surface's own current
        // height subtracted out; a HUD altimeter reads the nominal/rest sea
        // level, not the instantaneous wave crest/trough under the plane.
        self.flight_data.altimeter = rigidbody.translation().y;

        // Body-frame velocity - rotation.inverse() undoes the plane's own
        // orientation, same idea as `plane_up` below but for the velocity
        // vector instead of the up axis. Guarded on airspeed since atan2/acos
        // of a near-zero vector is meaningless, jittery noise (e.g. sitting
        // still on the runway) rather than a real angle - just holds the
        // last computed value below that speed instead of flickering.
        let body_velocity = rotation.inverse() * linvel;
        if body_velocity.magnitude() > 0.5 {
            self.flight_data.aoa_y = (-body_velocity.y).atan2(body_velocity.z).to_degrees();
            self.flight_data.aoa_x = body_velocity.x.atan2(body_velocity.z).to_degrees();
            self.flight_data.aoa = (body_velocity.z / body_velocity.magnitude()).clamp(-1.0, 1.0).acos().to_degrees();
        }

        // Turn rates - see FlightData::roll_rate's own comment for the
        // axis/units convention. No airspeed guard needed here (unlike
        // aoa_* above) - angular velocity is a direct reading, not a ratio.
        let body_angvel = rotation.inverse() * rigidbody.angvel();
        self.flight_data.roll_rate = body_angvel.z.to_degrees();
        self.flight_data.pitch_rate = body_angvel.x.to_degrees();
        self.flight_data.yaw_rate = body_angvel.y.to_degrees();
        self.flight_data.up_alignment = (rotation * Vector3::y()).y;

        self.update_g_meter(body_velocity, body_angvel, rotation.inverse() * GRAVITY, dt);

        if let Some(previous) = self.previous_velocity {
            // Earth gravity - a plain world constant (matches the Rapier
            // gravity `Physics::new` sets up in physics_handler.rs).
            let acceleration = (linvel - previous) / dt;
            // Felt acceleration = total acceleration minus gravity (pilot doesn't feel gravity).
            let felt_acceleration = acceleration - GRAVITY;
            let plane_up = rotation * Vector3::y_axis();
            // Raw, this step's own value - no smoothing, so short spikes
            // (impacts) show at full strength. Whatever needs a steadier
            // number filters it itself (e.g. the pilot's G tolerance
            // builds up over time).
            self.flight_data.g_meter = felt_acceleration.dot(&plane_up) / 9.81;

            // At the pilot's seat: the same, plus the plane turning around
            // them - they sit away from the center of mass, so a roll swings
            // them sideways as it starts/stops (angular acceleration) and
            // pushes them away from the axis while it lasts (centripetal).
            let angvel = *rigidbody.angvel();
            if let Some(previous_angvel) = self.previous_angvel {
                let ease = 1.0 - (-dt / PILOT_ANGULAR_ACCEL_SMOOTHING).exp();
                self.angular_acceleration += ((angvel - previous_angvel) / dt - self.angular_acceleration) * ease;
            }
            let arm = rigidbody.position() * self.pilot_position - rigidbody.center_of_mass();
            // The swing from a rotation `w` with angular acceleration `a`.
            let swing = |a: Vector3<f32>, w: Vector3<f32>| a.cross(&arm) + w.cross(&w.cross(&arm));
            // Just the roll (body Z) or pitch (body X) part of a world vector.
            let about = |v: Vector3<f32>, axis: Vector3<f32>| {
                let axis = rotation * axis;
                axis * v.dot(&axis)
            };
            let full_swing = swing(self.angular_acceleration, angvel);
            let roll_swing = swing(about(self.angular_acceleration, Vector3::z()), about(angvel, Vector3::z()));
            let pitch_swing = swing(about(self.angular_acceleration, Vector3::x()), about(angvel, Vector3::x()));
            let seat_swing = full_swing - roll_swing * (1.0 - SEAT_ROLL_SWING) - pitch_swing * (1.0 - SEAT_PITCH_SWING);
            let seat_acceleration = acceleration + seat_swing;
            self.flight_data.body_g = rotation.inverse() * (seat_acceleration - GRAVITY) / 9.81;
        }
        self.previous_velocity = Some(linvel);
        self.previous_angvel = Some(*rigidbody.angvel());

        // Impact G: the average acceleration across each window, all axes.
        let steps = |seconds: f32| ((seconds / dt).round() as usize).max(1);
        let longest = steps(IMPACT_WINDOWS_SECONDS[IMPACT_WINDOWS_SECONDS.len() - 1]);
        self.velocity_history.push_front(linvel);
        self.velocity_history.truncate(longest + 1);
        for (window, seconds) in IMPACT_WINDOWS_SECONDS.iter().enumerate() {
            // Until the history fills up, the window is whatever it has.
            let back = steps(*seconds).min(self.velocity_history.len() - 1);
            if back == 0 {
                continue;
            }
            let acceleration = (linvel - self.velocity_history[back]) / (back as f32 * dt);
            self.flight_data.impact_g[window] = (acceleration - Vector3::new(0.0, -9.81, 0.0)).magnitude() / 9.81;
        }
    }
}
