use nalgebra::{UnitQuaternion, Vector3};
use rapier3d::prelude::RigidBody;

use crate::engine::utils::lerps::lerp;

use super::flight_data::FlightData;

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
}

impl Instrumentation {
    pub fn new() -> Self {
        Self {
            flight_data: FlightData::new(),
            previous_velocity: None,
        }
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

        if let Some(previous) = self.previous_velocity {
            // Earth gravity - a plain world constant (matches the Rapier
            // gravity `Physics::new` sets up in physics_handler.rs).
            const GRAVITY: Vector3<f32> = Vector3::new(0.0, -9.81, 0.0);
            let acceleration = (linvel - previous) / dt;
            // Felt acceleration = total acceleration minus gravity (pilot doesn't feel gravity).
            let felt_acceleration = acceleration - GRAVITY;
            let plane_up = rotation * Vector3::y_axis();
            let target_g = felt_acceleration.dot(&plane_up) / 9.81;
            self.flight_data.g_meter = lerp(self.flight_data.g_meter, target_g, dt * 10.0);
        }
        self.previous_velocity = Some(linvel);
    }
}
