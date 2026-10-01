use rapier3d::prelude::RigidBody;
use std::f32::consts::PI;

use super::airfoil::AirFoil;
use super::super::super::aero_spec::{ControlSurfaceSpec, SurfaceKind, WingShape};

pub struct Wing {
    pub label: String,
    pub pressure_center: nalgebra::Vector3<f32>,
    pub wing_area: f32,
    pub chord: f32,
    pub air_foil: AirFoil,
    pub normal: nalgebra::Vector3<f32>,
    pub efficiency_factor: f32,
    pub control_input: f32,
    /// Roll command on this wing, -1..1 - what its hinged Aileron surfaces
    /// deflect by, and what an all-moving surface adds on top of
    /// `control_input` (the F-16's differential "rolling tail"). Set by
    /// `WingManager::update`, from the roll FLCS (or the raw stick).
    pub roll_input: f32,
    pub is_roll_axis: bool,
    pub last_lift_force: nalgebra::Vector3<f32>,
    pub stable: bool,
    pub incidence_angle: f32,
    // How much of `wing_area` is actually the movable control surface, as
    // opposed to fixed wing structure - see physics_force's own comment on
    // where this is used. Equal to `wing_area` for a wing that's entirely a
    // control surface itself (the elevator wings here - the F-16's real
    // horizontal tail is a fully-moving stabilator, so that's physically
    // correct, not a simplification); much smaller than `wing_area` for a
    // wing where the control surface is a hinged flap on a much larger fixed
    // wing (ailerons on the main wings - see wing_manager.rs's own comment
    // on why "Left wing"/"Right wing" needed this distinction and elevators
    // didn't).
    pub control_surface_area: f32,
    /// Hinged surfaces on the trailing edge - see `apply_surface_forces`.
    pub surfaces: Vec<ControlSurfaceSpec>,
    /// The planform, when the wing was given one - surfaces are then laid
    /// out on it (real taper and sweep) instead of on a rectangle around the
    /// pressure center. See `WingSpec::shape`.
    pub shape: Option<WingShape>,
    /// Flap setting, -1..1 of each Flap's travel. TODO: nothing sets this
    /// yet (no flap lever) - always 0.
    pub flap_input: f32,
}

pub const AIR_DENSITY: f32 = 1.225;

/// True angle of attack, degrees: + when the nose is above the airflow
/// (the air hitting the wing from below) - same as FlightData::aoa_y. The
/// airfoil tables are in this convention.
///
/// This used to be measured the other way round (+ with the nose BELOW the
/// airflow), and the lift direction (`lift_direction`) was flipped too - the
/// two cancelled out, so the jet still flew, but every table was read
/// mirrored: its negative side did the lifting, turning the F-16's camber
/// upside down and capping lift at the table's negative-side peak. Flipping
/// either one alone (as once tried on the lift direction) breaks it; both
/// together are the fix.
#[cfg(test)]
fn aoa_deg(local_velocity: &nalgebra::Vector3<f32>) -> f32 {
    aoa_in_plane_deg(local_velocity, &lift_up_axis(&nalgebra::Vector3::x()))
}

/// A surface's "up" in the jet's frame - the side its lift acts toward at
/// a positive angle of attack: nose x span. +Y for a wing spanning +X; for
/// a fin spanning +Y it's -X, so a fin's angle of attack is the sideslip.
fn lift_up_axis(span_axis: &nalgebra::Vector3<f32>) -> nalgebra::Vector3<f32> {
    nalgebra::Vector3::z().cross(span_axis).try_normalize(1e-6).unwrap_or_else(nalgebra::Vector3::y)
}

/// The angle of attack in the surface's own plane (see `lift_up_axis`),
/// degrees - + with the air hitting it from below its "up". For a wing it's
/// exactly `aoa_deg`; for a vertical fin, the sideslip.
fn aoa_in_plane_deg(local_velocity: &nalgebra::Vector3<f32>, up: &nalgebra::Vector3<f32>) -> f32 {
    (-local_velocity.dot(up)).atan2(local_velocity.z).to_degrees()
}

