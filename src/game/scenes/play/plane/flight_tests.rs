//! Flight tests - the real F-16 from its data.ron, flown by its real
//! `AircraftPhysics` in a rapier world stepped exactly like the physics
//! thread does (see Physics::physics_thread), with no level around it - so
//! how the jet answers its stick can be measured instead of guessed.
//!
//! `cargo test --bin pankarta-software flight_tests -- --nocapture` prints
//! each test's g trace.

use nalgebra::{UnitQuaternion, Vector3};
use rapier3d::prelude::*;

use crate::engine::physics::physics_behavior::{PhysicsBehavior, PhysicsInput, PhysicsNode};
use crate::engine::physics::physics_resources::{load_physics_from_definitions, PhysicsObjectDef};
use crate::game::scenes::play::plane::aircraft_physics::AircraftPhysics;
use crate::game::scenes::play::plane::aircraft_spec::AircraftSpec;
use crate::game::scenes::play::plane::controls::PlaneControls;
use crate::game::scenes::play::plane::messages::{AircraftEvent, AircraftState};

/// The physics thread's step.
const STEP: f32 = 1.0 / 120.0;
/// Mach 1 at sea level (m/s) - the same fixed speed of sound the jet's
/// instruments use.
const MACH_1: f32 = 340.29;

/// One jet alone in the sky.
struct Sim {
    bodies: RigidBodySet,
    colliders: ColliderSet,
    node: PhysicsNode,
    handle: RigidBodyHandle,
    pipeline: PhysicsPipeline,
    islands: IslandManager,
    broad_phase: DefaultBroadPhase,
    narrow_phase: NarrowPhase,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    ccd: CCDSolver,
    query: QueryPipeline,
    parameters: IntegrationParameters,
    /// Seconds flown.
    time: f32,
}

impl Sim {
    /// The F-16 at `altitude` (m), flying its nose at `speed` (m/s), turned
    /// by `rotation` - gear up, fly-by-wire on, full military power.
    fn f16(altitude: f32, speed: f32, rotation: UnitQuaternion<f32>) -> Self {
        let spec = AircraftSpec::load("f16").expect("assets/planes/f16 should load");
        let mut physics = spec.physics.clone();
        physics.rigidbody.initial_velocity = rotation * Vector3::new(0.0, 0.0, speed);
        let def = PhysicsObjectDef { id: "f16".to_owned(), position: Vector3::new(0.0, altitude, 0.0), rotation, physics };

        let mut bodies = RigidBodySet::new();
        let mut colliders = ColliderSet::new();
        let mut elements = std::collections::HashMap::new();
        load_physics_from_definitions(std::slice::from_ref(&def), &mut colliders, &mut bodies, &mut elements);
        let handle = elements["f16"].as_ref().expect("the jet has a body").rigidbody_handle;

        let behaviors: Vec<Box<dyn PhysicsBehavior>> = vec![Box::new(AircraftPhysics::new(&spec.aero, &spec.engine, &spec.gear, &spec.effects))];
        let mut node = PhysicsNode::new("f16".to_owned(), handle, behaviors);
        let mut input = PhysicsInput::default();
        input.push_event(AircraftEvent::GearUp);
        node.receive_input(input);

        let mut sim = Self {
            bodies,
            colliders,
            node,
            handle,
            pipeline: PhysicsPipeline::new(),
            islands: IslandManager::new(),
            broad_phase: DefaultBroadPhase::new(),
            narrow_phase: NarrowPhase::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            ccd: CCDSolver::new(),
            query: QueryPipeline::new(),
            parameters: IntegrationParameters { dt: STEP, ..Default::default() },
            time: 0.0,
        };
        // Gear up before anything's measured.
        sim.run(2.0, &mut |_| -> f32 { 0.0 });
        sim.time = 0.0;
        sim
    }

