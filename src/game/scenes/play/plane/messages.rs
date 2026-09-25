//! The contract between an aircraft's two halves - the ONLY data that
//! crosses between `Plane` (main thread) and `AircraftPhysics` (physics
//! thread). Anything not in here stays private to whichever half owns it.
//!
//! main → physics:
//! - `PlaneControls` (latest value, see controls.rs) - `node.set_physics_input`
//! - `AircraftEvent` (one-shot)                      - `node.push_physics_event`
//!
//! physics → main:
//! - `AircraftState` - published once per frame, read with
//!   `node.physics_state::<AircraftState>()`.

use std::collections::HashMap;

use nalgebra::Vector3;

use super::flight_data::FlightData;
use super::physics::wheels::wheel::WheelData;

/// One-shot pilot commands - queued, so each is seen by exactly one physics
/// step no matter how many steps run per frame.
#[derive(Clone, Copy, Debug)]
pub enum AircraftEvent {
    ToggleGear,
    GearUp,
    GearDown,
}

/// Everything the physics half reports back each frame.
#[derive(Clone)]
pub struct AircraftState {
    /// HUD/debug readouts - speed, altitude, g, AoA, turn rates...
    pub flight_data: FlightData,
    /// Landing gear extension, 0 = stowed, 1 = down and locked.
    pub gear_deploy: f32,
    /// Each wheel's suspension contact point, keyed by wheel mesh name.
    pub wheels: HashMap<String, WheelData>,
    /// What the simulated elevator wing is actually doing (after any
    /// fly-by-wire solve), in the same convention as `-controls.elevator` -
    /// the elevator meshes animate off this instead of raw stick.
    pub elevator_control_input: f32,
    /// Each wing's last lift force, keyed by wing label (F7 overlay).
    pub wing_lift_forces: HashMap<String, Vector3<f32>>,
}
