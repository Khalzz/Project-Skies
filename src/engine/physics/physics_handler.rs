use rapier3d::prelude::{CCDSolver, ColliderSet, CollisionPipeline, DefaultBroadPhase, ImpulseJointSet, IntegrationParameters, IslandManager, MultibodyJointSet, NarrowPhase, PhysicsPipeline, QueryPipeline, RigidBodySet};
use nalgebra:: {Quaternion, Vector3};
use std::collections::HashMap;
use rapier3d::prelude::{ColliderHandle, RigidBodyHandle};
use std::sync::mpsc::{Sender, Receiver};
use std::thread;
use std::time::{Duration, Instant};
use serde::{Deserialize, Serialize};
use crate::engine::physics::physics::DebugPhysicsMessageType;
use crate::engine::physics::physics_behavior::{DebugDraw, PhysicsBehavior, PhysicsInput, PhysicsNode, PhysicsPayload};

pub struct RenderMessage {
    pub translation: Vector3<f32>,
    pub rotation: Quaternion<f32>,
    pub linvel: Vector3<f32>,
    // World-space angular velocity (rad/s) - rapier's own RigidBody::angvel,
    // straight through with no conversion. See Instrumentation::update
    // for the body-frame rotation that turns this into pitch/yaw/roll rate.
    pub angvel: Vector3<f32>,
}

#[derive(Debug)]
pub enum PhysicsCommand {
    // Main thread requests this frame's physics data. `debug_lines` asks for
    // the F2 overlay's lines too (see PhysicsBehavior::debug_draw) - sent
    // every frame rather than as a separate toggle, so it can never drift
    // out of sync with the render side (e.g. across a scene restart).
    RequestData { debug_lines: bool },
    Shutdown,         // Main thread signals shutdown
    TogglePause,      // Toggle physics pause
    /// Teleports a named rigidbody (see `Physics::physics_elements`) to an exact
    /// translation/rotation/velocity - meant to be sent right before un-pausing
    /// after a scripted cinematic (see `CameraTrack::LookAt` /
    /// `GameLogic::update`'s cinematic handling) has been moving that object's
    /// *render* transform directly while physics sat paused. Without this, the
    /// rigidbody resumes stepping from wherever it was when it got paused -
    /// wherever the script left it visually - causing a visible snap the instant
    /// physics starts writing the render transform again.
    SetTransform { name: String, translation: Vector3<f32>, rotation: Quaternion<f32>, linvel: Vector3<f32> },
}

pub struct PhysicsData {
    pub rigidbody_handle: RigidBodyHandle,
    pub collider_handles: Vec<ColliderHandle>,
}

pub struct Physics {
    pub physics_pipeline: PhysicsPipeline,
    pub colission_pipeline: CollisionPipeline,
    pub query_pipeline: QueryPipeline,
    pub gravity: Vector3<f32>,
    
    // Thread-safe physics data
    pub rigidbody_set: RigidBodySet, 
    pub collider_set: ColliderSet,

    pub physics_elements: HashMap<String, Option<PhysicsData>>,
    
    // Delta time tracking
    pub delta_time: f32,
    pub last_physics_time: Instant,
}

impl Physics {
    pub fn new() -> Self {
        // Physics data
        let physics = Physics {
            physics_pipeline: PhysicsPipeline::new(),
            colission_pipeline: CollisionPipeline::new(),
            query_pipeline: QueryPipeline::new(),
            gravity: Vector3::new(0.0, -9.81, 0.0),
            rigidbody_set: RigidBodySet::new(),
            collider_set: ColliderSet::new(),
            physics_elements: HashMap::new(),
            delta_time: 0.0,
            last_physics_time: Instant::now(),
        };

        physics
    }

