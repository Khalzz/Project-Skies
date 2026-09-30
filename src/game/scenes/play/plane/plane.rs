use nalgebra::{UnitQuaternion, Vector3};

use crate::app::App;
use crate::engine::input::input;
use crate::engine::physics::water_contact::WaterContactState;
use crate::engine::rendering::render_pipeline::render_pass::AeroSample;
use crate::engine::rendering::camera::handler::SceneCameras;
use crate::engine::scene_manager::behavior::Behavior;
use crate::engine::scene_manager::node::Node;
use crate::engine::scene_manager::properties::{Model as ModelProperty, Transform3D};
use crate::engine::utils::lerps::lerp;
use crate::transform::Transform;

use super::afterburner::Afterburner;
use super::control_surfaces::{ControlInput, ControlSurface, SurfaceBase};
use super::controls::PlaneControls;
use super::fcs::Fcs;
use super::landing_gear::GearMeshes;
use super::messages::{AircraftEvent, AircraftState};
use super::effects;
use super::pilot::Pilot;
use crate::engine::particles::ParticleEmitters;
use super::quick_list::QuickList;
use super::quick_menu::{GearStatus, QuickAction, QuickMenuStatus};

/// An aircraft's main-thread half - reads the pilot's input, sends it to the
/// physics half (`AircraftPhysics`), and animates the model from whatever
/// the physics half reports back (`AircraftState`). Owns no physical state
/// itself - see messages.rs for everything the two halves exchange.
pub struct Plane {
    pub controls: PlaneControls,
    pub input_locked: bool,
    /// Hit the water - see `check_wreck`. Never cleared (a restart builds
    /// a new plane).
    pub wrecked: bool,
    /// The pilot's G tolerance - grey/black/red-out, passing out.
    pub pilot: Pilot,
    /// Wingtip vortex strength 0..1, eased - see update_effects.
    vortex_strength: f32,
    pub fcs: Fcs,
    control_surfaces: Vec<ControlSurface>,
    afterburner: Afterburner,
    // The afterburner mesh's own scale from the model, captured the first
    // frame - its length is stretched by `afterburner.value` on top of it.
    afterburner_base_scale: Option<Vector3<f32>>,
    gear_meshes: GearMeshes,
    quick_menu: QuickList,
}

impl Plane {
    // Cap on App::aero_debug_trail/App::wing_lift_trail (see those fields'
    // own doc comments) - oldest sample drops once this is exceeded, so the
    // F7 overlay reads as a "recent history" trail rather than growing
    // forever while it's on.
    const AERO_DEBUG_TRAIL_CAPACITY: usize = 500;
    // How much history App::pitch_rate_trail keeps (s).
    const PITCH_RATE_TRAIL_SECONDS: f32 = 10.0;

    pub fn new() -> Self {
        Self {
            controls: PlaneControls::new(),
            input_locked: false,
            wrecked: false,
            pilot: Pilot::new(),
            vortex_strength: 0.0,
            fcs: Fcs::new(),
            control_surfaces: Self::default_control_surfaces(),
            afterburner: Afterburner::new(),
            afterburner_base_scale: None,
            gear_meshes: GearMeshes::new(),
            quick_menu: QuickList::new(),
        }
    }

    fn default_control_surfaces() -> Vec<ControlSurface> {
        let rudder_base = UnitQuaternion::from_axis_angle(&Vector3::x_axis(), -28.4493_f32.to_radians());
        vec![
            ControlSurface::new("opaque", "left_elevator", *Vector3::x_axis(), 0.15, 7.0, SurfaceBase::None, ControlInput::Elevator),
            ControlSurface::new("opaque", "right_elevator", *Vector3::x_axis(), 0.15, 7.0, SurfaceBase::None, ControlInput::Elevator),
            ControlSurface::new("opaque", "left_aleron", *Vector3::x_axis(), -0.5, 7.0, SurfaceBase::Captured(None), ControlInput::Aileron),
            ControlSurface::new("opaque", "right_aleron", *Vector3::x_axis(), 0.5, 7.0, SurfaceBase::Captured(None), ControlInput::Aileron),
            ControlSurface::new("opaque", "rudder_0", *Vector3::y_axis(), 0.5, 7.0, SurfaceBase::Fixed(rudder_base), ControlInput::Rudder),
            ControlSurface::new("opaque", "rudder_1", *Vector3::y_axis(), 0.5, 7.0, SurfaceBase::Fixed(rudder_base), ControlInput::Rudder),
        ]
    }

