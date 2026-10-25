use wgpu::{Device, SurfaceConfiguration};
use std::thread;
use std::sync::mpsc::{channel, Sender, Receiver};
use std::collections::HashMap;
use std::rc::Rc;
use std::panic;
use nalgebra::Point3;

use crate::rendering::ui::physics_rendering::RenderPhysics;
use crate::rendering::camera::handler::CameraResources;
use crate::physics::physics_handler::{Physics, RenderMessage, PhysicsCommand};
use crate::physics::physics_behavior::{PhysicsBehavior, PhysicsInput, PhysicsPayload};
use crate::physics::physics_resources::{load_physics_from_definitions, PhysicsObjectDef};
use crate::primitive::manual_vertex::ManualVertex;

#[derive(Clone)]
pub enum DebugPhysicsMessageType {
    /// A world-space line. `owner` is the node whose body it was drawn on,
    /// so the render thread can carry it forward with that body's
    /// extrapolated pose (see `physics_pose_now`). `None` stays where it
    /// was drawn.
    RenderizableLines { owner: Option<String>, line: [ManualVertex; 2] },
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

/// The main thread's side of the open scene's physics thread - `app.physics`.
/// Talks to the thread without ever waiting on it, and keeps what it last
/// heard from it.
pub struct PhysicsState {
    /// The channels to the thread - None while the open scene has no physics.
    thread: Option<PhysicsDataTransmission>,
    /// Every body's latest pose from the thread - kept, so a frame where no
    /// new one has arrived still draws each body where it is by now (see
    /// `physics_pose_now`) instead of freezing it for a frame. Rc, like
    /// `debug_lines`: shared with the frame's FrameContext, not copied.
    pub(crate) poses: Rc<HashMap<String, RenderMessage>>,
    /// The thread's latest debug lines - kept like `poses`.
    pub(crate) debug_lines: Rc<Vec<DebugPhysicsMessageType>>,
    /// The gravity and time scale last sent to the thread, so they're sent
    /// again only when they change. Not the values themselves - gravity is
    /// in the settings, the time scale in `app.time`.
    sent_gravity: Option<nalgebra::Vector3<f32>>,
    sent_time_scale: f32,
}

impl PhysicsState {
    pub(crate) fn new() -> Self {
        Self { thread: None, poses: Rc::default(), debug_lines: Rc::default(), sent_gravity: None, sent_time_scale: 1.0 }
    }

    /// Whether the open scene's physics thread is running.
    pub fn is_running(&self) -> bool {
        self.thread.is_some()
    }

    /// The body `id`'s latest pose from the thread, as it was sampled (not
    /// moved on to now).
    pub fn pose(&self, id: &str) -> Option<&RenderMessage> {
        self.poses.get(id)
    }

    /// Takes over a scene's just-started thread - which starts with the
    /// scene's `gravity`, at normal speed.
    pub(crate) fn start(&mut self, thread: Option<PhysicsDataTransmission>, gravity: nalgebra::Vector3<f32>) {
        self.thread = thread;
        self.sent_gravity = Some(gravity);
        self.sent_time_scale = 1.0;
    }

    /// Shuts the thread down, if there's one, and forgets what it sent.
    pub(crate) fn stop(&mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.request_data_tx.send(PhysicsCommand::Shutdown);
        }
        self.poses = Rc::default();
        self.debug_lines = Rc::default();
    }

    /// Where commands for the thread go - None while there's no thread.
    pub(crate) fn command_tx(&self) -> Option<&Sender<PhysicsCommand>> {
        self.thread.as_ref().map(|thread| &thread.request_data_tx)
    }

    /// Sends `gravity` and `time_scale` to the thread when they aren't what
    /// it was last sent.
    pub(crate) fn sync(&mut self, gravity: nalgebra::Vector3<f32>, time_scale: f32) {
        let Some(thread) = &self.thread else { return };
        if self.sent_gravity != Some(gravity) {
            let _ = thread.request_data_tx.send(PhysicsCommand::SetGravity(gravity));
            self.sent_gravity = Some(gravity);
        }
        if self.sent_time_scale != time_scale {
            let _ = thread.request_data_tx.send(PhysicsCommand::SetTimeScale(time_scale));
            self.sent_time_scale = time_scale;
        }
    }

    /// Asks the thread for its data (with the debug lines when
    /// `debug_lines`), and takes whatever it has sent since last frame -
    /// only the newest poses and lines are kept, older ones would lag
    /// behind. Returns this frame's batch of poses (empty when none came).
    pub(crate) fn receive(&mut self, debug_lines: bool) -> Rc<HashMap<String, RenderMessage>> {
        let Some(thread) = &self.thread else { return Rc::default() };
        if let Err(e) = thread.request_data_tx.send(PhysicsCommand::RequestData { debug_lines }) {
            eprintln!("Failed to send physics command: {}", e);
        }

        let mut physics_data = HashMap::new();
        while let Ok(data) = thread.physics_data_rx.try_recv() {
            physics_data = data;
        }
        let physics_data = Rc::new(physics_data);
        if !physics_data.is_empty() {
            self.poses = Rc::clone(&physics_data);
        }
        // The last lines stay up on a frame where none arrived, instead of
        // flickering off for a frame.
        while let Ok(data) = thread.debug_physics_rx.try_recv() {
            self.debug_lines = Rc::new(data);
        }
        physics_data
    }

    /// Every batch of states the nodes' physics halves published since last
    /// frame, oldest first.
    pub(crate) fn take_node_states(&self) -> Vec<HashMap<String, Vec<PhysicsPayload>>> {
        self.thread.as_ref().map_or_else(Vec::new, |thread| thread.node_state_rx.try_iter().collect())
    }

    /// What behaviors wrote for their physics halves this frame, as one
    /// batch - nothing is sent when it's empty or there's no thread.
    pub(crate) fn send_inputs(&self, inputs: HashMap<String, PhysicsInput>) {
        if let (Some(thread), false) = (&self.thread, inputs.is_empty()) {
            if let Err(e) = thread.node_input_tx.send(inputs) {
                eprintln!("Failed to send node physics inputs: {}", e);
            }
        }
    }
}

// Always starts the physics thread - callers only call this when the scene
// actually wants physics; see Scene::start_physics. `gravity` and
// `fixed_timestep` are what the scene runs with (see settings).
pub fn physics_handling(device: &Device, config: &SurfaceConfiguration, camera: &CameraResources, physics_bodies: Vec<PhysicsObjectDef>, node_behaviors: Vec<(String, Vec<Box<dyn PhysicsBehavior>>)>, gravity: nalgebra::Vector3<f32>, fixed_timestep: f32) -> PhysicsDataTransmission {
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

        let mut physics = Physics::new(gravity);
        load_physics_from_definitions(&physics_bodies, &mut physics.collider_set, &mut physics.rigidbody_set, &mut physics.physics_elements);
        physics.physics_thread(fixed_timestep, physics_data_tx, request_data_rx, debug_physics_tx, node_behaviors, node_input_rx, node_state_tx);
    });

    return PhysicsDataTransmission {
        physics_data_rx, // Physics data for representation
        request_data_tx, // Transmisor to requesat data from the physics thread
        debug_physics_rx, // Receiver to receive debug physics messages
        node_input_tx, // Transmisor to send node inputs/events to their physics halves
        node_state_rx, // Receiver to receive states published by node physics halves
    };
}