    // `node_behaviors` are every node's physics half, keyed by node id (see
    // `SceneNodes::take_physics_behaviors`), talking to their main-thread
    // halves through `node_input_rx`/`node_state_tx`.
    pub fn physics_thread(&mut self, tx: Sender<HashMap<String, RenderMessage>>, rx: Receiver<PhysicsCommand>, debug_physics_tx: Sender<Vec<DebugPhysicsMessageType>>, node_behaviors: Vec<(String, Vec<Box<dyn PhysicsBehavior>>)>, node_input_rx: Receiver<HashMap<String, PhysicsInput>>, node_state_tx: Sender<HashMap<String, Vec<PhysicsPayload>>>) {
        const FIXED_TIMESTEP: f32 = 1.0 / 120.0; // Fixed timestep for 120 FPS for more responsive physics
        let mut accumulator = 0.0;
        let mut last_update = Instant::now();
        let mut should_send_data = false;
        let mut send_debug_lines = false;

        let integration_parameters = IntegrationParameters { dt: FIXED_TIMESTEP, ..Default::default() };
        let mut island_manager = IslandManager::new();
        let mut broad_phase = DefaultBroadPhase::new();
        let mut narrow_phase = NarrowPhase::new();
        let mut impulse_joint_set = ImpulseJointSet::new();
        let mut multibody_joint_set = MultibodyJointSet::new();
        let mut ccd_solver = CCDSolver::new();
        let physics_hooks = ();
        let event_handler = ();

        // A node's physics half needs a rigidbody to act on - one whose node
        // never registered a `Physics` property has nothing to drive, so it's
        // dropped here, loudly, instead of silently never running.
        let mut physics_nodes: Vec<PhysicsNode> = node_behaviors.into_iter()
            .filter_map(|(id, behaviors)| match self.physics_elements.get(&id) {
                Some(Some(physics_data)) => Some(PhysicsNode::new(id, physics_data.rigidbody_handle, behaviors)),
                _ => {
                    eprintln!("node '{id}' has physics behaviors but no rigidbody (missing Physics property?) - they won't run");
                    None
                }
            })
            .collect();

        let mut paused = false;
        let mut shutdown = false;

        loop {
            // Merge every input batch the main thread sent since last loop -
            // states overwrite, events queue until a fixed step consumes them.
            while let Ok(mut inputs) = node_input_rx.try_recv() {
                for physics_node in physics_nodes.iter_mut() {
                    if let Some(input) = inputs.remove(&physics_node.id) {
                        physics_node.receive_input(input);
                    }
                }
            }

            let now = Instant::now();
            let elapsed = now.duration_since(last_update).as_secs_f32();
            accumulator += elapsed;
            last_update = now;

            // Step the physics pipeline with fixed timestep (only if not paused)
            while accumulator >= FIXED_TIMESTEP && !paused {
                // Calculate delta time for this physics step
                let current_time = Instant::now();
                self.delta_time = current_time.duration_since(self.last_physics_time).as_secs_f32();
                self.last_physics_time = current_time;
                
                // Clamp delta time to prevent spiral of death
                let clamped_delta_time = self.delta_time.min(FIXED_TIMESTEP * 2.0);

                // Every node's physics half, at the real fixed rate, right
                // before the world advances by exactly that step.
                for physics_node in physics_nodes.iter_mut() {
                    physics_node.fixed_update(&mut self.rigidbody_set, &self.collider_set, &self.query_pipeline, FIXED_TIMESTEP);
                }
                
                self.physics_pipeline.step(
                    &self.gravity,
                    &integration_parameters,
                    &mut island_manager,
                    &mut broad_phase,
                    &mut narrow_phase,
                    &mut self.rigidbody_set,
                    &mut self.collider_set,
                    &mut impulse_joint_set,
                    &mut multibody_joint_set,
                    &mut ccd_solver,
                    Some(&mut self.query_pipeline),
                    &physics_hooks,
                    &event_handler,
                );

                accumulator -= FIXED_TIMESTEP;
            }

            loop {
                match rx.try_recv() {
                    Ok(PhysicsCommand::RequestData { debug_lines }) => {
                        should_send_data = true;
                        send_debug_lines = debug_lines;
                    },
                    Ok(PhysicsCommand::Shutdown) => {
                        println!("Physics thread received shutdown command");
                        shutdown = true;
                        break;
                    },
                    Ok(PhysicsCommand::TogglePause) => {
                        paused = !paused;
                        if !paused {
                            accumulator = 0.0;
                            last_update = Instant::now();
                            self.last_physics_time = Instant::now();
                        }
                        println!("Physics {}", if paused { "PAUSED" } else { "RESUMED" });
                    },
                    Ok(PhysicsCommand::SetTransform { name, translation, rotation, linvel }) => {
                        if let Some(Some(physics_data)) = self.physics_elements.get(&name) {
                            if let Some(rb) = self.rigidbody_set.get_mut(physics_data.rigidbody_handle) {
                                rb.set_translation(translation, true);
                                rb.set_rotation(nalgebra::Unit::new_normalize(rotation), true);
                                rb.set_linvel(linvel, true);
                            }
                        }
                    },
                    Err(_) => {
                        break;
                    }
                }
            }

            if shutdown {
                break;
            }

            if should_send_data {
                let mut new_render_messages: HashMap<String, RenderMessage> = HashMap::new();

                for (key, physics_data) in &self.physics_elements {
                    match physics_data {
                        Some(physics_data) => {
                            let rb = self.rigidbody_set.get(physics_data.rigidbody_handle).unwrap();

                            new_render_messages.insert(key.clone(), RenderMessage { translation: *rb.translation(), rotation: rb.rotation().into_inner(), linvel: *rb.linvel(), angvel: *rb.angvel() });
                        },
                        None => {},
                    }
                }

                if let Err(e) = tx.send(new_render_messages) {
                    println!("Failed to send render messages: {}", e);
                    break;
                }

                // Empty whenever the overlay is off, so the render side
                // drops whatever it was last showing.
                let mut debug_draw = DebugDraw::default();
                if send_debug_lines {
                    for physics_node in &physics_nodes {
                        physics_node.debug_draw(&self.rigidbody_set, &mut debug_draw);
                    }
                    // Every collider on a moving body, as it really is - fixed
                    // bodies (the ground/runway meshes) skipped, they'd bury
                    // the screen.
                    const COLLIDER_COLOR: [f32; 3] = [1.0, 0.55, 0.1];
                    for (_, body) in self.rigidbody_set.iter().filter(|(_, body)| body.is_dynamic()) {
                        for handle in body.colliders() {
                            if let Some(collider) = self.collider_set.get(*handle) {
                                debug_draw.collider(collider, COLLIDER_COLOR);
                            }
                        }
                    }
                }
                if let Err(e) = debug_physics_tx.send(debug_draw.into_lines()) {
                    println!("Failed to send debug physics messages: {}", e);
                }

                let node_states: HashMap<String, Vec<PhysicsPayload>> = physics_nodes.iter()
                    .map(|physics_node| (physics_node.id.clone(), physics_node.publish()))
                    .filter(|(_, states)| !states.is_empty())
                    .collect();
                if !node_states.is_empty() {
                    if let Err(e) = node_state_tx.send(node_states) {
                        println!("Failed to send node physics states: {}", e);
                    }
                }
                
                should_send_data = false; // Reset flag after sending
            }
        }
    }
    
    // Getter method to access delta time from other parts of the code
    pub fn get_delta_time(&self) -> f32 {
        self.delta_time
    }
    
    // Method to reset delta time (useful for debugging or when physics is paused)
    pub fn reset_delta_time(&mut self) {
        self.delta_time = 0.0;
        self.last_physics_time = Instant::now();
    }
}