use wgpu::{Device, SurfaceConfiguration};
use std::thread;
use std::sync::mpsc::{channel, Sender, Receiver};
use std::collections::HashMap;
use std::panic;
use nalgebra::Point3;

use crate::engine::rendering::ui::physics_rendering::RenderPhysics;
use crate::engine::rendering::camera::handler::CameraResources;
use crate::engine::physics::physics_handler::{Physics, RenderMessage, PhysicsCommand};
use crate::engine::physics::physics_behavior::{PhysicsBehavior, PhysicsInput, PhysicsPayload};
use crate::engine::physics::physics_resources::{load_physics_from_definitions, PhysicsObjectDef};
use crate::engine::primitive::manual_vertex::ManualVertex;

#[derive(Clone)]
pub enum DebugPhysicsMessageType {
    RenderizableLines([ManualVertex; 2]),
    RenderizablePoint(Point3<f32>),
}

pub struct PhysicsDataTransmission {
    pub physics_data_rx: Receiver<HashMap<String, RenderMessage>>,
    pub request_data_tx: Sender<PhysicsCommand>,
    pub debug_physics_rx: Receiver<Vec<DebugPhysicsMessageType>>,
    // Node-level physics halves (see PhysicsBehavior): input batches going
    // in, published states coming back, both keyed by node id.
    pub node_input_tx: Sender<HashMap<String, PhysicsInput>>,
    pub node_state_rx: Receiver<HashMap<String, Vec<PhysicsPayload>>>,
}

// Always starts the physics thread - callers only call this when the scene
// actually wants physics; see Scene::start_physics.
pub fn physics_handling(device: &Device, config: &SurfaceConfiguration, camera: &CameraResources, physics_bodies: Vec<PhysicsObjectDef>, node_behaviors: Vec<(String, Vec<Box<dyn PhysicsBehavior>>)>) -> PhysicsDataTransmission {
    // Data channels
    let (physics_data_tx, physics_data_rx) = channel::<HashMap<String, RenderMessage>>();
    let (request_data_tx, request_data_rx) = channel::<PhysicsCommand>();

    
    let (debug_physics_tx, debug_physics_rx) = channel::<Vec<DebugPhysicsMessageType>>();

    let (node_input_tx, node_input_rx) = channel::<HashMap<String, PhysicsInput>>();
    let (node_state_tx, node_state_rx) = channel::<HashMap<String, Vec<PhysicsPayload>>>();

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
        load_physics_from_definitions(&physics_bodies, &mut physics.collider_set, &mut physics.rigidbody_set, &mut physics.physics_elements);
        physics.physics_thread(physics_data_tx, request_data_rx, debug_physics_tx, node_behaviors, node_input_rx, node_state_tx);
    });

    return PhysicsDataTransmission {
        physics_data_rx, // Physics data for representation
        request_data_tx, // Transmisor to requesat data from the physics thread
        debug_physics_rx, // Receiver to receive debug physics messages
        node_input_tx, // Transmisor to send node inputs/events to their physics halves
        node_state_rx, // Receiver to receive states published by node physics halves
    };
}

