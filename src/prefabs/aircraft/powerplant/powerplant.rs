use nalgebra::Vector3;
use pankarta_software::physics::PhysicsWorld;
use pankarta_software::scene_manager::node::properties::Model;

use crate::prefabs::aircraft::powerplant::engine::{Engine, EngineRole};

/*
    A powerplant its the fisical element that defines forces applied on the plane, in this case the engines
*/

/*
    An engine contains different settings to define the position and direction of the engines that generate
    propulsion, this would include elements like engines for main propulsion or engines for vtol definition?
*/

pub struct Powerplant {
    engines: Vec<Engine>,
}

impl Powerplant {
    pub fn new(engines: Vec<Engine>) -> Powerplant {
        Powerplant { engines }
    }

    pub fn engines(&self) -> &[Engine] {
        &self.engines
    }

    pub fn set_lever(&mut self, role: EngineRole, lever: f32) {
        for engine in self.engines.iter_mut().filter(|engine| engine.role == role) {
            engine.lever = lever.clamp(0.0, 1.0);
        }
    }

    // Stretches each engine's flame mesh along its length by how lit its afterburner is
    pub fn place_flames(&mut self, model: &mut Model, dt: f32) {
        for engine in &mut self.engines {
            let lit = engine.afterburner();
            let Some(flame) = engine.flame.as_mut() else {
                continue;
            };
            let length = flame.update(lit, dt);
            if let Some(mesh) = model.mesh_mut(&flame.mesh) {
                let base = mesh.original_transform().scale;
                mesh.transform.scale = Vector3::new(base.x, base.y, base.z * length);
            }
        }
    }

    pub fn step(&mut self, id: &str, physics: &mut PhysicsWorld, dt: f32) {
        let Some(body) = physics.body_mut(id) else {
            return;
        };
        let rotation = *body.rotation();
        let translation = *body.translation();

        for engine in &mut self.engines {
            engine.spool(dt);
            let force = rotation * engine.direction().into_inner() * engine.thrust;
            let nozzle = translation + rotation * engine.nozzle;
            body.add_force_at_point(force, nozzle.into(), true);
        }
    }
}
