use std::{collections::HashMap, f32::consts::PI, hash::Hash, time::{Duration, Instant}};

use glyphon::{cosmic_text::Align, Color, FontSystem};
use nalgebra::{vector, Point3, Quaternion, UnitQuaternion, Vector3};
use rand::{rngs::ThreadRng, Rng};
use rapier3d::prelude::RigidBody;
use sdl2::{controller::GameController};
use crate::{app::{App, AppState}, audio::subtitles::Subtitle, input::{input::InputSubsystem, utils::to_axis}, physics::physics_handler::{MetadataType, PhysicsData, RenderMessage}, rendering::{camera::CameraRenderizable, ui::UiContainer}, transform::Transform, ui::{ui_node::{ChildrenType, UiNode, UiNodeContent, UiNodeParameters, Visibility}, ui_transform::UiTransform}, utils::lerps::{lerp, lerp_quaternion}};
use super::{airfoil::AirFoil, event_handling::EventSystem, plane::plane::Plane, wheel::Wheel, wing::Wing};
use std::sync::mpsc::Sender;
use crate::gameplay::plane::plane::PlaneControls;

// Add a way of setting timing that can be agnostic to real time (or that will not be affected by the player pausing)
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
    pub mod_quaternion: UnitQuaternion<f32>,
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
    pub previous_velocity: Option<Vector3<f32>>,
}

pub struct GameLogic { // here we define the data we use on our script
    pub camera_data: CameraData,
    pub blinking_alerts: HashMap<String, BlinkingAlert>,
    pub plane_systems: PlaneSystems,
    pub gravity: Vector3<f32>,
    pub subtitle_data: Subtitle,
    pub start_time: Instant,
    pub event_system: Option<EventSystem>,
    rng: ThreadRng,
    pub game_time: f64,
    pub plane: Plane,
} 

