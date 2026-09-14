use std::collections::HashMap;

use nalgebra::{UnitQuaternion, Vector3};

use crate::engine::physics::physics_handler::{MetadataType, RenderMessage};
use crate::engine::utils::lerps::lerp;

use super::flight_data::FlightData;

/// Everything read back out of the plane for HUD/debug display - refreshed
/// each tick by its own `update` (called from `Plane::apply_physics_feedback`,
/// which also hands the same tick's results to whichever other subsystem
/// owns them - gear meshes to `Airframe`, the fly-by-wire debug override to
/// `controls`), read by the flight HUD (`play::ui`), the F3/F7 debug panels
/// (`render_pass.rs`), and `play::scene::GameLogic` for the aero debug trail.
/// Grouped together since none of it is control logic - it's purely "what
/// does the plane currently show/report," as opposed to `Fcs` (what the
/// plane is doing) or `controls` (what's commanding it).
pub struct Instrumentation {
    pub flight_data: FlightData,
    pub previous_velocity: Option<Vector3<f32>>,
    pub velocity_sample_elapsed: f32,
    pub wing_lift_forces: HashMap<String, Vector3<f32>>,
    pub stall: bool,
}

impl Instrumentation {
    pub fn new() -> Self {
        Self {
            flight_data: FlightData { altimeter: 0.0, speedometer: 0.0, g_meter: 1.0, aoa_x: 0.0, aoa_y: 0.0, aoa: 0.0, roll_rate: 0.0, pitch_rate: 0.0, yaw_rate: 0.0, mach: 0.0 },
            previous_velocity: None,
            velocity_sample_elapsed: 0.0,
            wing_lift_forces: HashMap::new(),
            stall: false,
        }
    }

    /// Refreshes every field here from one tick's physics results. Only
    /// touches its own fields - gear mesh placement and the fly-by-wire
    /// debug override read the same `physics_message` independently, from
    /// `Plane::apply_physics_feedback`, since those belong to `Airframe`/
    /// `controls` respectively, not here.
    pub fn update(&mut self, physics_message: &RenderMessage, delta_time: f32) {
        let speed_ms = physics_message.linvel.magnitude();
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
        self.flight_data.altimeter = physics_message.translation.y;

        // Body-frame velocity - rotation.inverse() undoes the plane's own
        // orientation, same idea as `plane_up` below but for the velocity
        // vector instead of the up axis. Guarded on airspeed since atan2/acos
        // of a near-zero vector is meaningless, jittery noise (e.g. sitting
        // still on the runway) rather than a real angle - just holds the
        // last computed value below that speed instead of flickering.
        let rotation = UnitQuaternion::from_quaternion(physics_message.rotation);
        let body_velocity = rotation.inverse() * physics_message.linvel;
        if body_velocity.magnitude() > 0.5 {
            self.flight_data.aoa_y = (-body_velocity.y).atan2(body_velocity.z).to_degrees();
            self.flight_data.aoa_x = body_velocity.x.atan2(body_velocity.z).to_degrees();
            self.flight_data.aoa = (body_velocity.z / body_velocity.magnitude()).clamp(-1.0, 1.0).acos().to_degrees();
        }

        // Turn rates - see FlightData::roll_rate's own comment for the
        // axis/units convention. No airspeed guard needed here (unlike
        // aoa_* above) - angular velocity is a direct reading, not a ratio.
        let body_angvel = rotation.inverse() * physics_message.angvel;
        self.flight_data.roll_rate = body_angvel.z.to_degrees();
        self.flight_data.pitch_rate = body_angvel.x.to_degrees();
        self.flight_data.yaw_rate = body_angvel.y.to_degrees();

        self.velocity_sample_elapsed += delta_time;
        match &self.previous_velocity {
            Some(previous) if *previous != physics_message.linvel => {
                // Earth gravity - a plain world constant (matches the Rapier
                // gravity `Physics::new` sets up in physics_handler.rs).
                const GRAVITY: Vector3<f32> = Vector3::new(0.0, -9.81, 0.0);
                let acceleration = (physics_message.linvel - previous) / self.velocity_sample_elapsed;
                // Felt acceleration = total acceleration minus gravity (pilot doesn't feel gravity).
                let felt_acceleration = acceleration - GRAVITY;
                let plane_up = rotation * Vector3::y_axis();
                let target_g = felt_acceleration.dot(&plane_up) / 9.81;
                self.flight_data.g_meter = lerp(self.flight_data.g_meter, target_g, delta_time * 10.0);
                self.previous_velocity = Some(physics_message.linvel);
                self.velocity_sample_elapsed = 0.0;
            }
            None => {
                self.previous_velocity = Some(physics_message.linvel);
                self.velocity_sample_elapsed = 0.0;
            }
            _ => {}
        }

        if let Some(MetadataType::Wings(wings)) = physics_message.metadata.get("wings") {
            for wing in wings {
                self.wing_lift_forces.insert(wing.label.clone(), wing.last_lift_force);
            }
        }
    }
}