    /// Stick/throttle/trim into `controls`, toggles into `fcs`, and one-shot
    /// gear commands straight to the physics half as events.
    fn read_pilot_input(&mut self, node: &mut Node, delta_time: f32) {
        const STICK_INPUT_LERP_SPEED: f32 = 6.0;
        let target_elevator = input::get_axis("pitch_down", "pitch_up");
        let target_aileron = input::get_axis("roll_left", "roll_right");
        let target_rudder = input::get_axis("rudder_left", "rudder_right");

        let target_elevator = if self.fcs.pitch_autotrim {
            target_elevator
        } else {
            target_elevator + self.controls.trim.pitch
        };

        let target_aileron = target_aileron + self.controls.trim.roll;

        self.controls.elevator = lerp(self.controls.elevator, target_elevator, delta_time * STICK_INPUT_LERP_SPEED);
        self.controls.aileron = lerp(self.controls.aileron, target_aileron, delta_time * STICK_INPUT_LERP_SPEED);
        self.controls.rudder = lerp(self.controls.rudder, target_rudder, delta_time * STICK_INPUT_LERP_SPEED);

        const THROTTLE_RATE: f32 = 0.5;
        self.controls.throttle = (self.controls.throttle + input::get_axis("throttle_down", "throttle_up") * THROTTLE_RATE * delta_time).clamp(0.0, 1.0);
        self.controls.trim.update(delta_time);

        if input::is_action_just_pressed("toggle_fbw_pitch") {
            self.apply_quick_action(node, QuickAction::ToggleFlyByWire);
        }

        self.controls.wheel_brake = input::action_strength("wheel_brake");
        if input::is_action_just_pressed("toggle_parking_brake") {
            self.apply_quick_action(node, QuickAction::ToggleParkingBrake);
        }

        if input::is_action_just_pressed("toggle_landing_gear") {
            self.apply_quick_action(node, QuickAction::ToggleGear);
        }
        if input::is_action_just_pressed("landing_gear_up") {
            node.push_physics_event(AircraftEvent::GearUp);
        }
        if input::is_action_just_pressed("landing_gear_down") {
            node.push_physics_event(AircraftEvent::GearDown);
        }
    }

    /// Runs one of the plane's toggles - shared by the keyboard keys (see
    /// `read_pilot_input`) and the D-pad quick menu, so both always do the
    /// same thing.
    fn apply_quick_action(&mut self, node: &mut Node, action: QuickAction) {
        match action {
            QuickAction::ToggleGear => node.push_physics_event(AircraftEvent::ToggleGear),
            QuickAction::ToggleParkingBrake => {
                self.controls.parking_brake = !self.controls.parking_brake;
                println!("Parking brake: {}", if self.controls.parking_brake { "SET" } else { "RELEASED" });
            }
            QuickAction::ToggleFlyByWire => {
                self.fcs.pitch_autotrim = !self.fcs.pitch_autotrim;
                println!(
                    "Pitch fly-by-wire (g-command): {}",
                    if self.fcs.pitch_autotrim { "ENGAGED - stick commands g" } else { "OFF - manual + trim" }
                );
            }
        }
    }

    /// The D-pad quick menu: navigate/run actions while the player has
    /// control, and keep its HUD in sync (hidden during cinematics/pause).
    fn update_quick_menu(&mut self, node: &mut Node, app: &mut App) {
        let has_control = !self.input_locked && !self.wrecked && !self.pilot.is_unconscious() && !app.is_paused;
        if has_control {
            if let Some(action) = self.quick_menu.update() {
                self.apply_quick_action(node, action);
            }
        }

        let gear_deploy = node.physics_state::<AircraftState>().map(|state| state.gear_deploy).unwrap_or(1.0);
        let status = QuickMenuStatus {
            gear: GearStatus::from_deploy(gear_deploy),
            parking_brake: self.controls.parking_brake,
            fly_by_wire: self.fcs.pitch_autotrim,
        };
        self.quick_menu.refresh_ui(app, &status, has_control);
    }

