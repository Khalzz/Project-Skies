use nalgebra::vector;
use rapier3d::{dynamics::{RigidBody}};

use crate::{game::play::plane::{physics::wings::{airfoil::AirFoil, wing::Wing}, plane::PlaneControls}};

pub struct WingManager {
  pub wings: Vec<Wing>,
}

impl WingManager {
  pub fn new() -> Self {
    let naca_2412 = AirFoil::new("assets/aero_data/f16.ron".to_owned());
    let naca_0012 = AirFoil::new("assets/aero_data/f16-elevators.ron".to_owned());

    let wings = vec![
      Wing::new("Left wing".to_string(), vector![5.6, 0.0, 1.4], 16.5, 0.0, naca_2412.clone(), vector![1.0,0.0, 0.0], true, false, 4.0, 500_000.0), // left wing (+4° incidence, includes LEX area)
      Wing::new("Right wing".to_string(), vector![-5.6, 0.0, 1.4], 16.5, 0.0, naca_2412.clone(), vector![1.0, 0.0, 0.0], true, false, 4.0, 500_000.0), // right wing (+4° incidence, includes LEX area)
      Wing::new("Right elevator wing".to_string(), vector![4.2, 0.0, -7.0], 2.70, 0.0, naca_0012.clone(), vector![1.0, 0.0, 0.0], false, false, -1.5, 120_000.0), // right elevator wing (-5° trim to counter the main wings' +4° incidence pitching the nose up at cruise)
      Wing::new("Left elevator wing".to_string(), vector![-4.2, 0.0, -7.0], 2.70, 0.0, naca_0012.clone(), vector![1.0, 0.0, 0.0], false, false, -1.5, 120_000.0), // left elevator wing (-5° trim to counter the main wings' +4° incidence pitching the nose up at cruise)
      Wing::new("Rudder wing".to_string(), vector![0.0, 4.2, -11.2], 1.70, 0.0, naca_0012.clone(), vector![0.0, 1.0, 0.0], false, true, 0.0, 200_000.0) // rudder wing
    ];

    Self { wings }
  }

  pub fn update(&mut self, plane_controls: &PlaneControls, rigidbody: &mut RigidBody) {
    for wing in &mut self.wings {
      wing.control_input = match wing.label.as_str() {
          "Left wing"           => (-plane_controls.aileron + plane_controls.trim_roll).clamp(-1.0, 1.0),
          "Right wing"          => (plane_controls.aileron + plane_controls.trim_roll).clamp(-1.0, 1.0),
          "Left elevator wing"  => (plane_controls.elevator + plane_controls.trim_pitch).clamp(-1.0, 1.0),
          "Right elevator wing" => (plane_controls.elevator + plane_controls.trim_pitch).clamp(-1.0, 1.0),
          "Rudder wing"         => (plane_controls.rudder + plane_controls.trim_yaw).clamp(-1.0, 1.0),
          _ => 0.0,
      };

      wing.physics_force(rigidbody);
    }
  }
 }