    /// One step with this stick position (`elevator`, + forward).
    fn step(&mut self, elevator: f32) {
        let mut controls = PlaneControls::new();
        controls.elevator = elevator;
        controls.throttle = 0.85;
        controls.parking_brake = false;
        controls.fly_by_wire_pitch_autotrim = true;
        let mut input = PhysicsInput::default();
        input.set_state(controls);
        self.node.receive_input(input);

        self.node.fixed_update(&mut self.bodies, &self.colliders, &self.query, STEP);
        self.pipeline.step(
            &Vector3::new(0.0, -9.81, 0.0),
            &self.parameters,
            &mut self.islands,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.ccd,
            Some(&mut self.query),
            &(),
            &(),
        );
        self.time += STEP;
    }

    /// Flies `seconds` with the stick at `stick(time)`.
    fn run(&mut self, seconds: f32, stick: &mut dyn FnMut(f32) -> f32) {
        let end = self.time + seconds;
        while self.time < end {
            let elevator = stick(self.time);
            self.step(elevator);
        }
    }

    /// What the jet's instruments read now.
    fn state(&self) -> AircraftState {
        let payload = self.node.publish().into_iter().next().expect("the jet publishes its state");
        *payload.downcast::<AircraftState>().expect("an AircraftState")
    }

    fn speed(&self) -> f32 {
        self.bodies[self.handle].linvel().magnitude()
    }
}

/// The g over time while holding `elevator` from t = 0, sampled every 0.1 s
/// for `seconds` - printed, and returned as (time, g).
fn g_trace(sim: &mut Sim, elevator: f32, seconds: f32, label: &str) -> Vec<(f32, f32)> {
    let mut trace = Vec::new();
    let mut next_sample = 0.0;
    while sim.time < seconds {
        sim.step(elevator);
        if sim.time >= next_sample {
            let g = sim.state().flight_data.g_meter;
            trace.push((sim.time, g));
            next_sample += 0.1;
        }
    }
    let line: Vec<String> = trace.iter().step_by(2).map(|(t, g)| format!("{t:.1}s {g:+.1}")).collect();
    println!("{label} (speed now {:.0} m/s): {}", sim.speed(), line.join(" | "));
    trace
}

/// When the trace first reaches `g` (rising, or falling for a negative
/// target) - `None` if it never does.
fn time_to(trace: &[(f32, f32)], g: f32) -> Option<f32> {
    trace.iter().find(|(_, value)| if g >= 1.0 { *value >= g } else { *value <= g }).map(|(t, _)| *t)
}

#[test]
fn f16_full_pull_at_mach_1() {
    // Level at 1000 m, banked 90 degrees - the pull is a level turn, no
    // gravity along the lift.
    let banked = UnitQuaternion::from_axis_angle(&Vector3::z_axis(), 90f32.to_radians());
    let mut sim = Sim::f16(1000.0, MACH_1, banked);
    let trace = g_trace(&mut sim, -1.0, 6.0, "full pull, Mach 1, 90 deg bank");
    let peak = trace.iter().map(|(_, g)| *g).fold(f32::MIN, f32::max);
    println!("  -> 8 g at {:?} s, peak {peak:.1} g", time_to(&trace, 8.0));
    assert!(time_to(&trace, 8.0).is_some_and(|t| t < 3.0), "should reach 8 g within 3 s");
    assert!(peak < 9.5, "shouldn't overshoot 9 g by much (peak {peak:.1})");
}

#[test]
fn f16_full_push_at_mach_1() {
    let mut sim = Sim::f16(3000.0, MACH_1, UnitQuaternion::identity());
    let trace = g_trace(&mut sim, 1.0, 4.0, "full push, Mach 1, level");
    let lowest = trace.iter().map(|(_, g)| *g).fold(f32::MAX, f32::min);
    println!("  -> -1.8 g at {:?} s, lowest {lowest:.1} g", time_to(&trace, -1.8));
    assert!(time_to(&trace, -1.8).is_some_and(|t| t < 1.5), "should reach -2 g within ~1.5 s");
    assert!(lowest > -2.6, "shouldn't overshoot -2 g by much (lowest {lowest:.1})");
}