    /// Any part of the plane touching the water (see the node's
    /// `WaterContact`), or a crash that kills the pilot anywhere (see
    /// `Pilot`), wrecks it: the physics half stops flying it, and the
    /// controls go neutral with the throttle closed and stay that way.
    fn check_wreck(&mut self, node: &mut Node) {
        if self.wrecked {
            return;
        }
        let water_impact = node.physics_state::<WaterContactState>().and_then(|state| state.impact);
        if let Some(impact) = water_impact {
            println!("Crashed into the water at {:.0} m/s (collider {})", impact.speed, impact.collider);
        } else if !self.pilot.is_dead() {
            return;
        }
        self.wrecked = true;
        self.controls = PlaneControls { parking_brake: self.controls.parking_brake, ..PlaneControls::new() };
        self.controls.throttle = 0.0;
        node.push_physics_event(AircraftEvent::Wreck);
    }

    /// Drives the node's particle emitters (see plane::effects): wingtip
    /// vortices streaming off under G, smoke off the wreck.
    fn update_effects(&mut self, node: &mut Node, delta_time: f32) {
        // Condensation shows from ~3.5 G, full by ~7 (either sign); the raw G
        // is spiky, so the strength eases toward it.
        let g = node.physics_state::<AircraftState>().map(|state| state.flight_data.g_meter.abs()).unwrap_or(1.0);
        let t = ((g - 3.5) / 3.5).clamp(0.0, 1.0);
        let target = if self.wrecked { 0.0 } else { t * t * (3.0 - 2.0 * t) };
        self.vortex_strength = lerp(self.vortex_strength, target, (delta_time * 5.0).min(1.0));

        let Some(emitters) = node.get_property_mut::<ParticleEmitters>() else { return };
        for name in [effects::LEFT_VORTEX, effects::RIGHT_VORTEX] {
            if let Some(vortex) = emitters.get_mut(name) {
                vortex.intensity = self.vortex_strength;
                vortex.enabled = self.vortex_strength > 0.02;
            }
        }
        if let Some(smoke) = emitters.get_mut(effects::WRECK_SMOKE) {
            smoke.enabled = self.wrecked;
        }
    }

    /// The pilot feels the plane's G - frozen while paused, resting during
    /// cinematics (physics is paused, the G is stale), and after a wreck
    /// only the crash itself (see Pilot::recover).
    fn update_pilot(&mut self, node: &Node, app: &App, delta_time: f32) {
        if app.is_paused {
            return;
        }
        let flight_data = node.physics_state::<AircraftState>().map(|state| &state.flight_data);
        match flight_data {
            Some(flight_data) if !self.input_locked => {
                if self.wrecked {
                    self.pilot.recover(flight_data.impact_g, delta_time);
                } else {
                    self.pilot.update(flight_data.g_meter, flight_data.impact_g, delta_time);
                }
            }
            _ => self.pilot.rest(delta_time),
        }
    }

    /// F7 overlay's rolling history - see `App::aero_debug_trail`/
    /// `App::wing_lift_trail`/`App::pitch_rate_trail`.
    fn record_aero_debug_trail(app: &mut App, state: &AircraftState, controls: &PlaneControls, delta_time: f32) {
        let flight_data = &state.flight_data;
        app.aero_debug_trail.push_back(AeroSample {
            speed_kt: flight_data.speedometer,
            mach: flight_data.mach,
            aoa_deg: flight_data.aoa_y,
            altitude_m: flight_data.altimeter,
            roll_rate: flight_data.roll_rate,
            // FlightData::pitch_rate is about the body's +X (the left wing),
            // so its + is nose DOWN - flipped to nose-up +.
            pitch_rate: -flight_data.pitch_rate,
            yaw_rate: flight_data.yaw_rate,
            climb_rate: flight_data.velocity.y,
            aileron: controls.aileron,
            // elevator + is stick forward (see Plane::update) - flipped to
            // pull +.
            pitch_stick: -controls.elevator,
            rudder: controls.rudder,
            up_alignment: flight_data.up_alignment,
        });
        if app.aero_debug_trail.len() > Self::AERO_DEBUG_TRAIL_CAPACITY {
            app.aero_debug_trail.pop_front();
        }

        let main_wing_lift_y = state.wing_lift_forces.get("Left wing").map(|f| f.y).unwrap_or(0.0);
        let elevator_wing_lift_y = state.wing_lift_forces.get("Right elevator wing").map(|f| f.y).unwrap_or(0.0);
        let next_index = app.wing_lift_trail.back().map(|(i, _, _)| i + 1.0).unwrap_or(0.0);
        app.wing_lift_trail.push_back((next_index, main_wing_lift_y, elevator_wing_lift_y));
        if app.wing_lift_trail.len() > Self::AERO_DEBUG_TRAIL_CAPACITY {
            app.wing_lift_trail.pop_front();
        }

        let now = app.pitch_rate_trail.back().map(|(t, _)| t + delta_time).unwrap_or(0.0);
        app.pitch_rate_trail.push_back((now, flight_data.pitch_rate));
        while app.pitch_rate_trail.front().is_some_and(|(t, _)| now - t > Self::PITCH_RATE_TRAIL_SECONDS) {
            app.pitch_rate_trail.pop_front();
        }
    }
}

