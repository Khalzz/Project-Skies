use std::{collections::HashMap, f32::consts::PI, time::Instant};

use glyphon::{cosmic_text::Align, Color, FontSystem};
use nalgebra::{vector, Point3, Quaternion, UnitQuaternion, Vector3};
use rand::{rngs::ThreadRng, Rng};
use rapier3d::prelude::RigidBody;
use sdl2::controller::GameController;
use crate::{app::{App, AppState}, audio::subtitles::Subtitle, rendering::{camera::CameraRenderizable, ui::UiContainer}, transform::Transform, ui::{ui_node::{UiNode, UiNodeContent, UiNodeParameters, Visibility}, ui_transform::UiTransform}, utils::lerps::{lerp, lerp_quaternion}};
use super::{airfoil::AirFoil, controller::Controller, wing::Wing, wheel::Wheel};

pub enum CameraState {
    Normal,
    Cockpit,
    Cinematic,
    Frontal,
    Free,
}

pub struct Bandit {
    tag: String,
    locked: bool,
}

pub struct CameraData {
    camera_state: CameraState,
    pub look_at: Option<Vector3<f32>>,
    pub next_look_at: Option<Vector3<f32>>,
    pub mod_quaternion: UnitQuaternion<f32>
}

pub struct BlinkingAlert {
    alert_state: bool,
    time_alert: f32
}

pub struct BaseRotations {
    left_aleron: Option<Quaternion<f32>>,
    right_aleron: Option<Quaternion<f32>>,
}

pub struct FlightData {
    pub altimeter: f32,
    pub speedometer: f32,
    pub g_meter: f32,
}

pub struct PlaneSystems {
    bandits: Vec<Bandit>,
    stall: bool,
    pub flight_data: FlightData,
    pub afterburner_value: f32,
    pub base_rotations: BaseRotations,
    pub flap_ratio: f32,
    pub wings: Vec<Wing>,
    pub wheels: Vec<Wheel>,
    pub previous_velocity: Option<Vector3<f32>>,
}

pub struct GameLogic { // here we define the data we use on our script
    pub controller: Controller,
    pub camera_data: CameraData,
    pub blinking_alerts: HashMap<String, BlinkingAlert>,
    rng: ThreadRng,
    pub plane_systems: PlaneSystems,
    pub gravity: Vector3<f32>,
    pub subtitle_data: Subtitle
} 

