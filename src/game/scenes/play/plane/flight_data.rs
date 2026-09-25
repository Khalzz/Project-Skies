use nalgebra::Vector3;

/// Everything the flight HUD reads back out of the plane every frame -
/// computed on the physics thread by `Instrumentation`, read on the main
/// thread from `AircraftState::flight_data`.
#[derive(Clone)]
pub struct FlightData {
    // World-space velocity (m/s), straight off the rigidbody - what the HUD's
    // velocity-vector marker points along.
    pub velocity: Vector3<f32>,
    pub altimeter: f32,
    pub speedometer: f32,
    pub g_meter: f32,
    // True airspeed (rigidbody linvel, m/s) divided by a fixed sea-
    // level speed of sound (340.29 m/s, standard ISA value at 15°C) - not
    // altitude-corrected, same sea-level-only assumption the rest of this
    // aerodynamics model already makes (see e.g. Wing::physics_force's own
    // fixed air_density constant).
    pub mach: f32,
    // Angle of attack, decomposed into the plane's own body axes (see
    // Instrumentation::update for the exact rotation.inverse() * linvel
    // derivation) - forward is local +Z, up is local +Y, right is local +X
    // (same convention `plane_up`/water_splash's own "forward" already use).
    // aoa_y is the classic vertical AoA (relative wind above/below the nose,
    // positive = nose pitched above the flight path); aoa_x is the
    // horizontal/sideslip equivalent (relative wind left/right of the nose).
    // aoa is the resultant total angle between the nose and the velocity
    // vector regardless of direction (always >= 0), not just aoa_x/aoa_y
    // added together.
    pub aoa_x: f32,
    pub aoa_y: f32,
    pub aoa: f32,
    // Turn rates (deg/s), same body-axis derivation and convention as
    // aoa_x/aoa_y above (rotation.inverse() * angvel instead of * linvel) -
    // roll_rate is rotation about the forward axis (elevator has no effect
    // on this), pitch_rate about the right/left axis (elevator's own axis),
    // yaw_rate about the up axis (rudder's own axis). Unlike aoa_*, these
    // aren't gated on airspeed - angular velocity is a direct physical
    // reading, not a ratio that blows up near zero the way atan2/acos of a
    // near-zero vector does.
    pub roll_rate: f32,
    pub pitch_rate: f32,
    pub yaw_rate: f32,
    // Nothing sets this yet - always false, same as before it moved here.
    pub stall: bool,
}

impl FlightData {
    pub fn new() -> Self {
        Self { velocity: Vector3::zeros(), altimeter: 0.0, speedometer: 0.0, g_meter: 1.0, mach: 0.0, aoa_x: 0.0, aoa_y: 0.0, aoa: 0.0, roll_rate: 0.0, pitch_rate: 0.0, yaw_rate: 0.0, stall: false }
    }
}
