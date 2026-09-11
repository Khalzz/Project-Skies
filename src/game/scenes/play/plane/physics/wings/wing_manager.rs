use nalgebra::vector;
use rapier3d::{dynamics::{RigidBody}};

use crate::{game::scenes::play::plane::{physics::{pitch_flcs::PitchFlcs, wings::{airfoil::AirFoil, wing::Wing}}, controls::PlaneControls}};

pub struct WingManager {
  pub wings: Vec<Wing>,
  // F-16-style pitch fly-by-wire (g-command). Only consulted when
  // `PlaneControls::fly_by_wire_pitch_autotrim` is set; otherwise it's kept
  // reset so a later engage starts clean. See PitchFlcs' own doc comment.
  pub pitch_flcs: PitchFlcs,
}

impl WingManager {
  pub fn new() -> Self {
    let naca_2412 = AirFoil::new("assets/aero_data/f16.ron".to_owned());
    let naca_0012 = AirFoil::new("assets/aero_data/f16-elevators.ron".to_owned());

    let wings = vec![
      // Sized to real F-16 reference areas (see the conversation this came
      // out of - the old 16.5/2.70 pair was invented/hand-tuned, not
      // grounded in real dimensions, and that mismatch was a real
      // contributor to the "6° AoA at any speed" trim problem: an
      // oversized, over-lifting main wing paired with an undersized tail
      // that couldn't pull the torque balance back down). Real F-16 wing
      // reference area is ~300 sq ft = 27.87 m² total (already includes the
      // LEX/strake, same as before) -> 13.94 m² per side. Real F-16
      // horizontal tail/stabilator area is ~11.84 m² total (~63.7 sq ft per
      // side) -> 5.92 m² per side, up from the old invented 2.70 - this was
      // the more consequential of the two resizes, since the old tail was
      // proportionally about half the real one's share of the main wing
      // (~6:1 vs the real ~2.4:1), leaving it too weak to counter the main
      // wings' pitching moment regardless of trim value.
      // control_surface_area on the main wings kept at the same ~73%
      // fraction of wing_area as before this resize (10.1/13.94 ≈ 0.73), so
      // roll authority isn't incidentally changed by this pass - still
      // expect that to need its own re-measurement against the
      // rolling_rate curve separately from this straight-line-flight fix.
      Wing::new("Left wing".to_string(), vector![5.6, 0.0, 1.4], 13.94, 0.0, naca_2412.clone(), vector![1.0,0.0, 0.0], true, false, 0.0, 500_000.0, 10.1), // left wing (+4° incidence, includes LEX area)
      Wing::new("Right wing".to_string(), vector![-5.6, 0.0, 1.4], 13.94, 0.0, naca_2412.clone(), vector![1.0, 0.0, 0.0], true, false, 0.0, 500_000.0, 10.1), // right wing (+4° incidence, includes LEX area)
      // Elevator wings ARE the control surface, full stop - the F-16's real
      // horizontal tail is an all-moving stabilator, not a flap on a fixed
      // tailplane, so control_surface_area == wing_area here is physically
      // correct.
      Wing::new("Right elevator wing".to_string(), vector![4.2, 0.0, -7.0], 5.92, 0.0, naca_0012.clone(), vector![1.0, 0.0, 0.0], false, false, -1.26, 120_000.0, 5.92), // right elevator wing (-5° trim to counter the main wings' +4° incidence pitching the nose up at cruise)
      Wing::new("Left elevator wing".to_string(), vector![-4.2, 0.0, -7.0], 5.92, 0.0, naca_0012.clone(), vector![1.0, 0.0, 0.0], false, false, -1.26, 120_000.0, 5.92), // left elevator wing (-5° trim to counter the main wings' +4° incidence pitching the nose up at cruise)
      Wing::new("Rudder wing".to_string(), vector![0.0, 4.2, -11.2], 1.70, 0.0, naca_0012.clone(), vector![0.0, 1.0, 0.0], false, true, 0.0, 200_000.0, 1.70) // rudder wing
    ];

    Self { wings, pitch_flcs: PitchFlcs::new() }
  }

  pub fn update(&mut self, plane_controls: &PlaneControls, rigidbody: &mut RigidBody) {
    // Elevator (all-moving stabilator) command for this tick. Two modes:
    //
    //  - fly-by-wire engaged: PitchFlcs runs an F-16-style normal-g command
    //    law. The pilot's `plane_controls.elevator` is a G COMMAND, not an
    //    angle (centred = 1g, full aft = 9g, full fwd = -2g), and the loop
    //    drives the stabilator to hold it - integral term standing in for
    //    trim, so nothing is re-trimmed by hand. See PitchFlcs' doc comment
    //    for the full control law and sign derivation.
    //
    //  - fly-by-wire off: plain `-plane_controls.elevator` (unchanged from
    //    before), with pitch trim folded in upstream by Plane::update.
    //
    // The FLCS is kept reset while disengaged so a later engage starts from
    // a clean integrator rather than a stale one.
    let elevator_command = if plane_controls.fly_by_wire_pitch_autotrim {
      self.pitch_flcs.update(rigidbody, plane_controls).clamp(-1.0, 1.0)
    } else {
      self.pitch_flcs.reset();
      (-plane_controls.elevator).clamp(-1.0, 1.0)
    };

    for wing in &mut self.wings {
      // No separate "+ trim.pitch"/"+ trim.roll" here - Plane::update folds
      // pitch/roll trim into plane_controls.elevator/aileron at the input
      // stage (and, while FBW pitch is engaged, deliberately does NOT fold
      // pitch trim in, since the stick is a g command there).
      wing.control_input = match wing.label.as_str() {
          "Left wing"           => (-plane_controls.aileron).clamp(-1.0, 1.0),
          "Right wing"          => (plane_controls.aileron).clamp(-1.0, 1.0),
          "Left elevator wing" | "Right elevator wing" => elevator_command,
          "Rudder wing"         => (plane_controls.rudder + plane_controls.trim.yaw).clamp(-1.0, 1.0),
          _ => 0.0,
      };

      wing.physics_force(rigidbody);
    }
  }
 }