impl GameLogic {
    // this is called once
    pub fn new(app: &mut App) -> Self {
        // UI ELEMENTS AND LIST
        let altitude = UiNode::new(
            UiTransform::new(((app.config.width as f32 / 2.0) - (150.0 / 2.0)) - 400.0, (app.config.height as f32 / 2.0) - (30.0 / 2.0), 30.0, 150.0, 0.0), 
            Visibility::new([0.0, 0.0, 0.0, 0.0], [0.0, 255.0, 0.0, 255.0]),
            UiNodeParameters::Text { text: "ALT", color: Color::rgba(0, 255, 75, 255), align: Align::Center, font_size: 20.0}, 
            app,
            None
        );

        let speed = UiNode::new(
            UiTransform::new(((app.config.width as f32 / 2.0) - (150.0 / 2.0)) + 400.0, (app.config.height as f32 / 2.0) - (30.0 / 2.0), 30.0, 150.0, 0.0), 
            Visibility::new([0.0, 0.0, 0.0, 0.0], [0.0, 255.0, 0.0, 255.0]),
            UiNodeParameters::Text { text: "SPD", color: Color::rgba(0, 255, 75, 255), align: Align::Center, font_size: 20.0}, 
            app,
            None
        );
        
        let framerate = UiNode::new(
            UiTransform::new(10.0, 10.0, 30.0, 100.0, 0.0), 
            Visibility::new([0.0, 0.0, 0.0, 0.0], [0.0, 255.0, 0.0, 255.0]),
            UiNodeParameters::Text { text: "90 fps", color: Color::rgba(0, 255, 75, 255), align: Align::Center, font_size: 20.0}, 
            app,
            None
        );

        let g_number = UiNode::new(
            UiTransform::new(10.0, 50.0, 30.0, 100.0, 0.0), 
            Visibility::new([0.0, 0.0, 0.0, 0.0], [0.0, 255.0, 0.0, 255.0]),
            UiNodeParameters::Text { text: "G", color: Color::rgba(0, 255, 75, 255), align: Align::Center, font_size: 20.0}, 
            app,
            None
        );
        
        let altitude_alert = UiNode::new(
            UiTransform::new((app.config.width as f32 / 2.0) - (140.0 / 2.0), ((app.config.height as f32 / 2.0) - (50.0 / 2.0)) + 50.0, 50.0, 140.0, 0.0), 
            Visibility::new([0.0, 0.0, 0.0, 0.0], [255.0, 0.0, 0.0, 255.0]),
            UiNodeParameters::Text { text: "ALT", color: Color::rgba(0, 255, 75, 255), align: Align::Center, font_size: 20.0 }, 
            app,
            None
        );

        let compass = UiNode::new(
            UiTransform::new((app.config.width as f32 / 2.0) - (100.0 / 2.0), 300.0, 50.0, 100.0, 0.0), 
            Visibility::new([0.0, 0.0, 0.0, 0.0], [0.0, 255.0, 0.0, 255.0]),
            UiNodeParameters::Text { text: "90°", color: Color::rgba(0, 255, 75, 255), align: Align::Center, font_size: 20.0 }, 
            app,
            None
        );

        let subtitle = UiNode::new(
            UiTransform::new((app.config.width as f32 / 2.0) - (app.config.width as f32 * 0.9) / 2.0, app.config.height as f32 * 0.7, 100.0, app.config.width as f32 * 0.9, 0.0), 
            Visibility::new([0.0, 0.0, 0.0, 0.7], [0.0, 0.0, 0.0, 0.0]),
            UiNodeParameters::VerticalContainerData { margin: 10.0, separation: 10.0, children: vec![] }, 
            app,
            None
        );
        

        app.ui.renderizable_elements.clear();
        app.ui.renderizable_elements.insert("static".to_owned(), UiContainer::Tagged(HashMap::new()));
        app.ui.renderizable_elements.insert("bandits".to_owned(), UiContainer::Untagged(vec![]));

        app.ui.add_to_ui("static".to_owned(), "altitude".to_owned(), altitude);

        app.ui.add_to_ui("static".to_owned(), "speed".to_owned(), speed);
        app.ui.add_to_ui("static".to_owned(), "g_number".to_owned(), g_number);
        app.ui.add_to_ui("static".to_owned(), "compass".to_owned(), compass);
        app.ui.add_to_ui("static".to_owned(), "altitude_alert".to_owned(), altitude_alert);
        app.ui.add_to_ui("static".to_owned(), "framerate".to_owned(), framerate);
        app.ui.add_to_ui("static".to_owned(), "subtitles".to_owned(), subtitle);
        // app.ui.add_to_ui("static".to_owned(), "level_data".to_owned(), level_data);

        let mut subtitle_data = Subtitle::new();

        // this might give error
        // app.ui.add_to_ui("static".to_owned(), "crosshair".to_owned(), crosshair);

        let camera_data = CameraData { 
            camera_state: CameraState::Normal, 
            look_at: None,
            next_look_at: None,
            mod_quaternion: UnitQuaternion::identity(),
        };

        let fellow = Bandit {
            tag: "fellow_aviator".to_owned(),
            locked: true,
        };

        let tower = Bandit {
            tag: "tower".to_owned(),
            locked: false,
        };

        let tower2 = Bandit {
            tag: "tower2".to_owned(),
            locked: false,
        };

        let crane = Bandit {
            tag: "crane".to_owned(),
            locked: false,
        };

        // load airfoil:
        let naca_2412 = AirFoil::new("assets/aero_data/f16.ron".to_owned());
        let naca_0012 = AirFoil::new("assets/aero_data/f16-elevators.ron".to_owned());

        // i have to also add left and right ailerons
        let wings = vec![
            Wing::new(vector![8.5, 0.0, 1.0], 6.96, 2.50, 0.0, naca_2412.clone(), vector![0.0, 1.0, 0.0], 0.5), // left wing
            Wing::new(vector![-8.5, 0.0, 1.0], 6.96, 2.50, 0.0, naca_2412.clone(), vector![0.0, 1.0, 0.0], 0.5), // right wing
            Wing::new(vector![0.0, 0.0, -9.0], 6.54, 2.70, 0.0, naca_0012.clone(), vector![0.0, 1.0, 0.0], 1.0), // elevator wing
            Wing::new(vector![0.0, 1.0, -9.0], 6.96, 2.50, 0.0, naca_0012.clone(), vector![1.0, 0.0, 0.0], 0.15) // rudder wing
        ];

        let wheels = vec![
            Wheel::new(vector![0.0, -0.0, 10.0], 4.0, 18000.0, 20000.0, "wheel-f".to_string()),
            Wheel::new(vector![-3.0, -0.0, 0.0], 4.0, 20000.0, 20000.0, "wheel-lb".to_string()),
            Wheel::new(vector![3.0, -0.0, 0.0], 4.0, 20000.0, 20000.0, "wheel-rb".to_string())
        ];

        let plane_systems = PlaneSystems {
            bandits: vec![tower, tower2, crane, fellow],
            stall: false,
            afterburner_value: 0.0,
            base_rotations: BaseRotations { left_aleron: None, right_aleron: None },
            flap_ratio: 0.0,
            wings,
            wheels,
            previous_velocity: None,
            flight_data: FlightData { altimeter: 0.0, speedometer: 0.0, g_meter: 0.0 }
        };

        let rng = rand::thread_rng();

        let mut blinking_alerts: HashMap<String, BlinkingAlert> = HashMap::new();
        blinking_alerts.insert("altitude".to_owned(), BlinkingAlert { alert_state: false, time_alert: 0.0 });
        blinking_alerts.insert("stall".to_owned(), BlinkingAlert { alert_state: false, time_alert: 0.0 });

        let gravity = vector![0.0, -9.81, 0.0];

        Self {
            controller: Controller::new(0.3, 0.2),
            camera_data,
            blinking_alerts,
            plane_systems,
            rng,
            gravity,
            subtitle_data
        }
    }

