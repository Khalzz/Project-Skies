use nalgebra::{UnitQuaternion, Vector3};

use crate::app::App;
use crate::engine::input::input;
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
use super::quick_list::QuickList;
use super::quick_menu::{GearStatus, QuickAction, QuickMenuStatus};

/// An aircraft's main-thread half - reads the pilot's input, sends it to the
/// physics half (`AircraftPhysics`), and animates the model from whatever
/// the physics half reports back (`AircraftState`). Owns no physical state
/// itself - see messages.rs for everything the two halves exchange.
pub struct Plane {
    pub controls: PlaneControls,
    pub input_locked: bool,
    pub fcs: Fcs,
    control_surfaces: Vec<ControlSurface>,
    afterburner: Afterburner,
    gear_meshes: GearMeshes,
    quick_menu: QuickList,
}

impl Plane {
    // Cap on App::aero_debug_trail/App::wing_lift_trail (see those fields'
    // own doc comments) - oldest sample drops once this is exceeded, so the
    // F7 overlay reads as a "recent history" trail rather than growing
    // forever while it's on.
    const AERO_DEBUG_TRAIL_CAPACITY: usize = 500;

    pub fn new() -> Self {
        Self {
            controls: PlaneControls::new(),
            input_locked: false,
            fcs: Fcs::new(),
            control_surfaces: Self::default_control_surfaces(),
            afterburner: Afterburner::new(),
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
        let has_control = !self.input_locked && !app.is_paused;
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

    /// F7 overlay's rolling history - see `App::aero_debug_trail`/
    /// `App::wing_lift_trail`.
    fn record_aero_debug_trail(app: &mut App, state: &AircraftState) {
        let flight_data = &state.flight_data;
        app.aero_debug_trail.push_back((flight_data.speedometer, flight_data.aoa_y, flight_data.roll_rate));
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
    }
}

impl Behavior for Plane {
    fn update(&mut self, node: &mut Node, _cameras: &mut SceneCameras, app: &mut App, delta_time: f32) {
        // 1. Input → physics half.
        if !self.input_locked {
            self.read_pilot_input(node, delta_time);
        }
        self.update_quick_menu(node, app);
        self.controls.fly_by_wire_pitch_autotrim = self.fcs.pitch_autotrim;
        node.set_physics_input(self.controls.clone());

        // 2. Physics half → model. `None` until the physics thread has
        // reported back for the first time.
        let state = node.physics_state::<AircraftState>();
        let scale = node.get_property::<Transform3D>().map(|transform| transform.scale).unwrap_or(Vector3::new(1.0, 1.0, 1.0));
        let Some(model_ref) = node.get_property::<ModelProperty>().map(|model| &model.model_ref) else { return };
        let Some(model_instance) = app.game_models.get_mut(model_ref) else { return };
        let model = &mut model_instance.model;

        self.afterburner.update(self.controls.throttle, delta_time);
        if let Some(afterburner_mesh) = model.mesh_lists.get_mut("transparent").and_then(|meshes| meshes.get_mut("Afterburner")) {
            let new_transform = Transform::new(afterburner_mesh.transform.position, afterburner_mesh.transform.rotation, Vector3::new(1.0, 1.0, self.afterburner.value));
            afterburner_mesh.change_transform(&app.renderer.queue, new_transform);
        }

        let simulated_elevator = state.map(|state| state.elevator_control_input);
        for surface in &mut self.control_surfaces {
            surface.apply(model, &self.controls, simulated_elevator, delta_time, &app.renderer.queue);
        }

        if let Some(state) = state {
            self.gear_meshes.place(model, &state.wheels, state.gear_deploy, scale, &app.renderer.queue);

            if app.show_aero_debug_overlay {
                Self::record_aero_debug_trail(app, state);
            }
        }
    }
}