impl GameLogic {
    // this is called once
    pub fn new(app: &mut App) -> Self {
        // UI ELEMENTS AND LIST
        let altitude = UiNode::new(
            UiTransform::new(((app.config.width as f32 / 2.0) - (150.0 / 2.0)) - 400.0, (app.config.height as f32 / 2.0) - (30.0 / 2.0), 30.0, 150.0, 0.0, false), 
            Visibility::new([0.0, 0.0, 0.0, 0.0], [0.0, 255.0, 0.0, 255.0]),
            UiNodeParameters::Text { text: "ALT", color: Color::rgba(0, 255, 75, 255), align: Align::Center, font_size: 20.0}, 
            app,
        );

        let speed = UiNode::new(
            UiTransform::new(((app.config.width as f32 / 2.0) - (150.0 / 2.0)) + 400.0, (app.config.height as f32 / 2.0) - (30.0 / 2.0), 30.0, 150.0, 0.0, false), 
            Visibility::new([0.0, 0.0, 0.0, 0.0], [0.0, 255.0, 0.0, 255.0]),
            UiNodeParameters::Text { text: "SPD", color: Color::rgba(0, 255, 75, 255), align: Align::Center, font_size: 20.0}, 
            app,
        );
        
        let altitude_alert = UiNode::new(
            UiTransform::new((app.config.width as f32 / 2.0) - (140.0 / 2.0), ((app.config.height as f32 / 2.0) - (50.0 / 2.0)) + 50.0, 50.0, 140.0, 0.0, false), 
            Visibility::new([0.0, 0.0, 0.0, 0.0], [255.0, 0.0, 0.0, 255.0]),
            UiNodeParameters::Text { text: "ALT", color: Color::rgba(0, 255, 75, 255), align: Align::Center, font_size: 20.0 }, 
            app,
        );

        let compass = UiNode::new(
            UiTransform::new((app.config.width as f32 / 2.0) - (100.0 / 2.0), 300.0, 50.0, 100.0, 0.0, false), 
            Visibility::new([0.0, 0.0, 0.0, 0.0], [0.0, 255.0, 0.0, 255.0]),
            UiNodeParameters::Text { text: "90°", color: Color::rgba(0, 255, 75, 255), align: Align::Center, font_size: 20.0 }, 
            app,
        );

        let timer = UiNode::new(
            UiTransform::new(10.0, 10.0, 30.0, 100.0, 0.0, false), 
            Visibility::new([0.0, 0.0, 0.0, 0.0], [0.0, 255.0, 0.0, 255.0]),
            UiNodeParameters::Text { text: "00:00:000", color: Color::rgba(0, 255, 75, 255), align: Align::Center, font_size: 20.0}, 
            app,
        );

        let framerate = UiNode::new(
            UiTransform::new(10.0, 10.0, 30.0, 100.0, 0.0, false), 
            Visibility::new([0.0, 0.0, 0.0, 0.0], [0.0, 255.0, 0.0, 255.0]),
            UiNodeParameters::Text { text: "90 fps", color: Color::rgba(0, 255, 75, 255), align: Align::Center, font_size: 20.0}, 
            app,
        );

        let g_number = UiNode::new(
            UiTransform::new(10.0, 50.0, 30.0, 100.0, 0.0, false), 
            Visibility::new([0.0, 0.0, 0.0, 0.0], [0.0, 255.0, 0.0, 255.0]),
            UiNodeParameters::Text { text: "G", color: Color::rgba(0, 255, 75, 255), align: Align::Center, font_size: 20.0}, 
            app,
        );

        let throttle_value = UiNode::new(
            UiTransform::new(10.0, 50.0, 30.0, 100.0, 0.0, false), 
            Visibility::new([0.0, 0.0, 0.0, 0.0], [0.0, 255.0, 0.0, 255.0]),
            UiNodeParameters::Text { text: "0%", color: Color::rgba(0, 255, 75, 255), align: Align::Center, font_size: 20.0}, 
            app,
        );

        let mut game_info = UiNode::new(
            UiTransform::new(10.0, 10.0, 0.0, 150.0, 0.0, false), 
            Visibility::new([0.0, 0.0, 0.0, 0.7], [0.0, 0.0, 0.0, 0.0]),
            UiNodeParameters::VerticalContainerData { margin: 10.0, separation: 10.0, children: ChildrenType::MappedChildren(HashMap::new()) }, 
            app,
        );

        game_info.add_children("framerate".to_owned(), framerate);
        game_info.add_children("g_number".to_owned(), g_number);
        game_info.add_children("timer".to_owned(), timer);
        game_info.add_children("throttle_value".to_owned(), throttle_value);

        let subtitle = UiNode::new(
            UiTransform::new((app.config.width as f32 / 2.0) - (app.config.width as f32 * 0.9) / 2.0, app.config.height as f32 * 0.7, 0.0, app.config.width as f32 * 0.9, 0.0, true), 
            Visibility::new([0.0, 0.0, 0.0, 0.7], [0.0, 0.0, 0.0, 0.0]),
            UiNodeParameters::VerticalContainerData { margin: 10.0, separation: 10.0, children: ChildrenType::IndexedChildren(vec![]) }, 
            app,
        );
        

        app.ui.renderizable_elements.clear();
        app.ui.renderizable_elements.insert("static".to_owned(), UiContainer::Tagged(HashMap::new()));
        app.ui.renderizable_elements.insert("bandits".to_owned(), UiContainer::Untagged(vec![]));

        app.ui.add_to_ui("static".to_owned(), "altitude".to_owned(), altitude);

        app.ui.add_to_ui("static".to_owned(), "speed".to_owned(), speed);
        app.ui.add_to_ui("static".to_owned(), "compass".to_owned(), compass);
        app.ui.add_to_ui("static".to_owned(), "altitude_alert".to_owned(), altitude_alert);
        app.ui.add_to_ui("static".to_owned(), "subtitles".to_owned(), subtitle);
        app.ui.add_to_ui("static".to_owned(), "game_info".to_owned(),game_info);

        let subtitle_data = Subtitle::new();

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

        let plane_systems = PlaneSystems {
            bandits: vec![tower, tower2, crane, fellow],
            stall: false,
            afterburner_value: 0.0,
            base_rotations: BaseRotations { left_aleron: None, right_aleron: None },
            flap_ratio: 0.0,
            previous_velocity: None,
            flight_data: FlightData { altimeter: 0.0, speedometer: 0.0, g_meter: 0.0 }
        };

        let rng = rand::thread_rng();

        let mut blinking_alerts: HashMap<String, BlinkingAlert> = HashMap::new();
        blinking_alerts.insert("altitude".to_owned(), BlinkingAlert { alert_state: false, time_alert: 0.0 });
        blinking_alerts.insert("stall".to_owned(), BlinkingAlert { alert_state: false, time_alert: 0.0 });

        let gravity = vector![0.0, -9.81, 0.0];

        let event_system = match EventSystem::new(&app.scene_openned) {
            Ok(system) => Some(system),
            Err(error) => {
                eprintln!("Error: {}", error);
                None
            },
        };

        Self {
            camera_data,
            blinking_alerts,
            plane_systems,
            rng,
            gravity,
            start_time: Instant::now(),
            event_system,
            subtitle_data,
            game_time: 0.0,
            plane: Plane::new(),
        }
    }