    // this is called every frame
    pub fn update(&mut self, mut app_state: &mut AppState, mut event_pump: &mut sdl2::EventPump, app: &mut App, controller: &mut Option<GameController>) {
        self.controller.update(&mut app_state, &mut event_pump, app, controller, app.time.delta_time); // should be first in the function

        if self.controller.fix_view.just_pressed {
            println!("se agrego un nuevo texto");
            self.subtitle_data.add_text(&"ejemplo de mensaje".to_string(), app);
        }

        self.subtitle_data.update(app);
        self.camera_control(app, app.time.delta_time);
        self.ui_control(app, app.time.delta_time);
        self.plane_movement(app, app.time.delta_time);
    }

    

    fn plane_movement (&mut self, app: &mut App, delta_time: f32) {
        let plane = app.renderizable_instances.get_mut("player").unwrap();
        let plane_model = app.game_models.get_mut(&plane.model_ref).unwrap();

        // elevators
        if let Some(elevator) = plane_model.model.meshes.get_mut("left_elevator") {
            let final_rotation = UnitQuaternion::from_axis_angle(&Vector3::x_axis() ,0.15 * -self.controller.y);
            let elevator_rotation = lerp_quaternion(elevator.transform.rotation,  *final_rotation, app.time.delta_time * 7.0);
            let elevator_transform = Transform::new(elevator.transform.position, elevator_rotation, elevator.transform.scale);
            elevator.change_transform(&app.queue, elevator_transform);
        }

        if let Some(elevator) = plane_model.model.meshes.get_mut("right_elevator") {
            let final_rotation = UnitQuaternion::from_axis_angle(&Vector3::x_axis() ,0.15 * -self.controller.y);
            let elevator_rotation = lerp_quaternion(elevator.transform.rotation,  *final_rotation, app.time.delta_time * 7.0);
            let elevator_transform = Transform::new(elevator.transform.position, elevator_rotation, elevator.transform.scale);
            elevator.change_transform(&app.queue, elevator_transform);
        }

        // wings
        /* 
        let l_wing = app.game_models.get_mut(&plane.model_ref).unwrap().model.meshes.get_mut("left_wing").unwrap();
        let l_wing_rotation = lerp_quaternion(l_wing.instance.transform.rotation,Quaternion::from_angle_y(Rad(angle)), delta_time);
        let l_wing_transform = Transform::new(l_wing.instance.transform.position, l_wing_rotation, l_wing.instance.transform.scale);
        l_wing.change_transform(&app.queue, l_wing_transform);

        let r_wing = app.game_models.get_mut(&plane.model_ref).unwrap().model.meshes.get_mut("right_wing").unwrap();
        let r_wing_rotation = lerp_quaternion(r_wing.instance.transform.rotation,Quaternion::from_angle_y(Rad(-angle)), delta_time);
        let r_wing_transform = Transform::new(r_wing.instance.transform.position, r_wing_rotation, r_wing.instance.transform.scale);
        r_wing.change_transform(&app.queue, r_wing_transform);
        */

        if let Some(aleron) = plane_model.model.meshes.get_mut("left_aleron") {
            match self.plane_systems.base_rotations.left_aleron {
                Some(base_rotation) => {
                    let dependent = UnitQuaternion::from_quaternion(base_rotation.clone()) * UnitQuaternion::from_axis_angle(&Vector3::x_axis() ,0.5 * -self.controller.x);
                    let aleron_rotation = lerp_quaternion(aleron.transform.rotation,  *dependent, app.time.delta_time * 7.0);
                    let aleron_transform = Transform::new(aleron.transform.position, aleron_rotation, aleron.transform.scale);
                    aleron.change_transform(&app.queue, aleron_transform);
                },
                None => {
                    self.plane_systems.base_rotations.left_aleron = Some(aleron.transform.rotation);
                },
            }
        }

        if let Some(aleron) = plane_model.model.meshes.get_mut("right_aleron") {
            match self.plane_systems.base_rotations.right_aleron {
                Some(base_rotation) => {
                    let dependent = UnitQuaternion::from_quaternion(base_rotation.clone()) * UnitQuaternion::from_axis_angle(&Vector3::x_axis(), 0.5 * self.controller.x);
                    let aleron_rotation = lerp_quaternion(aleron.transform.rotation,  *dependent, app.time.delta_time * 7.0);
                    let aleron_transform = Transform::new(aleron.transform.position, aleron_rotation, aleron.transform.scale);
                    aleron.change_transform(&app.queue, aleron_transform);
                },
                None => {
                    // this is not correctly resetting once the plane is reseted
                    self.plane_systems.base_rotations.right_aleron = Some(aleron.transform.rotation);
                },
            }
        }

        // rudders
        // only rudder or left rudder if it haves 2
        if let Some(rudder) = plane_model.model.meshes.get_mut("rudder_0") {
            let rudder_rotation = lerp_quaternion(rudder.transform.rotation, *UnitQuaternion::from_axis_angle(&Vector3::x_axis(),-28.4493 * PI / 180.0) * *UnitQuaternion::from_axis_angle(&Vector3::y_axis(),0.5 * self.controller.yaw), delta_time * 7.0);
            let rudder_transform = Transform::new(rudder.transform.position, rudder_rotation, rudder.transform.scale);
            rudder.change_transform(&app.queue, rudder_transform);
        }

        // right rudder if it haves 2
        if let Some(rudder) = plane_model.model.meshes.get_mut("rudder_1") {
            let rudder_rotation = lerp_quaternion(rudder.transform.rotation, *UnitQuaternion::from_axis_angle(&Vector3::x_axis(),-28.4493 * PI / 180.0) * *UnitQuaternion::from_axis_angle(&Vector3::y_axis(),0.5 * self.controller.yaw), delta_time * 7.0);
            let rudder_transform = Transform::new(rudder.transform.position, rudder_rotation, rudder.transform.scale);
            rudder.change_transform(&app.queue, rudder_transform);
        }

        if let Some(afterburner) = plane_model.model.meshes.get_mut("Afterburner") {
            if self.controller.power > 0.0 {
                self.plane_systems.afterburner_value =  lerp(self.plane_systems.afterburner_value, self.controller.power + self.rng.gen_range(-0.5..0.5), app.time.delta_time * 20.0);
            } else {
                self.plane_systems.afterburner_value = lerp(self.plane_systems.afterburner_value, 0.0, delta_time * 2.0)
            }

            afterburner.change_transform(&app.queue, Transform::new(afterburner.transform.position, afterburner.transform.rotation, Vector3::new(1.0, 1.0, self.plane_systems.afterburner_value)));
        }

        match &plane.physics_data {
            Some(physics_data) => {
                if let Some(rigidbody) = app.physics.rigidbody_set.get_mut(physics_data.rigidbody_handle) {
                    self.plane_systems.flight_data.g_meter = self.calculate_g_forces(rigidbody, app.time.delta_time);
                    self.plane_systems.flight_data.speedometer = rigidbody.linvel().norm() * 3.6; // in kmh, if i want it as knots change the 3.6 to 1.94384

                    rigidbody.reset_torques(true);
                    rigidbody.reset_forces(true);

                    // Thrust                    
                    let max_thrust = 60000.0; // newtons of force generated by engine
                    let power_value_world = rigidbody.rotation() * nalgebra::Vector3::new(0.0, 0.0, max_thrust * self.controller.power);

                    // Apply the thrust force to the rigidbody.
                    rigidbody.add_force(power_value_world, true);
                    // Thrust

                    self.plane_systems.wings[0].control_input = self.controller.x;
                    self.plane_systems.wings[1].control_input = -self.controller.x;
                    self.plane_systems.wings[2].control_input = self.controller.y;
                    self.plane_systems.wings[3].control_input = self.controller.yaw;
                    
                    for wing in &mut self.plane_systems.wings {
                        wing.physics_force(rigidbody, &mut app.physics.render_physics.renderizable_lines);
                    }
                }

                for (_i, wheel) in &mut self.plane_systems.wheels.iter_mut().enumerate() {
                    if let Some((suspension_force, suspension_origin, wheel_position)) = wheel.update_wheel(&physics_data, &mut app.physics.render_physics.renderizable_lines, &app.physics.collider_set, &mut app.physics.rigidbody_set, &app.physics.query_pipeline) {
                        if let Some(rigidbody) = app.physics.rigidbody_set.get_mut(physics_data.rigidbody_handle) {
                            
                            if let Some(wheel_mesh) = &mut plane_model.model.meshes.get_mut(&wheel.mesh_name) {
                                let final_pos =  rigidbody.rotation().inverse() * (wheel_position - rigidbody.translation());

                                wheel_mesh.transform.position = Vector3::new(final_pos.x / plane.instance.transform.scale.x, final_pos.y / plane.instance.transform.scale.y, final_pos.z / plane.instance.transform.scale.z);
                                wheel_mesh.update_transform(&app.queue);
                            }

                            rigidbody.add_force_at_point(suspension_force, suspension_origin.into(), true);
                        }
                    }
                }
            },
            None => {

            },
        }
    }