/// Which way lift acts: square to the airflow and to the span, on the
/// wing's upper side - velocity x span (for a wing spanning +X flying toward
/// +Z, that's +Y, up).
fn lift_direction(velocity_dir: &nalgebra::Vector3<f32>, span_axis: &nalgebra::Vector3<f32>) -> nalgebra::Vector3<f32> {
    velocity_dir.cross(span_axis).normalize()
}

/// How much of a hinged surface's deflection turns into an effective change
/// in the wing's angle of attack: thin-airfoil theory's flap effectiveness
/// for a surface `chord_ratio` of the chord deep (Glauert - a quarter-chord
/// surface shifts the wing's angle by ~0.61 of its own deflection, a
/// full-chord one by all of it), times a falloff past 10 deg - the flow
/// starts separating over the hinge, so big deflections do proportionally
/// less (down to 60% of it by 25 deg).
fn flap_effectiveness(chord_ratio: f32, deflection_deg: f32) -> f32 {
    let theta = (2.0 * chord_ratio.clamp(0.0, 1.0) - 1.0).acos();
    let tau = 1.0 - (theta - theta.sin()) / PI;
    let falloff = 1.0 - 0.4 * ((deflection_deg.abs() - 10.0) / 15.0).clamp(0.0, 1.0);
    tau * falloff
}

/// Where along the chord (0 leading edge .. 1 trailing edge) a hinged
/// surface's extra lift acts - thin-airfoil theory (Glauert), for a surface
/// `chord_ratio` of the chord deep: deflecting it changes the lift over the
/// WHOLE chord, not just the strip itself, so the extra lift acts well
/// forward of the hinge - 0.25 for a full-chord surface, toward 0.5 for a
/// tiny one, ~0.43 for a quarter-chord one. With cos(theta) = 2 x chord_ratio
/// - 1: 0.25 + (1/2 sin(theta) (1 - cos(theta))) / (2 (pi - theta + sin(theta))).
fn flap_lift_center(chord_ratio: f32) -> f32 {
    let theta = (2.0 * chord_ratio.clamp(0.0, 1.0) - 1.0).acos();
    let lift_slope = 2.0 * (PI - theta + theta.sin());
    if lift_slope <= 1e-6 {
        return 0.5;
    }
    0.25 + 0.5 * theta.sin() * (1.0 - theta.cos()) / lift_slope
}

impl Wing {
    pub fn new(label: String, pressure_center: nalgebra::Vector3<f32>, wing_area: f32, chord: f32, air_foil: AirFoil, normal: nalgebra::Vector3<f32>, is_roll_axis: bool, stable: bool, incidence_angle: f32, control_surface_area: f32, surfaces: Vec<ControlSurfaceSpec>) -> Self {
        Self {
            label,
            wing_area,
            chord,
            air_foil,
            normal,
            pressure_center,
            efficiency_factor: 1.0,
            control_input: 0.0,
            roll_input: 0.0,
            is_roll_axis,
            last_lift_force: nalgebra::Vector3::zeros(),
            stable,
            incidence_angle,
            control_surface_area,
            surfaces,
            flap_input: 0.0,
            shape: None,
        }
    }

    /// Where a surface's extra lift acts, in the plane's frame: midway along
    /// its stretch of the span, `flap_lift_center` of the chord back there
    /// (not at the hinged strip itself - see that fn). Just the pressure
    /// center when there's no chord to size the wing from.
    pub fn surface_center(&self, surface: &ControlSurfaceSpec) -> nalgebra::Vector3<f32> {
        let lift_center = flap_lift_center(surface.chord_ratio);
        if let Some(shape) = &self.shape {
            let span_fraction = (surface.span_start + surface.span_end) * 0.5;
            return shape.point(&self.normal, span_fraction, lift_center);
        }
        if self.chord <= 0.0 {
            return self.pressure_center;
        }
        let half_span = self.wing_area / self.chord;
        let tip = self.tip_direction();
        let along_span = ((surface.span_start + surface.span_end) * 0.5 - 0.5) * half_span;
        // The pressure center sits at a quarter chord.
        let aft = (lift_center - 0.25) * self.chord;
        self.pressure_center + tip * along_span - nalgebra::Vector3::z() * aft
    }