#[test]
fn f16_hands_off_holds_one_g() {
    let mut sim = Sim::f16(3000.0, 250.0, UnitQuaternion::identity());
    let trace = g_trace(&mut sim, 0.0, 5.0, "hands off, 250 m/s, level");
    let (_, last) = *trace.last().unwrap();
    assert!((last - 1.0).abs() < 0.3, "hands off should settle near 1 g (got {last:.2})");
}

#[test]
fn f16_full_pull_at_corner_speed_settles() {
    // ~330 kt - where 9 g first becomes available; the AoA limiter
    // shouldn't let it wobble.
    let banked = UnitQuaternion::from_axis_angle(&Vector3::z_axis(), 90f32.to_radians());
    let mut sim = Sim::f16(1000.0, 170.0, banked);
    let trace = g_trace(&mut sim, -1.0, 6.0, "full pull, 170 m/s, 90 deg bank");
    // The last two seconds: steady, not oscillating.
    let tail: Vec<f32> = trace.iter().filter(|(t, _)| *t > 4.0).map(|(_, g)| *g).collect();
    let spread = tail.iter().copied().fold(f32::MIN, f32::max) - tail.iter().copied().fold(f32::MAX, f32::min);
    assert!(spread < 1.5, "should settle, not oscillate (spread {spread:.2} g)");
}

/// The spread (max - min) of the trace's g after `from` seconds.
fn spread_after(trace: &[(f32, f32)], from: f32) -> f32 {
    let tail: Vec<f32> = trace.iter().filter(|(t, _)| *t > from).map(|(_, g)| *g).collect();
    tail.iter().copied().fold(f32::MIN, f32::max) - tail.iter().copied().fold(f32::MAX, f32::min)
}

#[test]
fn f16_half_stick_pulls_about_five_g() {
    // Half stick back commands 5 g (1 + 0.5 x 8).
    let banked = UnitQuaternion::from_axis_angle(&Vector3::z_axis(), 90f32.to_radians());
    let mut sim = Sim::f16(2000.0, 250.0, banked);
    let trace = g_trace(&mut sim, -0.5, 4.0, "half pull, 250 m/s, 90 deg bank");
    let (_, last) = *trace.last().unwrap();
    assert!((last - 5.0).abs() < 0.5, "half stick should hold ~5 g (got {last:.2})");
    assert!(spread_after(&trace, 2.0) < 0.5, "and hold it steadily");
}

#[test]
fn f16_slow_pull_doesnt_oscillate() {
    let banked = UnitQuaternion::from_axis_angle(&Vector3::z_axis(), 90f32.to_radians());
    let mut sim = Sim::f16(1000.0, 120.0, banked);
    let trace = g_trace(&mut sim, -1.0, 5.0, "full pull, 120 m/s, 90 deg bank");
    assert!(spread_after(&trace, 2.5) < 1.0, "should settle at low speed too (spread {:.2})", spread_after(&trace, 2.5));
}

#[test]
fn f16_high_altitude_pull_doesnt_oscillate() {
    // Thin air at 9 km - the q schedule has to keep it from going sluggish
    // or twitchy.
    let banked = UnitQuaternion::from_axis_angle(&Vector3::z_axis(), 90f32.to_radians());
    let mut sim = Sim::f16(9000.0, 300.0, banked);
    let trace = g_trace(&mut sim, -0.5, 5.0, "half pull, 300 m/s at 9 km, 90 deg bank");
    assert!(time_to(&trace, 4.5).is_some_and(|t| t < 2.0), "should reach ~5 g within 2 s");
    assert!(spread_after(&trace, 2.5) < 1.0, "and settle (spread {:.2})", spread_after(&trace, 2.5));
}
