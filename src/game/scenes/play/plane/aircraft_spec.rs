use nalgebra::Vector3;

/// One wing's own geometry/aero definition - a 1:1 data mirror of
/// `Wing::new`'s own params, minus the already-loaded `AirFoil` (resolved
/// from `airfoil_path` when `WingManager::new` builds the real `Wing`s, same
/// timing `AirFoil::new` already ran at before this - still main-thread,
/// still once, at scene start, nothing about when this file gets read
/// changes).
#[derive(Debug, Clone)]
pub struct WingSpec {
    pub label: String,
    pub pressure_center: Vector3<f32>,
    pub wing_area: f32,
    pub chord: f32,
    pub airfoil_path: String,
    pub normal: Vector3<f32>,
    pub is_roll_axis: bool,
    pub stable: bool,
    pub incidence_angle: f32,
    pub max_force: f32,
    pub control_surface_area: f32,
}

/// A node's own aircraft configuration - authored directly on the node (see
/// `play::scene::spawn_world`'s "player" node) instead of hardcoded inside
/// `WingManager`. This is the ONLY source for these values now - there's no
/// parallel hardcoded default anywhere else to fall out of sync with.
/// Collected at spawn time into `SceneContent::aircraft_specs` (mirroring
/// `physics_bodies`), read once by `PlanePhysicsLogic::new` to build this
/// node's own `AircraftUnit`.
#[derive(Debug, Clone)]
pub struct AircraftSpec {
    pub wings: Vec<WingSpec>,
}