    /// The span axis, pointing root to tip.
    pub fn tip_direction(&self) -> nalgebra::Vector3<f32> {
        if let Some(shape) = &self.shape {
            return shape.span_axis(&self.normal);
        }
        if self.pressure_center.dot(&self.normal) < 0.0 { -self.normal } else { self.normal }
    }

    /// Each hinged surface's extra lift and drag, applied at its own center.
    /// The whole wing's own lift/drag (at its own angle, surfaces at 0) is
    /// already applied by `physics_force` - this adds only what the surface's
    /// deflection changes: the strip of wing it runs along (its span share
    /// of `wing_area`) flies as if at a higher/lower angle by
    /// `flap_effectiveness` x the deflection. `aileron_input` is -1..1.
    fn apply_surface_forces(&self, rigidbody: &mut RigidBody, aileron_input: f32) {
        let rotation = *rigidbody.rotation();
        for surface in &self.surfaces {
            let input = match surface.kind {
                SurfaceKind::Aileron => aileron_input,
                SurfaceKind::Flap => self.flap_input,
                // The fin's own control input - the pedals (see
                // WingManager::update's "Rudder wing").
                SurfaceKind::Rudder => self.control_input,
            };
            let deflection_deg = input.clamp(-1.0, 1.0) * surface.max_deflection_deg;
            if deflection_deg.abs() < 1e-3 {
                continue;
            }

            // The air as this strip sees it - including the plane's own
            // rotation, so a rolling wing's surfaces feel it too.
            let arm = rotation * self.surface_center(surface);
            let world_velocity = rigidbody.linvel() + rigidbody.angvel().cross(&arm);
            let local_velocity = rotation.inverse() * world_velocity;
            if local_velocity.magnitude() < 0.01 {
                continue;
            }
            // Same angle/sign convention as physics_force (see there): a
            // positive deflection is trailing edge UP - it takes lift away.
            let base_aoa_deg = aoa_in_plane_deg(&local_velocity, &lift_up_axis(&self.normal)) + self.incidence_angle;
            let aoa_shift_deg = -flap_effectiveness(surface.chord_ratio, deflection_deg) * deflection_deg;
            let (base_cl, base_cd) = self.air_foil.sample(base_aoa_deg);
            let (cl, cd) = self.air_foil.sample(base_aoa_deg + aoa_shift_deg);

            // The part of the wing the surface runs along - on a shaped wing
            // its real (tapered) strip, scaled with any wing_area override.
            let span_share = (surface.span_end - surface.span_start).clamp(0.0, 1.0);
            let area = match &self.shape {
                Some(shape) => shape.strip_area(surface.span_start, surface.span_end) * self.wing_area / shape.area(),
                None => self.wing_area * span_share,
            };
            let dynamic_pressure = 0.5 * AIR_DENSITY * local_velocity.magnitude_squared();
            let velocity_dir = world_velocity.normalize();
            let lift_dir = lift_direction(&velocity_dir, &(rotation * self.normal));
            let force = (lift_dir * (cl - base_cl) - velocity_dir * (cd - base_cd)) * dynamic_pressure * area;

            let point = rigidbody.translation() + arm;
            rigidbody.add_force_at_point(force, point.into(), true);
        }
    }

