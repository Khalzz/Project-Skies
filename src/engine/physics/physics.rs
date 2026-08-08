use wgpu::{Device, SurfaceConfiguration};
use std::thread;
use std::sync::mpsc::{channel, Sender, Receiver};
use std::collections::HashMap;
use std::panic;
use nalgebra::Point3;

use crate::engine::rendering::ui::physics_rendering::RenderPhysics;
use crate::engine::rendering::camera::CameraHandler;
use crate::engine::physics::physics_handler::{Physics, RenderMessage, PhysicsCommand, PhysicsTick};
use crate::engine::physics::physics_resources::load_physics_from_level;
use crate::game::play::plane::plane::PlaneControls;
use crate::engine::primitive::manual_vertex::ManualVertex;

#[derive(Clone)]
pub enum DebugPhysicsMessageType {
    RenderizableLines([ManualVertex; 2]),
    RenderizablePoint(Point3<f32>),
}

pub struct PhysicsDataTransmission {
    pub physics_data_rx: Receiver<HashMap<String, RenderMessage>>,
    pub request_data_tx: Sender<PhysicsCommand>,
    pub plane_control_tx: Sender<PlaneControls>,
    pub debug_physics_rx: Receiver<Vec<DebugPhysicsMessageType>>,
}

// Always starts the physics thread - callers only call this when a scene's
// Scene::physics() actually returned Some(...); see App::run.
pub fn physics_handling(device: &Device, config: &SurfaceConfiguration, camera: &CameraHandler, level_path: String, physics_tick: Box<dyn PhysicsTick + Send>) -> PhysicsDataTransmission {
    // Data channels
    let (physics_data_tx, physics_data_rx) = channel::<HashMap<String, RenderMessage>>();
    let (request_data_tx, request_data_rx) = channel::<PhysicsCommand>();

    let (plane_control_tx, plane_control_rx) = channel::<PlaneControls>();
    
    let (debug_physics_tx, debug_physics_rx) = channel::<Vec<DebugPhysicsMessageType>>();

    let render_physics = RenderPhysics::new(&device, &config, &camera);

    thread::spawn(move || {
        // Set a custom panic hook to print detailed error info
        panic::set_hook(Box::new(|panic_info| {
            eprintln!("=== PHYSICS THREAD PANIC ===");
            if let Some(location) = panic_info.location() {
                eprintln!("Panic at {}:{}:{}", location.file(), location.line(), location.column());
            }
            if let Some(message) = panic_info.payload().downcast_ref::<&str>() {
                eprintln!("Message: {}", message);
            } else if let Some(message) = panic_info.payload().downcast_ref::<String>() {
                eprintln!("Message: {}", message);
            }
            eprintln!("============================");
        }));

        let mut physics = Physics::new();
        load_physics_from_level(level_path, &mut physics.collider_set, &mut physics.rigidbody_set, &mut physics.physics_elements);
        physics.physics_thread(physics_data_tx, request_data_rx, plane_control_rx, debug_physics_tx, physics_tick);
    });

    return PhysicsDataTransmission {
        physics_data_rx, // Physics data for representation
        request_data_tx, // Transmisor to requesat data from the physics thread
        plane_control_tx, // Transmisor to send plane controls to the physics thread
        debug_physics_rx, // Receiver to receive debug physics messages
    };
}

