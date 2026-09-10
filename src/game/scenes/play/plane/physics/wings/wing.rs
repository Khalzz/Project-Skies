use rapier3d::prelude::RigidBody;
use std::f32::consts::PI;
use nalgebra::vector;

use crate::{engine::physics::physics::DebugPhysicsMessageType, engine::primitive::manual_vertex::ManualVertex};

use super::airfoil::AirFoil;
use super::super::rolling_rate::{max_roll_rate_deg_s, RollRateParams};

pub struct Wing {
    pub label: String,
    pub pressure_center: nalgebra::Vector3<f32>,
    pub wing_area: f32,
    pub chord: f32,
    pub air_foil: AirFoil,
    pub normal: nalgebra::Vector3<f32>,
    pub efficiency_factor: f32,
    pub control_input: f32,
    pub is_roll_axis: bool,
    pub last_lift_force: nalgebra::Vector3<f32>,
    pub stable: bool,
    pub incidence_angle: f32,
    pub max_force: f32,
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
}

impl Wing {
    pub fn new(label: String, pressure_center: nalgebra::Vector3<f32>, wing_area: f32, chord: f32, air_foil: AirFoil, normal: nalgebra::Vector3<f32>, is_roll_axis: bool, stable: bool, incidence_angle: f32, max_force: f32, control_surface_area: f32) -> Self {
        Self {
            label,
            wing_area,
            chord,
            air_foil,
            normal,
            pressure_center,
            efficiency_factor: 1.0,
            control_input: 0.0,
            is_roll_axis,
            last_lift_force: nalgebra::Vector3::zeros(),
            stable,
            incidence_angle,
            max_force,
            control_surface_area,
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

        let forward_speed = local_velocity.z;
        let vertical_speed = local_velocity.y;

        let max_deflection = if self.is_roll_axis { 25.0 } else { 25.0 };
        let air_density = 1.225f32;
        let speed_sq = local_velocity.magnitude_squared();
        let dynamic_pressure = 0.5 * air_density * speed_sq;

        // Derived from rolling_rate::max_roll_rate_deg_s rather than a
        // separate roll_authority_gain fn (removed) - dividing back out by
        // params.base_max_roll_rate_deg_s recovers the same 0..1
        // speed_taper*aoa_taper product that fn used to return directly.
        // NOTE this means base_max_roll_rate_deg_s itself cancels out of
        // this ratio - it still has ZERO effect on the actual force
        // simulation below, only on the chart's own displayed curve (see
        // the conversation this came out of) - this fixes the compile
        // error/keeps behavior identical to before, it does NOT make that
        // constant drive real gameplay.
        //
        // Only ailerons (is_roll_axis) get this; elevator/rudder still get
        // full commanded deflection regardless of speed/AoA.
        // true_airspeed_ms is this wing's own local airspeed (not the
        // aircraft's raw linvel - already includes this wing's own
        // rotational contribution, same value dynamic_pressure above is
        // built from); altitude_m is rigidbody.translation().y directly -
        // this game's "sea level" is Y=0 and 1 world unit = 1 meter (see
        // e.g. Plane::FlightData::altimeter's own comment), so that's
        // already altitude in the units equivalent_airspeed expects, no
        // conversion needed. aoa_deg is this wing's own local flow angle,
        // same stand-in for aircraft AoA as before, taken as absolute value
        // since aoa_taper only treats positive/nose-up AoA as authority-
        // reducing on its own.
        let roll_gain = if self.is_roll_axis {
            let true_airspeed_ms = local_velocity.magnitude();
            let altitude_m = rigidbody.translation().y;
            let raw_wing_aoa_deg = vertical_speed.atan2(forward_speed).to_degrees().abs();
            let params = RollRateParams::default();
            max_roll_rate_deg_s(true_airspeed_ms, altitude_m, raw_wing_aoa_deg, &params) / params.base_max_roll_rate_deg_s
        } else {
            1.0
        };
        let effective_control_input = self.control_input * roll_gain;

        let velocity_dir_world = (rigidbody.rotation() * local_velocity).normalize();
        let span_axis_world = rigidbody.rotation() * self.normal;
        // Reverted - swapping this cross product's operand order (tried per
        // the logged cl/lift.y sign mismatch - see the conversation this
        // came out of) broke actual flight behavior, so whatever that sign
        // mismatch's real explanation is, it isn't this. Left at the
        // original order pending more investigation.
        let lift_dir = span_axis_world.cross(&velocity_dir_world).normalize();
        let velocity_dir_local = local_velocity.normalize();

        let total_force = if self.stable {
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
            let base_aoa_deg = vertical_speed.atan2(forward_speed).to_degrees() + self.incidence_angle;
            let deflected_aoa_deg = base_aoa_deg + effective_control_input * (max_deflection as f32);

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
            let angular_vel = rigidbody.angvel();
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
        let wing_normal_world = rigidbody.rotation() * nalgebra::Vector3::y();
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

        let mag = total_force.magnitude();
        let clamped = if mag > self.max_force { total_force * (self.max_force / mag) } else { total_force };

        rigidbody.add_force_at_point(clamped.into(), world_pressure_center.into(), true);
    }
}