    pub fn physics_force(&mut self, rigidbody: &mut RigidBody) {
        let world_pressure_center = rigidbody.rotation() * self.pressure_center
            + rigidbody.translation();

        let angular_contribution = rigidbody.angvel()
            .cross(&(rigidbody.rotation() * self.pressure_center));
        let world_velocity = rigidbody.linvel() + angular_contribution;
        let local_velocity = rigidbody.rotation().inverse() * world_velocity;

        if local_velocity.magnitude() < 0.01 {
            return;
        }

        let max_deflection = if self.is_roll_axis { 25.0 } else { 25.0 };
        let air_density = 1.225f32;
        let speed_sq = local_velocity.magnitude_squared();
        let dynamic_pressure = 0.5 * air_density * speed_sq;

        // The roll command is used as given - the speed/AoA scheduling that
        // used to scale it down here now lives in the roll FLCS (see
        // physics::roll_flcs), which decides the whole command; with the
        // FLCS off it's the raw stick, the airframe's own full authority.
        let roll_command = self.roll_input;
        let effective_control_input = (self.control_input + roll_command).clamp(-1.0, 1.0);

        let velocity_dir_world = (rigidbody.rotation() * local_velocity).normalize();
        let span_axis_world = rigidbody.rotation() * self.normal;
        let lift_dir = lift_direction(&velocity_dir_world, &span_axis_world);
        let velocity_dir_local = local_velocity.normalize();

        // `stable` is the old fixed side-force model for a fin with no
        // shape; a shaped fin flies as a real wing (the else branch), its
        // angle of attack being the sideslip - see aoa_in_plane_deg.
        let total_force = if self.stable && self.shape.is_none() {
            let sideslip_speed = local_velocity.x; // lateral velocity in local space
        
            let yaw_damping = 1000.0; // tune this
            let deflection_deg = self.control_input * max_deflection;
            let (cl, _cd) = self.air_foil.sample(deflection_deg);
            let control_authority = dynamic_pressure * self.wing_area * cl.abs();
            
            // Damping resists sideslip, control_input steers into it
            let side_force_magnitude = (-sideslip_speed * yaw_damping) + (self.control_input.signum() * control_authority);
            
            let side_axis_world = rigidbody.rotation() * nalgebra::Vector3::x();
            let side_force = side_axis_world * side_force_magnitude;
            
            self.last_lift_force = side_force;
            side_force
        } else {
            // True angle of attack (see aoa_deg), and the control input as
            // a change to it - + input = LESS lift (trailing edge up), the
            // convention the pitch/roll FLCS and every control mapping use.
            let base_aoa_deg = aoa_in_plane_deg(&local_velocity, &lift_up_axis(&self.normal)) + self.incidence_angle;
            let deflected_aoa_deg = base_aoa_deg - effective_control_input * (max_deflection as f32);

            let (base_lift_coefficient, base_drag_coefficient) = self.air_foil.sample(base_aoa_deg);
            let (deflected_lift_coefficient, deflected_drag_coefficient) = self.air_foil.sample(deflected_aoa_deg);

            // Baseline Cl/Cd (no control input) applies over the wing's full
            // area - that part of the wing is there regardless of what the
            // control surface is doing. Only the INCREMENT the control
            // surface's own deflection adds gets scaled down to
            // control_surface_area/wing_area - a wing that's entirely its
            // own control surface (control_surface_area == wing_area, see
            // that field's own doc comment) gets this ratio at 1.0, i.e.
            // identical to before; a wing where the control surface is a
            // small flap on a much bigger fixed wing (ailerons) gets a much
            // smaller ratio, so full aileron deflection can't generate lift
            // as if the entire wing had rotated by max_deflection.
            let control_area_ratio = self.control_surface_area / self.wing_area;
            let lift_coefficient = base_lift_coefficient + (deflected_lift_coefficient - base_lift_coefficient) * control_area_ratio;
            let drag_coefficient = base_drag_coefficient + (deflected_drag_coefficient - base_drag_coefficient) * control_area_ratio;

            let lift_force = lift_dir * (dynamic_pressure * self.wing_area * lift_coefficient);
            let drag_force = rigidbody.rotation() * (-velocity_dir_local * dynamic_pressure * self.wing_area * drag_coefficient);
            let damping_coefficient = 235.0;
            // Pitch/yaw only - the roll axis gets no artificial damping: the
            // wings' own lift already resists rolling (each wing's airflow
            // includes the roll's own motion - see angular_contribution
            // above), and this on top of it was most of why the physics roll
            // came out far slower than the roll-rate curve.
            let roll_axis = rigidbody.rotation() * nalgebra::Vector3::z();
            let angular_vel = rigidbody.angvel() - roll_axis * rigidbody.angvel().dot(&roll_axis);
            let angular_vel = &angular_vel;
            let arm = rigidbody.rotation() * self.pressure_center;
            let damping_velocity = angular_vel.cross(&arm);
            let damping_force = -damping_velocity * damping_coefficient * self.wing_area;

            self.last_lift_force = lift_force;
            lift_force + drag_force + damping_force
        };

        // Parasitic drag: opposes forward airspeed only (local Z), not gravity
        let parasitic_cd = 0.02;
        let forward_drag = -local_velocity.z.signum() * local_velocity.z * local_velocity.z * 0.5 * air_density * self.wing_area * parasitic_cd;
        let parasitic_drag_local = nalgebra::Vector3::new(0.0, 0.0, forward_drag);
        let parasitic_drag = rigidbody.rotation() * parasitic_drag_local;
        let total_force = total_force + parasitic_drag;

        let flat_plate_cd = 1.28;
        let thickness_ratio = 0.04; // 4% thick airfoil
        // Square to the surface - up for a wing, sideways for a fin.
        let wing_normal_world = rigidbody.rotation() * lift_up_axis(&self.normal);
        let normal_speed = world_velocity.dot(&wing_normal_world);
        let normal_drag_magnitude = 0.5 * air_density * normal_speed * normal_speed.abs() * self.wing_area * thickness_ratio * flat_plate_cd;
        let normal_drag = -wing_normal_world * normal_drag_magnitude;
        let total_force = total_force + normal_drag;

        // Lateral (spanwise) drag: air hitting the wing edge during sideslip
        // Frontal area ~ chord * thickness, approximated as wing_area * 0.1
        let lateral_cd = 1.0;
        let lateral_area = self.wing_area * 0.1;
        let span_axis_world = rigidbody.rotation() * self.normal;
        let lateral_speed = world_velocity.dot(&span_axis_world);
        let lateral_drag_magnitude = 0.5 * air_density * lateral_speed * lateral_speed.abs() * lateral_area * lateral_cd;
        let lateral_drag = -span_axis_world * lateral_drag_magnitude;
        let total_force = total_force + lateral_drag;

        rigidbody.add_force_at_point(total_force.into(), world_pressure_center.into(), true);

        self.apply_surface_forces(rigidbody, roll_command);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use nalgebra::Vector3;

    #[test]
    fn nose_above_the_airflow_is_positive_aoa() {
        // Flying along +Z with the nose pitched up 5 deg: the air comes from
        // ahead and below, so the body sees its velocity pointing slightly down.
        let alpha = 5.0f32.to_radians();
        let local_velocity = Vector3::new(0.0, -alpha.sin(), alpha.cos()) * 200.0;
        assert!((aoa_deg(&local_velocity) - 5.0).abs() < 1e-3);
    }

    #[test]
    fn a_hinged_surfaces_lift_acts_forward_of_it() {
        assert!((flap_lift_center(1.0) - 0.25).abs() < 1e-4);
        assert!((flap_lift_center(0.23) - 0.426).abs() < 2e-3, "{}", flap_lift_center(0.23));
        assert!(flap_lift_center(0.01) > 0.48 && flap_lift_center(0.01) <= 0.5);
    }

    #[test]
    fn a_fin_weathervanes() {
        // Sliding toward +X (sideslip), the air hits the fin from that side:
        // a positive angle, and lift pushing the tail back toward -X - which
        // swings the nose into the airflow.
        let fin = Vector3::y();
        let up = lift_up_axis(&fin);
        let sideslip = 10.0f32.to_radians();
        let local_velocity = Vector3::new(sideslip.sin(), 0.0, sideslip.cos()) * 200.0;
        assert!((aoa_in_plane_deg(&local_velocity, &up) - 10.0).abs() < 1e-3);
        let lift = lift_direction(&local_velocity.normalize(), &fin);
        assert!(lift.x < 0.0, "lift {lift:?}");
    }

    #[test]
    fn lift_points_up_for_a_wing_flying_forward() {
        let lift = lift_direction(&Vector3::z(), &Vector3::x());
        assert!((lift - Vector3::y()).norm() < 1e-6);
    }
}
