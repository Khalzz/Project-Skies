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

/// An aircraft's configuration, instead of hardcoded inside `WingManager` -
/// this is the ONLY source for these values, there's no parallel hardcoded
/// default anywhere else to fall out of sync with. Read once by
/// `AircraftPhysics::new` (see `play::scene::spawn_world`'s "player" node).
#[derive(Debug, Clone)]
pub struct AircraftSpec {
    pub wings: Vec<WingSpec>,
}

impl AircraftSpec {
    // The F-16's own aero data - the only source for it, see AircraftSpec's
    // own doc comment. play::scene::spawn_world builds the "player" node's
    // AircraftPhysics from this. A second aircraft type would get its own fn
    // here rather than a parameter.
    //
    // Sized to real F-16 reference areas (see the conversation this came out
    // of - the old 16.5/2.70 pair was invented/hand-tuned, not grounded in
    // real dimensions, and that mismatch was a real contributor to the "6°
    // AoA at any speed" trim problem: an oversized, over-lifting main wing
    // paired with an undersized tail that couldn't pull the torque balance
    // back down). Real F-16 wing reference area is ~300 sq ft = 27.87 m²
    // total (already includes the LEX/strake) -> 13.94 m² per side. Real
    // F-16 horizontal tail/stabilator area is ~11.84 m² total (~63.7 sq ft
    // per side) -> 5.92 m² per side. control_surface_area on the main wings
    // kept at the same ~73% fraction of wing_area (10.1/13.94 ≈ 0.73), so
    // roll authority isn't incidentally changed. Elevator wings ARE the
    // control surface, full stop - the F-16's real horizontal tail is an
    // all-moving stabilator, not a flap on a fixed tailplane, so
    // control_surface_area == wing_area there is physically correct.
    pub fn f16() -> AircraftSpec {
        AircraftSpec {
            wings: vec![
                WingSpec { label: "Left wing".to_owned(), pressure_center: Vector3::new(5.6, 0.0, 1.4), wing_area: 13.94, chord: 0.0, airfoil_path: "assets/aero_data/f16.ron".to_owned(), normal: Vector3::new(1.0, 0.0, 0.0), is_roll_axis: true, stable: false, incidence_angle: 0.0, max_force: 500_000.0, control_surface_area: 10.1 },
                WingSpec { label: "Right wing".to_owned(), pressure_center: Vector3::new(-5.6, 0.0, 1.4), wing_area: 13.94, chord: 0.0, airfoil_path: "assets/aero_data/f16.ron".to_owned(), normal: Vector3::new(1.0, 0.0, 0.0), is_roll_axis: true, stable: false, incidence_angle: 0.0, max_force: 500_000.0, control_surface_area: 10.1 },
                // -5° trim to counter the main wings' +4° incidence pitching the nose up at cruise.
                WingSpec { label: "Right elevator wing".to_owned(), pressure_center: Vector3::new(4.2, 0.0, -7.0), wing_area: 5.92, chord: 0.0, airfoil_path: "assets/aero_data/f16-elevators.ron".to_owned(), normal: Vector3::new(1.0, 0.0, 0.0), is_roll_axis: false, stable: false, incidence_angle: -1.26, max_force: 120_000.0, control_surface_area: 5.92 },
                WingSpec { label: "Left elevator wing".to_owned(), pressure_center: Vector3::new(-4.2, 0.0, -7.0), wing_area: 5.92, chord: 0.0, airfoil_path: "assets/aero_data/f16-elevators.ron".to_owned(), normal: Vector3::new(1.0, 0.0, 0.0), is_roll_axis: false, stable: false, incidence_angle: -1.26, max_force: 120_000.0, control_surface_area: 5.92 },
                WingSpec { label: "Rudder wing".to_owned(), pressure_center: Vector3::new(0.0, 4.2, -11.2), wing_area: 1.70, chord: 0.0, airfoil_path: "assets/aero_data/f16-elevators.ron".to_owned(), normal: Vector3::new(0.0, 1.0, 0.0), is_roll_axis: false, stable: true, incidence_angle: 0.0, max_force: 200_000.0, control_surface_area: 1.70 },
            ],
        }
    }
}