impl Behavior for Plane {
    fn update(&mut self, node: &mut Node, _cameras: &mut SceneCameras, app: &mut App, delta_time: f32) {
        // 1. Input → physics half.
        self.update_pilot(node, app, delta_time);
        self.check_wreck(node);
        if !app.is_paused {
            self.update_effects(node, delta_time);
        }
        if !self.input_locked && !self.wrecked && !self.pilot.is_unconscious() {
            self.read_pilot_input(node, delta_time);
        } else if self.pilot.is_unconscious() {
            // Passed out: stick and pedals straight back to center, held
            // there until the pilot comes to (the throttle stays where it
            // was). Center means the same as a hands-off stick in
            // read_pilot_input - trim kept: without fly-by-wire the jet
            // needs its pitch trim to hold ~1 G, and without it a centered
            // stick noses it over into heavy negative G.
            self.controls.elevator = if self.fcs.pitch_autotrim { 0.0 } else { self.controls.trim.pitch };
            self.controls.aileron = self.controls.trim.roll;
            self.controls.rudder = 0.0;
        }
        self.update_quick_menu(node, app);
        self.controls.fly_by_wire_pitch_autotrim = self.fcs.pitch_autotrim;
        node.set_physics_input(self.controls.clone());

        // 2. Physics half → model. `None` until the physics thread has
        // reported back for the first time.
        let state = node.physics_state::<AircraftState>();
        let node_scale = node.get_property::<Transform3D>().map(|transform| transform.scale).unwrap_or(Vector3::new(1.0, 1.0, 1.0));
        let Some(model_property) = node.get_property::<ModelProperty>() else { return };
        // What maps model space to the world - node x model scale.
        let scale = model_property.render_scale(node_scale);
        // The key its loaded copy is kept under - see render_bridge.
        let model_ref = model_property.source.as_ref().map(|source| source.key())
            .or_else(|| app.model_sources.get(&model_property.model_ref).map(|source| source.key()))
            .unwrap_or_else(|| model_property.model_ref.clone());
        let model_ref = &model_ref;
        let Some(model_instance) = app.game_models.get_mut(model_ref) else { return };
        let model = &mut model_instance.model;

        self.afterburner.update(state.map(|state| state.afterburner).unwrap_or(0.0), delta_time);
        if let Some(afterburner_mesh) = model.mesh_lists.get_mut("transparent").and_then(|meshes| meshes.get_mut("Afterburner")) {
            let base = *self.afterburner_base_scale.get_or_insert(afterburner_mesh.transform.scale);
            let new_transform = Transform::new(afterburner_mesh.transform.position, afterburner_mesh.transform.rotation, Vector3::new(base.x, base.y, base.z * self.afterburner.value));
            afterburner_mesh.change_transform(&app.renderer.queue, new_transform);
        }

        let simulated_elevator = state.map(|state| state.elevator_control_input);
        for surface in &mut self.control_surfaces {
            surface.apply(model, &self.controls, simulated_elevator, delta_time, &app.renderer.queue);
        }

        if let Some(state) = state {
            self.gear_meshes.place(model, &state.wheels, state.gear_deploy, scale, &app.renderer.queue);

            if app.debug_overlay_visible() {
                Self::record_aero_debug_trail(app, state, &self.controls, delta_time);
            }
        }
    }
}