    pub fn calculate_g_forces(&mut self, rigidbody: &RigidBody, delta_time: f32) -> f32 {
        let current_velocity = rigidbody.linvel();
        let mut g_forces = 0.0;

        if let Some(prev_velocity) = self.plane_systems.previous_velocity {
            let acceleration = (current_velocity - prev_velocity) / delta_time;

            let total_acceleration = acceleration + self.gravity;

            // Calculate G-forces
            g_forces = total_acceleration.magnitude() / 9.81;

            // Update previous velocity
            self.plane_systems.previous_velocity = Some(*current_velocity);
        }

        self.plane_systems.previous_velocity = Some(*current_velocity);
        g_forces
    }

    fn camera_control(&mut self, app: &mut App, delta_time: f32) {
        if let Some(player) = app.renderizable_instances.get_mut("player") {
            match &player.physics_data {
                Some(physics_data) => {
                    if let Some(rigidbody) = app.physics.rigidbody_set.get_mut(physics_data.rigidbody_handle) {
                        match self.camera_data.camera_state {
                            CameraState::Normal => {
                                app.camera.camera.up = rigidbody.rotation() * *Vector3::y_axis();
                                app.camera.projection.fovy = 60.0;
                                app.camera.camera.position = (*rigidbody.translation() + (*rigidbody.rotation() * Vector3::new(0.0, 7.0, -28.0))).into();
                                app.camera.camera.look_at((rigidbody.translation() + (rigidbody.rotation() * Vector3::new(0.0, 0.0, 100.0))).into());
                            },
                            CameraState::Cockpit => {
                                app.camera.camera.up = rigidbody.rotation() * *Vector3::y_axis();
                                app.camera.projection.fovy = 70.0;
                                if let Some(cameras) = &player.instance.metadata.cameras {
                                    app.camera.camera.position = (*rigidbody.translation() + (*rigidbody.rotation() * cameras.cockpit_camera)).into();
                                }
                                app.camera.camera.look_at((rigidbody.translation() + (rigidbody.rotation() * Vector3::new(0.0, 0.0, 100.0))).into());
                            },
                            CameraState::Cinematic => {
                                app.camera.camera.up = rigidbody.rotation() * *Vector3::y_axis();
                                app.camera.projection.fovy = 60.0;
                                if let Some(cameras) = &player.instance.metadata.cameras {
                                    app.camera.camera.position = (*rigidbody.translation() + (*rigidbody.rotation() * cameras.cinematic_camera)).into();
                                }
                                app.camera.camera.look_at((rigidbody.translation() + (rigidbody.rotation() * Vector3::new(30.0, 0.0, 100.0))).into());
                            },
                            CameraState::Frontal => {
                                app.camera.camera.up = rigidbody.rotation() * *Vector3::y_axis();
                                app.camera.projection.fovy = 60.0;
                                if let Some(cameras) = &player.instance.metadata.cameras {
                                    app.camera.camera.position = (*rigidbody.translation() + (*rigidbody.rotation() * cameras.frontal_camera)).into();
                                }
                                app.camera.camera.look_at((*rigidbody.translation()).into());
                            },
                            CameraState::Free => {
                                app.camera.camera.up = *Vector3::y_axis();
                                app.camera.projection.fovy = 60.0;
                                app.camera.camera.yaw = -self.controller.mouse.x as f32;
                                app.camera.camera.pitch = self.controller.mouse.y as f32;

                                // Clamp pitch to avoid flipping over
                                let rotation_y = UnitQuaternion::from_axis_angle(&Vector3::y_axis(), app.camera.camera.yaw.to_radians());
                                let rotation_x = UnitQuaternion::from_axis_angle(&Vector3::x_axis(), app.camera.camera.pitch.to_radians());

                                // Combine rotations
                                self.camera_data.mod_quaternion = rotation_y * rotation_x;

                                app.camera.camera.position = ((self.camera_data.mod_quaternion * Vector3::new(0.0, 0.0, -50.0)) + rigidbody.translation()).into();
                                app.camera.camera.look_at((*rigidbody.translation()).into());
                            },
                        }
                    }
                },
                None => {
                    println!("the player dont have a physics data, check the data.ron from: {}", player.instance.id)
                },
            }
        }
        
        self.calculate_lockable(app);
        if self.controller.change_camera.released {
            self.next_camera(&mut app.camera);
        }
    }

    
    
