pub mod physics;
pub mod plane;
pub mod aircraft_physics;
pub mod messages;
// D-pad layout of the quick menu - its HUD/navigation is unused while the
// list version (quick_list) is being tried out; its actions/status types are
// shared with it.
#[allow(dead_code)]
pub mod quick_menu;
pub mod quick_list;
pub mod pilot;
pub mod effects;
pub mod flight_system;
pub mod engine;
pub mod utils;
pub mod controls;
pub mod control_surfaces;
pub mod landing_gear;
pub mod afterburner;
pub mod flight_data;
pub mod instrumentation;
pub mod fcs;
pub mod airframe;
pub mod aircraft_spec;
pub mod aero_spec;
pub mod autopilot;
#[cfg(test)]
mod flight_tests;
pub mod gear_spec;