    // this is called every frame
    pub fn update(&mut self, app: &mut App, input_subsystem: &InputSubsystem, plane_control_tx: &Sender<PlaneControls>, physics_data: &HashMap<String, RenderMessage>) {
        self.game_time += app.time.delta_time as f64;

        if input_subsystem.is_just_pressed("test") {
            self.subtitle_data.add_text(&"SKIBIDI DAM DAM DAM YES YES".to_string(), app);
        }

        self.plane.update(app.time.delta_time, input_subsystem);
        plane_control_tx.send(self.plane.controls.clone());

        self.plane_movement(app, app.time.delta_time, physics_data);
        self.subtitle_data.update(app);
        self.camera_control(app, app.time.delta_time, input_subsystem);
        self.ui_control(app, app.time.delta_time);
    }

    fn plane_movement (&mut self, app: &mut App, delta_time: f32, physics_data: &HashMap<String, RenderMessage>) {
        let plane = app.renderizable_instances.get_mut("player").unwrap();
        let physics_data_renderizable = physics_data.get("player");
        let plane_model = app.game_models.get_mut(&plane.model_ref).unwrap();

        // elevators
        if let Some(meshes) = plane_model.model.mesh_lists.get_mut("opaque") {
            match physics_data_renderizable {
                Some(physics_data_renderizable) => {
                    if let Some(wheels) = physics_data_renderizable.metadata.get("wheels") {
                        match &wheels {
                            MetadataType::Wheels(wheels) => {
                                for (index, wheel) in wheels.iter() {
                                    if let Some(wheel_mesh) = &mut meshes.get_mut(index.as_str()) {
                                        let final_pos =  plane.instance.transform.rotation.inverse() * (wheel.wheel_position - plane.instance.transform.position);
                                        wheel_mesh.transform.position = Vector3::new(final_pos.x / plane.instance.transform.scale.x, final_pos.y / plane.instance.transform.scale.y, final_pos.z / plane.instance.transform.scale.z);
                                        wheel_mesh.update_transform(&app.queue);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                },
                None => {}
            }

            if let Some(elevator) = meshes.get_mut("left_elevator") {
                let final_rotation = UnitQuaternion::from_axis_angle(&Vector3::x_axis() ,0.15 * -self.plane.controls.elevator);
                let elevator_rotation = lerp_quaternion(elevator.transform.rotation,  *final_rotation, app.time.delta_time * 7.0);
                let elevator_transform = Transform::new(elevator.transform.position, elevator_rotation, elevator.transform.scale);
                elevator.change_transform(&app.queue, elevator_transform);
            }
    
            if let Some(elevator) = meshes.get_mut("right_elevator") {
                let final_rotation = UnitQuaternion::from_axis_angle(&Vector3::x_axis() ,0.15 * -self.plane.controls.elevator);
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

            if let Some(aleron) = meshes.get_mut("left_aleron") {
                match self.plane_systems.base_rotations.left_aleron {
                    Some(base_rotation) => {
                        let dependent = UnitQuaternion::from_quaternion(base_rotation.clone()) * UnitQuaternion::from_axis_angle(&Vector3::x_axis() ,0.5 * -self.plane.controls.aileron);
                        let aleron_rotation = lerp_quaternion(aleron.transform.rotation,  *dependent, app.time.delta_time * 7.0);
                        let aleron_transform = Transform::new(aleron.transform.position, aleron_rotation, aleron.transform.scale);
                        aleron.change_transform(&app.queue, aleron_transform);
                    },
                    None => {
                        self.plane_systems.base_rotations.left_aleron = Some(aleron.transform.rotation);
                    },
                }
            }

            if let Some(aleron) = meshes.get_mut("right_aleron") {
                match self.plane_systems.base_rotations.right_aleron {
                    Some(base_rotation) => {
                        let dependent = UnitQuaternion::from_quaternion(base_rotation.clone()) * UnitQuaternion::from_axis_angle(&Vector3::x_axis(), 0.5 * self.plane.controls.aileron);
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
            if let Some(rudder) = meshes.get_mut("rudder_0") {
                let rudder_rotation = lerp_quaternion(rudder.transform.rotation, *UnitQuaternion::from_axis_angle(&Vector3::x_axis(),-28.4493 * PI / 180.0) * *UnitQuaternion::from_axis_angle(&Vector3::y_axis(),0.5 * self.plane.controls.rudder), delta_time * 7.0);
                let rudder_transform = Transform::new(rudder.transform.position, rudder_rotation, rudder.transform.scale);
                rudder.change_transform(&app.queue, rudder_transform);
            }

            // right rudder if it haves 2
            if let Some(rudder) = meshes.get_mut("rudder_1") {
                let rudder_rotation = lerp_quaternion(rudder.transform.rotation, *UnitQuaternion::from_axis_angle(&Vector3::x_axis(),-28.4493 * PI / 180.0) * *UnitQuaternion::from_axis_angle(&Vector3::y_axis(),0.5 * self.plane.controls.rudder), delta_time * 7.0);
                let rudder_transform = Transform::new(rudder.transform.position, rudder_rotation, rudder.transform.scale);
                rudder.change_transform(&app.queue, rudder_transform);
            }
        }

        if let Some(meshes) = plane_model.model.mesh_lists.get_mut("transparent") {
            if let Some(afterburner) = meshes.get_mut("Afterburner") {
                if self.plane.controls.throttle > 0.0 {
                    self.plane_systems.afterburner_value =  lerp(self.plane_systems.afterburner_value, self.plane.controls.throttle + self.rng.gen_range(-0.5..0.5), app.time.delta_time * 20.0);
                } else {
                    self.plane_systems.afterburner_value = lerp(self.plane_systems.afterburner_value, 0.0, delta_time * 2.0)
                }

                afterburner.change_transform(&app.queue, Transform::new(afterburner.transform.position, afterburner.transform.rotation, Vector3::new(1.0, 1.0, self.plane_systems.afterburner_value)));
            } 
        }
    }

    fn camera_control(&mut self, app: &mut App, delta_time: f32, input_subsystem: &InputSubsystem) {
        if let Some(player) = app.renderizable_instances.get_mut("player") {
            // Calculate target camera position and look-at point
            let (target_position, target_look_at, target_up) = match self.camera_data.camera_state {
                CameraState::Normal => {
                    let target_pos = player.instance.transform.position + (player.instance.transform.rotation * Vector3::new(0.0, 7.0, -28.0));
                    let look_at = player.instance.transform.position + (player.instance.transform.rotation * Vector3::new(0.0, 0.0, 100.0));
                    (target_pos, look_at, player.instance.transform.rotation * *Vector3::y_axis())
                },
                CameraState::Cockpit => {
                    app.camera.projection.fovy = 70.0;
                    let target_pos = if let Some(cameras) = &player.instance.metadata.cameras {
                        player.instance.transform.position + (player.instance.transform.rotation * cameras.cockpit_camera)
                    } else {
                        player.instance.transform.position + (player.instance.transform.rotation * Vector3::new(0.0, 1.8, 13.5))
                    };
                    let look_at = player.instance.transform.position + (player.instance.transform.rotation * Vector3::new(0.0, 0.0, 100.0));
                    (target_pos, look_at, player.instance.transform.rotation * *Vector3::y_axis())
                },
                CameraState::Cinematic => {
                    app.camera.projection.fovy = 60.0;
                    let target_pos = if let Some(cameras) = &player.instance.metadata.cameras {
                        player.instance.transform.position + (player.instance.transform.rotation * cameras.cinematic_camera)
                    } else {
                        player.instance.transform.position + (player.instance.transform.rotation * Vector3::new(-10.0, 3.0, -5.0))
                    };
                    let look_at = player.instance.transform.position + (player.instance.transform.rotation * Vector3::new(30.0, 0.0, 100.0));
                    (target_pos, look_at, player.instance.transform.rotation * *Vector3::y_axis())
                },
                CameraState::Frontal => {
                    app.camera.projection.fovy = 60.0;
                    let target_pos = if let Some(cameras) = &player.instance.metadata.cameras {
                        player.instance.transform.position + (player.instance.transform.rotation * cameras.frontal_camera)
                    } else {
                        player.instance.transform.position + (player.instance.transform.rotation * Vector3::new(0.0, 6.0, 30.0))
                    };
                    let look_at = player.instance.transform.position;
                    (target_pos, look_at, player.instance.transform.rotation * *Vector3::y_axis())
                },
                CameraState::Free => {
                    app.camera.projection.fovy = 60.0;
                    
                    app.camera.camera.yaw = -input_subsystem.mouse.get_x() as f32 * input_subsystem.mouse.get_sensitivity().0;
                    app.camera.camera.pitch = input_subsystem.mouse.get_y() as f32 * input_subsystem.mouse.get_sensitivity().1;

                    // Clamp pitch to avoid flipping over
                    let rotation_y = UnitQuaternion::from_axis_angle(&Vector3::y_axis(), app.camera.camera.yaw.to_radians());
                    let rotation_x = UnitQuaternion::from_axis_angle(&Vector3::x_axis(), app.camera.camera.pitch.to_radians());

                    // Combine rotations
                    self.camera_data.mod_quaternion = rotation_y * rotation_x;

                    let target_pos = (self.camera_data.mod_quaternion * Vector3::new(0.0, 0.0, -50.0)) + player.instance.transform.position;
                    let look_at = player.instance.transform.position;
                    (target_pos, look_at, *Vector3::y_axis())
                },
            };

            // Apply camera position directly (no interpolation to match object movement)
            app.camera.camera.position = target_position.into();
            app.camera.camera.look_at(target_look_at.into());
            app.camera.camera.up = target_up;
        }
        // self.calculate_lockable(app);
        if input_subsystem.is_just_pressed("change_camera") {
            self.next_camera(&mut app.camera);
        }
    }

    // i do this so my "ui_controller" is smaller
    fn update_text_label(&mut self, ui_tagged_elements: &mut HashMap<String, UiNode>, tag: &str, text: &str, font_system: &mut FontSystem) {
        if let Some(node) = ui_tagged_elements.get_mut(tag) {
            match &mut node.content {
                UiNodeContent::Text(label) => {
                    label.set_text(font_system, &text, true);
                },
                _ => {}
            }
        }
    }

    fn format_duration(seconds: f64) -> String {
        let duration = Duration::from_secs_f64(seconds);

        let total_millis = duration.as_millis() as u64;
        let hours = total_millis / 3_600_000; // 1 hour = 3,600,000 milliseconds
        let minutes = (total_millis % 3_600_000) / 60_000; // 1 minute = 60,000 milliseconds
        let seconds = (total_millis % 60_000) / 1_000; // 1 second = 1,000 milliseconds
        let milliseconds = total_millis % 1_000; // Remaining milliseconds
        // Format as hh:mm:ss:milmilmil
        format!("{:02}:{:02}:{:02}:{:03}", hours, minutes, seconds, milliseconds)
    }

    fn ui_control(&mut self, app: &mut App, delta_time: f32) {
        if app.throttling.last_ui_update.elapsed() >= app.throttling.ui_update_interval {
            match app.ui.renderizable_elements.get_mut("static").unwrap() {
                UiContainer::Tagged(hash_map) => {
                    match hash_map.get_mut("game_info") {
                        Some(info) => {
                            match info.get_container_hashed() {
                                Ok(map) => {
                                    self.update_text_label(map, "framerate", &format!("FPS: {}", app.time.get_fps()), &mut app.ui.text.font_system);
                                    self.update_text_label(map, "g_number", &format!("G: {:.0}", self.plane_systems.flight_data.g_meter), &mut app.ui.text.font_system);
                                    self.update_text_label(map, "timer", &Self::format_duration(self.game_time), &mut app.ui.text.font_system);
                                    self.update_text_label(map, "throttle_value", &format!("Power: {}%", (self.plane.controls.throttle * 100.0).round()), &mut app.ui.text.font_system);
                                },
                                Err(_) => todo!(),
                            }
                        },
                        None => {},
                    }

                    self.update_text_label(hash_map, "altitude", &format!("ALT: {}", self.plane_systems.flight_data.altimeter), &mut app.ui.text.font_system);
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

            app.ui.has_changed = true; // Mark UI as changed so it gets processed
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

    fn map_to_range(x: f64, in_min: f64, in_max: f64, out_min: f64, out_max: f64) -> f64 {
        (x - in_min) * (out_max - out_min) / (in_max - in_min) + out_min
    }

    fn next_camera(&mut self, camera: &mut CameraRenderizable) {
        match self.camera_data.camera_state {
            CameraState::Normal => {
                self.camera_data.camera_state = CameraState::Free;
            },
            CameraState::Cockpit => self.camera_data.camera_state = CameraState::Cinematic,
            CameraState::Cinematic => self.camera_data.camera_state = CameraState::Frontal,
            CameraState::Frontal => self.camera_data.camera_state = CameraState::Normal,
            CameraState::Free => self.camera_data.camera_state = CameraState::Cockpit,

        }
    }
}