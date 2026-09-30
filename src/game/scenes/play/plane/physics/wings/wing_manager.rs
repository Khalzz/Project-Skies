use rapier3d::{dynamics::{RigidBody}};

use std::sync::atomic::Ordering;

use crate::game::scenes::play::plane::{aero_spec::AeroSpec, controls::PlaneControls, physics::{pitch_flcs::PitchFlcs, roll_flcs::{ROLL_FLCS, RollFlcs}, yaw_flcs::{YAW_FLCS, YawFlcs}, wings::{airfoil::AirFoil, wing::Wing}}};

/// The F-16 rolls with its tail too: the two all-moving stabilators move
/// opposite ways alongside the flaperons. This is how much of their travel
/// the roll command gets (0.2 of +-25 deg = +-5 deg), on top of the pitch
/// command.
const STABILATOR_ROLL_SHARE: f32 = 0.2;

pub struct WingManager {
  pub wings: Vec<Wing>,
  // F-16-style pitch fly-by-wire (g-command). Only consulted when
  // `PlaneControls::fly_by_wire_pitch_autotrim` is set; otherwise it's kept
  // reset so a later engage starts clean. See PitchFlcs' own doc comment.
  pub pitch_flcs: PitchFlcs,
  // F-16-style roll-rate command - see RollFlcs' own doc comment. Only
  // consulted while `ROLL_FLCS` is on; kept reset otherwise.
  pub roll_flcs: RollFlcs,
  // F-16-style yaw channel - pedals command sideslip, plus yaw damper and
  // aileron-rudder interconnect. See YawFlcs' own doc comment. Only
  // consulted while `YAW_FLCS` is on; kept reset otherwise.
  pub yaw_flcs: YawFlcs,
}

impl WingManager {
  /// Builds every wing straight from the node's own `AeroSpec` - no
  /// hardcoded default set of wings lives here anymore (see that type's own
  /// doc comment on why). `AirFoil::new` still does its file read here, same
  /// timing as before (main thread, once, at scene start) - only the
  /// geometry/label/area/etc. now comes from spec data instead of a literal.
  pub fn new(spec: &AeroSpec) -> Self {
    Self { wings: Self::build_wings(spec), pitch_flcs: PitchFlcs::new(), roll_flcs: RollFlcs::new(), yaw_flcs: YawFlcs::new() }
  }

  /// Swaps in `spec`'s wings on a running jet (data.ron edited mid-flight -
  /// see messages::AircraftReload). The flight computers keep their state,
  /// so the controls don't jolt as it happens.
  pub fn set_wings(&mut self, spec: &AeroSpec) {
    self.wings = Self::build_wings(spec);
  }

  fn build_wings(spec: &AeroSpec) -> Vec<Wing> {
    spec.wings.iter().map(|w| { let mut wing = Wing::new(
      w.label.clone(),
      w.pressure_center,
      w.wing_area,
      w.chord,
      AirFoil::new(w.airfoil_path.clone()),
      w.normal,
      w.is_roll_axis,
      w.stable,
      w.incidence_angle,
      w.control_surface_area,
      w.surfaces.clone(),
    ); wing.shape = w.shape.clone(); wing }).collect()
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

    // Roll: the FLCS turns the stick into a roll-rate command and works out
    // the surface travel to hold it; with it off, the stick is the surface
    // travel, straight.
    let roll_command = if ROLL_FLCS.load(Ordering::Relaxed) {
      self.roll_flcs.update(rigidbody, plane_controls.aileron, dt)
    } else {
      self.roll_flcs.reset();
      plane_controls.aileron.clamp(-1.0, 1.0)
    };

    // Yaw: the FLCS turns the pedals (trim included) into a sideslip command
    // and works out the rudder to hold it; with it off, the pedals are the
    // rudder, straight.
    let pedals = (plane_controls.rudder + plane_controls.trim.yaw).clamp(-1.0, 1.0);
    let rudder_command = if YAW_FLCS.load(Ordering::Relaxed) {
      self.yaw_flcs.update(rigidbody, pedals, roll_command, dt)
    } else {
      self.yaw_flcs.reset();
      pedals
    };

    for wing in &mut self.wings {
      // No separate "+ trim.pitch"/"+ trim.roll" here - Plane::update folds
      // pitch/roll trim into plane_controls.elevator/aileron at the input
      // stage (and, while FBW pitch is engaged, deliberately does NOT fold
      // pitch trim in, since the stick is a g command there).
      let side_aileron = aileron_for_side(wing.pressure_center.x, roll_command);
      (wing.control_input, wing.roll_input) = match wing.label.as_str() {
          // Fixed wings - rolled by their hinged flaperons (see the spec's
          // `surfaces`), which follow roll_input.
          "Left wing" | "Right wing" => (0.0, side_aileron),
          // All-moving stabilators: pitch command, plus the F-16's
          // differential "rolling tail" share of the roll command.
          "Left elevator wing" | "Right elevator wing" => (elevator_command, STABILATOR_ROLL_SHARE * side_aileron),
          "Rudder wing"         => (rudder_command, 0.0),
          _ => (0.0, 0.0),
      };

      wing.physics_force(rigidbody);
    }
  }
 }

/// The -1..1 roll command a surface gets from the stick's `aileron`, by
/// which side of the plane it's on (`side_x`, its position's X) - opposite
/// on the two sides: +X (the left wing) gets -aileron. By position rather
/// than label, since the stabilators' labels are the other way round from
/// the wings' ("Left elevator wing" sits at -X). Shared with the F7 "Wing
/// Surfaces" view.
pub fn aileron_for_side(side_x: f32, aileron: f32) -> f32 {
  if side_x > 0.0 { (-aileron).clamp(-1.0, 1.0) } else { aileron.clamp(-1.0, 1.0) }
}
