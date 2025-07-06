use rapier3d::prelude::{CCDSolver, ColliderSet, CollisionPipeline, DefaultBroadPhase, ImpulseJointSet, IntegrationParameters, IslandManager, MultibodyJointSet, NarrowPhase, PhysicsPipeline, QueryPipeline, RigidBodySet};
use nalgebra:: {Quaternion, Vector3};
use std::collections::HashMap;
use rapier3d::prelude::{ColliderHandle, RigidBodyHandle};
use std::sync::mpsc::{Sender, Receiver};
use std::thread;
use std::time::{Duration, Instant};
use crate::gameplay::plane::plane::{Plane, PlaneControls};
use crate::gameplay::plane::physics_logic::PlanePhysicsLogic;
use crate::gameplay::wheel::WheelData;
use serde::{Deserialize, Serialize};
use crate::physics::physics::DebugPhysicsMessageType;

#[derive(Debug, Clone)]
pub enum MetadataType {
    Translation(Vector3<f32>),
    Rotation(Quaternion<f32>),
    Wheels(HashMap<String, WheelData>),
}

pub struct RenderMessage {
    pub translation: Vector3<f32>,
    pub rotation: Quaternion<f32>,
    pub metadata: HashMap<String, MetadataType>
}

#[derive(Debug)]
pub enum PhysicsCommand {
    RequestData,  // Main thread requests physics data
    Shutdown,     // Main thread signals shutdown
}

pub struct PhysicsData {
    pub rigidbody_handle: RigidBodyHandle,
    pub collider_handle: Option<ColliderHandle>,
    pub metadata: HashMap<String, MetadataType>
}

pub struct Physics {
    pub physics_pipeline: PhysicsPipeline,
    pub colission_pipeline: CollisionPipeline,
    pub query_pipeline: QueryPipeline,
    pub gravity: Vector3<f32>,
    
    // Thread-safe physics data
    pub rigidbody_set: RigidBodySet, 
    pub collider_set: ColliderSet,

    pub physics_elements: HashMap<String, Option<PhysicsData>>
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
        };

        physics
    }

    pub fn physics_thread(&mut self, tx: Sender<HashMap<String, RenderMessage>>, rx: Receiver<PhysicsCommand>, plane_control_rx: Receiver<PlaneControls>, debug_physics_tx: Sender<Vec<DebugPhysicsMessageType>>) {
        const FIXED_TIMESTEP: f32 = 1.0 / 120.0; // Fixed timestep for 120 FPS for more responsive physics
        let mut accumulator = 0.0;
        let mut last_update = Instant::now();
        let mut should_send_data = false;

        let integration_parameters = IntegrationParameters { dt: FIXED_TIMESTEP, ..Default::default() };
        let mut island_manager = IslandManager::new();
        let mut broad_phase = DefaultBroadPhase::new();
        let mut narrow_phase = NarrowPhase::new();
        let mut impulse_joint_set = ImpulseJointSet::new();
        let mut multibody_joint_set = MultibodyJointSet::new();
        let mut ccd_solver = CCDSolver::new();
        let physics_hooks = ();
        let event_handler = ();

        let mut plane_physics_logic = PlanePhysicsLogic::new();
        let mut plane_controls: PlaneControls = PlaneControls::new();

        loop {
            match plane_control_rx.try_recv() {
                Ok(plane_control) => {
                    plane_controls = plane_control;
                },
                Err(_) => {
                    // No message available, continuing with physics
                }
            }

            let now = Instant::now();
            let elapsed = now.duration_since(last_update).as_secs_f32();
            accumulator += elapsed;
            last_update = now;

            // Apply forces before physics step
            match self.physics_elements.get_mut("player") {
                Some(physics_data) => {
                    match physics_data {
                        Some(physics_data) => {
                            plane_physics_logic.update(&plane_controls, &self.collider_set, &mut self.rigidbody_set, &self.query_pipeline, physics_data, &debug_physics_tx);
                        },
                        None => {
                            println!("Player not found");
                        }
                    }
                },
                None => {
                    println!("Player not found");
                }
            }

            // Step the physics pipeline with fixed timestep
            while accumulator >= FIXED_TIMESTEP {
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

            match rx.try_recv() {
                Ok(PhysicsCommand::RequestData) => {
                    should_send_data = true;
                },
                Ok(PhysicsCommand::Shutdown) => {
                    println!("Physics thread received shutdown command");
                    break;
                },
                Err(_) => {
                    // No message available, continuing with physics
                }
            }

            if should_send_data {
                let mut new_render_messages: HashMap<String, RenderMessage> = HashMap::new();

                for (key, physics_data) in &self.physics_elements {
                    match physics_data {
                        Some(physics_data) => {
                            let metadata = physics_data.metadata.clone();

                            new_render_messages.insert(key.clone(), RenderMessage { translation: *self.rigidbody_set.get(physics_data.rigidbody_handle).unwrap().translation(), rotation: self.rigidbody_set.get(physics_data.rigidbody_handle).unwrap().rotation().into_inner(), metadata: metadata });
                        },
                        None => {},
                    }
                }

                if let Err(e) = tx.send(new_render_messages) {
                    println!("Failed to send render messages: {}", e);
                    break;
                }

                if let Err(e) = debug_physics_tx.send(plane_physics_logic.renderizable_lines.clone()) {
                    println!("Failed to send debug physics messages: {}", e);
                }
                
                should_send_data = false; // Reset flag after sending
            }
        }
    }
}