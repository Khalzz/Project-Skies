//! An aircraft - everything that makes one fly, and fly itself:
//! - `plane`: the `Plane` behavior on its node - the pilot's controls, its
//!   systems, its effects, crashing;
//! - `aircraft_physics` and `physics`: its physics half, on the physics
//!   thread - the wings, the wheels, the flight control laws;
//! - `aircraft_spec` (and `aero_spec`, `gear_spec`, `pylon_spec`): its
//!   data.ron;
//! - `ai`: a bot's flying - its autopilot, its pilot's skill, its defense
//!   against missiles;
//! - `build`: its node, as a map places it (`aircraft`).

mod build;
pub use build::*;

pub mod aircraft_physics;
pub mod messages;
pub mod physics;
pub mod plane;
pub mod transponder;
// D-pad layout of the quick menu - its HUD/navigation is unused while the
// list version (quick_list) is being tried out; its actions/status types are
// shared with it.
pub mod aero_spec;
pub mod afterburner;
pub mod ai;
pub mod aircraft_spec;
pub mod airframe;
pub mod control_surfaces;
pub mod controls;
pub mod effects;
pub mod engine;
pub mod fcs;
pub mod flight_data;
pub mod flight_system;
#[cfg(test)]
mod flight_tests;
pub mod fuel;
pub mod gear_spec;
pub mod instrumentation;
pub mod landing_gear;
pub mod pilot;
pub mod pylon_spec;
pub mod quick_list;
#[allow(dead_code)]
pub mod quick_menu;
pub mod speed_brake;
pub mod utils;