    fn calculate_lockable(&mut self, app: &mut App) {
        let plane = &app.renderizable_instances.get("player").unwrap().instance;
        for lockable in &self.plane_systems.bandits {
            if lockable.locked && self.controller.fix_view.pressed && self.controller.fix_view.time_pressed > self.controller.fix_view_hold_window {
                match app.renderizable_instances.get(&lockable.tag) {
                    Some(look_at) => {
                        let look_at_position = look_at.instance.transform.position;
                        app.camera.camera.look_at(look_at_position.into());
                        match self.camera_data.camera_state {
                            CameraState::Normal => {
                                let plane_pos = plane.transform.position;
                                let direction = (look_at_position - plane_pos).normalize();
                                let rotation = UnitQuaternion::from_axis_angle(&Vector3::z_axis(), direction.angle(&Vector3::z()));
                                let pos = plane_pos + rotation * Vector3::new(0.0, 0.0, -50.0);
                                let final_pos = pos + plane.transform.rotation * Vector3::new(0.0, 20.0, 0.0);
                                app.camera.camera.position = Point3::from(final_pos);
                            },
                            CameraState::Cockpit => {
                                // Here we will get the actual rotation of the camera and get the angle of the actual plane
                                // so we can define a max value for the yaw and the pitch
                            },
                            CameraState::Cinematic => {},
                            CameraState::Frontal => {},
                            CameraState::Free => {},
                        }
                    },
                    None => {},
                }
            }
        }
    }

