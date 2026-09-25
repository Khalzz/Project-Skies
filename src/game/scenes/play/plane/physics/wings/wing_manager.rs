use rapier3d::{dynamics::{RigidBody}};

use crate::{game::scenes::play::plane::{aircraft_spec::AircraftSpec, physics::{pitch_flcs::PitchFlcs, wings::{airfoil::AirFoil, wing::Wing}}, controls::PlaneControls}};

pub struct WingManager {
  pub wings: Vec<Wing>,
  // F-16-style pitch fly-by-wire (g-command). Only consulted when
  // `PlaneControls::fly_by_wire_pitch_autotrim` is set; otherwise it's kept
  // reset so a later engage starts clean. See PitchFlcs' own doc comment.
  pub pitch_flcs: PitchFlcs,
}

impl WingManager {
  /// Builds every wing straight from the node's own `AircraftSpec` - no
  /// hardcoded default set of wings lives here anymore (see that type's own
  /// doc comment on why). `AirFoil::new` still does its file read here, same
  /// timing as before (main thread, once, at scene start) - only the
  /// geometry/label/area/etc. now comes from spec data instead of a literal.
  pub fn new(spec: &AircraftSpec) -> Self {
    let wings = spec.wings.iter().map(|w| Wing::new(
      w.label.clone(),
      w.pressure_center,
      w.wing_area,
      w.chord,
      AirFoil::new(w.airfoil_path.clone()),
      w.normal,
      w.is_roll_axis,
      w.stable,
      w.incidence_angle,
      w.max_force,
      w.control_surface_area,
    )).collect();

    Self { wings, pitch_flcs: PitchFlcs::new() }
  }

  pub fn update(&mut self, plane_controls: &PlaneControls, rigidbody: &mut RigidBody, dt: f32) {
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
      self.pitch_flcs.update(rigidbody, plane_controls, dt).clamp(-1.0, 1.0)
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