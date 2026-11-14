/*
    Airframe references all sections related to the "physics of the plane" mostly the aerodynamic ones
    for example the wings contain:

    - Wing: the main surface thats defined as a "airfoil".
    - Aileron: the main surface defined for controling the deviation of lift through the surface.
    - Flaps: the surface dedicated to generate more lift for take off, landing or tighter turns.
*/

// Landing gear works through the definition of his parameters

/*
pub struct Wing {
    pub label: String,
    pub pressure_center: nalgebra::Vector3<f32>,
    pub wing_area: f32,
    pub chord: f32,
    pub air_foil: AirFoil,
    pub normal: nalgebra::Vector3<f32>,
    pub efficiency_factor: f32,
    pub control_input: f32,
    pub roll_input: f32,
    pub is_roll_axis: bool,
    pub last_lift_force: nalgebra::Vector3<f32>,
    pub stable: bool,
    pub incidence_angle: f32,
    pub control_surface_area: f32,
    pub surfaces: Vec<ControlSurfaceSpec>,
    pub shape: Option<WingShape>,
    pub flap_input: f32,
    pub hp: f32,
    pub max_hp: f32,
}
*/

pub struct Airframe {
    //landing_gear: LandingGear,
}

impl Airframe {
    pub fn new() -> Airframe {
        Airframe {}
    }
}