    // i do this so my "ui_controller" is smaller
    fn update_text_label(&mut self, ui_tagged_elements: &mut HashMap<String, UiNode>, tag: &str, text: &str, font_system: &mut FontSystem) {
        if let Some(framerate) = ui_tagged_elements.get_mut(tag) {
            match &mut framerate.content {
                UiNodeContent::Text(label) => {
                    label.set_text(font_system, &text, true);
                },
                _ => {}
            }
        }
    }

    fn ui_control(&mut self, app: &mut App, delta_time: f32) {
        if app.throttling.last_ui_update.elapsed() >= app.throttling.ui_update_interval {
            match app.ui.renderizable_elements.get_mut("static").unwrap() {
                UiContainer::Tagged(hash_map) => {
                    self.update_text_label(hash_map, "framerate", &format!("FPS: {}", app.time.get_fps()), &mut app.ui.text.font_system);
                    self.update_text_label(hash_map, "altitude", &format!("ALT: {}", self.plane_systems.flight_data.altimeter), &mut app.ui.text.font_system);
                    self.update_text_label(hash_map, "g_number", &format!("G: {:.0}", self.plane_systems.flight_data.g_meter), &mut app.ui.text.font_system);
                    self.update_text_label(hash_map, "speed", &format!("SPD: {:.0}", self.plane_systems.flight_data.speedometer), &mut app.ui.text.font_system);
        
                    let rotation = Self::map_to_range(app.camera.camera.yaw.into(), -PI as f64, PI  as f64, 0.0, 360.0).round();
                    
                    let text_compass = if rotation >= 355.0 || rotation <= 5.0 {
                        "N".to_owned()
                    } else if rotation >= 175.0 && rotation <= 185.0{
                        "S".to_owned()
                    } else if rotation >= 85.0 && rotation <= 95.0 {
                        "E".to_owned()
                    } else if rotation >= 265.0 && rotation <= 275.0 {
                        "O".to_owned()
                    } else {
                        rotation.round().to_string() + "°"
                    };
        
                    self.update_text_label(hash_map, "compass", &format!("{}", text_compass).to_string(), &mut app.ui.text.font_system);

                    
                    if let Some(altitude_alert) = hash_map.get_mut("altitude_alert") {
                        self.blinking_alert("altitude".to_owned(), altitude_alert, self.plane_systems.flight_data.altimeter < 1000.0, delta_time);
                    }
        
                    if let Some(stall_alert) = hash_map.get_mut("stall_alert") {
                        self.blinking_alert("stall".to_owned(), stall_alert, self.plane_systems.stall, delta_time);
                    }
                },
                _ => {},
            };

            /* 
            let plane_direction = self.final_rotation * Vector3::new(0.0, 0.0, 1000000.0);
            let plane_direction = app.camera.world_to_screen((plane_direction.x, plane_direction.y, plane_direction.z).into() , app.config.width, app.config.height);
            match plane_direction {
                Some(lock_pos) => {
                    app.dynamic_ui_components.get_mut("dynamic_static").unwrap()[0].rectangle.border_color = [0.0, 1.0, 0.0, 1.0];
                    app.dynamic_ui_components.get_mut("dynamic_static").unwrap()[0].rectangle.position.left = (lock_pos.x() as f32 - 1.0) as u32;
                    app.dynamic_ui_components.get_mut("dynamic_static").unwrap()[0].rectangle.position.right = (lock_pos.x() as f32 + 1.0)  as u32;
                    app.dynamic_ui_components.get_mut("dynamic_static").unwrap()[0].rectangle.position.top = (lock_pos.y() as f32 - 1.0) as u32;
                    app.dynamic_ui_components.get_mut("dynamic_static").unwrap()[0].rectangle.position.bottom = (lock_pos.y() as f32 + 1.0) as u32;
                },
                None => {
                    app.dynamic_ui_components.get_mut("dynamic_static").unwrap()[0].rectangle.border_color = [0.0, 1.0, 0.0, 0.0];
                },
            }
            */

            self.targeting_system(app);
            app.throttling.last_ui_update = Instant::now();
        }
    }

    fn blinking_alert(&mut self, blinking_alert: String ,blinkable: &mut UiNode, condition: bool, delta_time: f32) {
        let blinking_alert = self.blinking_alerts.get_mut(&blinking_alert).unwrap();

        blinking_alert.time_alert += delta_time;
        if condition {
            if blinking_alert.alert_state == false {
                if blinking_alert.time_alert > 0.5 {
                    blinking_alert.time_alert = 0.0;
                    blinking_alert.alert_state = true;
                }
            } else {
                if blinking_alert.time_alert > 0.5 {
                    blinking_alert.time_alert = 0.0;
                    blinking_alert.alert_state = false;
                }
            }
        } else {
            blinking_alert.time_alert = 0.0;
            blinking_alert.alert_state = false
        }

        
            match &mut blinkable.content {
                UiNodeContent::Text(label) => {
                    if blinking_alert.alert_state {
                        blinkable.visibility.border_color = [1.0, 0.0, 0.0, 1.0];
                        label.color = Color::rgba(255, 0, 0, 255);

                    } else {
                        blinkable.visibility.border_color = [0.0, 0.0, 0.0, 0.0];
                        label.color = Color::rgba(0, 0, 0, 0);
                    }
                },
                _ => {}
            }
        
    }

    fn targeting_system(&mut self, app: &mut App) {
        // we set the position of all the bandits
        let mut bandits_target = vec![];
        
        for markable in &self.plane_systems.bandits {
            match app.renderizable_instances.get(&markable.tag) {
                Some(bandit) => bandits_target.push(bandit.instance.transform.position),
                None => continue,
            }
        }

        let mut diff_magnitude = f32::MAX;
        let mut closest_index: usize = 0;

        match app.ui.renderizable_elements.get_mut("bandits").unwrap() {
            UiContainer::Tagged(hash_map) => todo!(),
            UiContainer::Untagged(vec) => {
                // self.load_needed_bandit_markers(&bandits_target, vec, &mut app.ui.text.font_system);

                // for each position of the bandit we set a screen position and move the button
                /* 
                for (i, bandit_position) in bandits_target.iter().enumerate() {
                    let target_position = app.camera.world_to_screen(Point3::new(bandit_position.x, bandit_position.y, bandit_position.z), app.config.width, app.config.height);

                    match target_position {
                        Some(lock_pos) => {
                            if !self.plane_systems.bandits[i].locked {
                                let screen_middle = Vector2::new((app.config.width / 2) as f32, (app.config.height / 2) as f32);
                                let target_pos = Vector2::new(lock_pos.x as f32, lock_pos.y as f32);
                                if (screen_middle - target_pos).magnitude() < diff_magnitude {
                                    closest_index = i;
                                    diff_magnitude = (screen_middle - target_pos).magnitude();
                                    self.camera_data.next_look_at = Some(*bandit_position);
                                }
                                // search for the closest element to the middle of the screen
                            }

                            vec[i].visibility.border_color = if self.plane_systems.bandits[i].locked { [0.0, 0.0, 1.0, 1.0] } else { [0.0, 1.0, 0.0, 1.0] };
                            vec[i].transform.rect.left = (lock_pos.x() - 20) as u32;
                            vec[i].transform.rect.right = (lock_pos.x() + 20)  as u32;
                            vec[i].transform.rect.top = (lock_pos.y() - 20) as u32;
                            vec[i].transform.rect.bottom = (lock_pos.y() + 20) as u32;
                        },
                        None => {
                            vec[i].visibility.border_color = [0.0, 1.0, 0.0, 0.0];
                        },
                    }
                }
                */

            },
        };

        
        
        if self.controller.fix_view.released && self.controller.fix_view.time_pressed < self.controller.fix_view_hold_window {
            for bandit in &mut self.plane_systems.bandits {
                if bandit.locked {
                    bandit.locked = false;
                } else {
                    continue;
                }
            }
            self.plane_systems.bandits[closest_index].locked = true;
            self.camera_data.look_at = self.camera_data.next_look_at;
        }

        
    }
    /* 
    fn load_needed_bandit_markers(&mut self, bandits_target: &Vec<Vector3<f32>>, bandit_list: &mut Vec<UiNode>, font_system: &mut FontSystem) {
        if self.plane_systems.bandits.len() != bandit_list.len() {
            let _ = &bandits_target.iter().for_each(|_target| {
                let crosshair = UiNode::new(transform, visibility, content_data, app)

                let crosshair: Button = Button::new(
                    ButtonConfig {
                        rect_pos: RectPos { top: 50, left: 50, bottom: 50, right: 50 },
                        fill_color: [0.0, 0.0, 0.0, 0.0],
                        fill_color_active: [0.0, 0.0, 0.0, 0.0],
                        border_color: [0.0, 1.0, 0.0, 1.0],
                        border_color_active: [0.0, 1.0, 0.0, 1.0],
                        text: "",
                        text_color: Color::rgba(0, 255, 0, 255),
                        text_color_active: Color::rgba(0, 0, 75, 000),
                        rotation: Quaternion::identity()
                    },
                    font_system,
                );
                bandit_list.push(crosshair);
            });       
        }
    }
    */

    fn map_to_range(x: f64, in_min: f64, in_max: f64, out_min: f64, out_max: f64) -> f64 {
        (x - in_min) * (out_max - out_min) / (in_max - in_min) + out_min
    }

    fn next_camera(&mut self, camera: &mut CameraRenderizable) {
        match self.camera_data.camera_state {
            CameraState::Normal => {
                self.camera_data.camera_state = CameraState::Free;
                camera.camera.yaw = 0.0;
                camera.camera.pitch = 0.0;
            },
            CameraState::Cockpit => self.camera_data.camera_state = CameraState::Cinematic,
            CameraState::Cinematic => self.camera_data.camera_state = CameraState::Frontal,
            CameraState::Frontal => self.camera_data.camera_state = CameraState::Normal,
            CameraState::Free => self.camera_data.camera_state = CameraState::Cockpit,

        }
    }
}