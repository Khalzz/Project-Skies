use nalgebra::{UnitQuaternion, UnitVector3, Vector2, Vector3};
use pankarta_software::utils::lerps::lerp;
use rand::{rngs::ThreadRng, Rng};

pub struct ThrustVector {
    pub max_deflection: Vector2<f32>, // radians: (pitch, yaw), 0 = no vectoring
    deflection: Vector2<f32>,
}

impl ThrustVector {
    pub fn new(max_deflection: Vector2<f32>) -> ThrustVector {
        ThrustVector {
            max_deflection,
            deflection: Vector2::new(0.0, 0.0),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ThrustStage {
    pub lever: (f32, f32),
    pub thrust: (f32, f32), // fraction of max_thrust at each end of the lever range
    pub afterburner: bool,
}

/*
    The throttle lever (0..1) to thrust: stages in order, each starting where the last one ended,
    covering the whole lever travel
*/
#[derive(Debug, Clone)]
pub struct ThrustMap {
    stages: Vec<ThrustStage>,
}

impl ThrustMap {
    pub fn new(stages: Vec<ThrustStage>) -> ThrustMap {
        assert!(!stages.is_empty(), "thrust map: needs at least one stage");
        assert!(
            stages[0].lever.0 == 0.0,
            "thrust map: the first stage has to start at 0.0"
        );
        assert!(
            stages[stages.len() - 1].lever.1 == 1.0,
            "thrust map: the last stage has to end at 1.0"
        );
        for pair in stages.windows(2) {
            assert!(
                pair[0].lever.1 == pair[1].lever.0,
                "thrust map: each stage has to start where the previous one ends"
            );
        }
        ThrustMap { stages }
    }

    pub fn stage(&self, lever: f32) -> &ThrustStage {
        let lever = lever.clamp(0.0, 1.0);
        self.stages
            .iter()
            .find(|stage| lever <= stage.lever.1)
            .unwrap_or(&self.stages[self.stages.len() - 1])
    }

    // The thrust fractions the afterburner stages span, from where it lights to full
    pub fn afterburner_range(&self) -> Option<(f32, f32)> {
        let mut stages = self.stages.iter().filter(|stage| stage.afterburner);
        let first = stages.next()?;
        let last = stages.last().unwrap_or(first);
        Some((first.thrust.0, last.thrust.1))
    }

    pub fn thrust(&self, lever: f32) -> f32 {
        let lever = lever.clamp(0.0, 1.0);
        let stage = self.stage(lever);
        let along = (lever - stage.lever.0) / (stage.lever.1 - stage.lever.0);
        stage.thrust.0 + (stage.thrust.1 - stage.thrust.0) * along
    }
}

// Which command drives the engine, not what kind of engine it is
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum EngineRole {
    Main,
    Lift,
    Reaction,
}

// How fast the thrust follows the lever, in newtons per second
#[derive(Debug, Clone, Copy)]
pub struct Spool {
    pub up: f32,
    pub afterburner: f32,
    pub down: f32,
}

// The afterburner's flame mesh: how long it's drawn, flickering a little like a real one
pub struct Flame {
    pub mesh: String,
    length: f32,
    rng: ThreadRng,
}

impl Flame {
    fn new(mesh: &str) -> Flame {
        Flame {
            mesh: mesh.to_string(),
            length: 0.0,
            rng: rand::thread_rng(),
        }
    }

    // Grows quickly toward how lit it is, and dies down slower
    pub fn update(&mut self, lit: f32, dt: f32) -> f32 {
        if lit > 0.0 {
            let target = (lit + self.rng.gen_range(-0.15..0.15)).max(0.0);
            self.length = lerp(self.length, target, dt * 12.0);
        } else {
            self.length = lerp(self.length, 0.0, dt * 3.0);
        }
        self.length
    }
}

pub struct Engine {
    pub role: EngineRole,
    pub nozzle: Vector3<f32>,
    pub thrust_axis: UnitVector3<f32>,
    pub max_thrust: f32,
    pub thrust_map: ThrustMap,
    pub spool: Spool,
    pub lever: f32,
    pub thrust: f32,
    thrust_vector: Option<ThrustVector>,
    pub flame: Option<Flame>,
}

impl Engine {
    pub fn new(
        role: EngineRole,
        nozzle: Vector3<f32>,
        max_thrust: f32,
        thrust_map: ThrustMap,
        thrust_vector: Option<ThrustVector>,
    ) -> Engine {
        Engine {
            role,
            nozzle,
            thrust_axis: -Vector3::z_axis(),
            max_thrust,
            thrust_map,
            spool: Spool {
                up: f32::INFINITY,
                afterburner: f32::INFINITY,
                down: f32::INFINITY,
            },
            lever: 0.0,
            thrust: 0.0,
            thrust_vector,
            flame: None,
        }
    }

    pub fn flame(mut self, mesh: &str) -> Engine {
        self.flame = Some(Flame::new(mesh));
        self
    }

    // How lit the afterburner is (0..1), from the thrust the engine actually makes
    pub fn afterburner(&self) -> f32 {
        let Some((lights, full)) = self.thrust_map.afterburner_range() else {
            return 0.0;
        };
        let thrust = self.thrust / self.max_thrust;
        if thrust < lights {
            return 0.0;
        }
        ((thrust - lights) / (full - lights).max(1e-6)).clamp(0.0, 1.0)
    }

    pub fn axis(mut self, thrust_axis: UnitVector3<f32>) -> Engine {
        self.thrust_axis = thrust_axis;
        self
    }

    pub fn spool_rates(mut self, spool: Spool) -> Engine {
        self.spool = spool;
        self
    }

    // Moves thrust toward what the lever asks for, at the engine's spool speed
    pub fn spool(&mut self, dt: f32) {
        let target = self.thrust_map.thrust(self.lever) * self.max_thrust;
        let rate = if target < self.thrust {
            self.spool.down
        } else if self.thrust_map.stage(self.lever).afterburner {
            self.spool.afterburner
        } else {
            self.spool.up
        };
        self.thrust += (target - self.thrust).clamp(-rate * dt, rate * dt);
    }

    // The push direction in the aircraft's frame, after any thrust vectoring
    pub fn direction(&self) -> UnitVector3<f32> {
        match &self.thrust_vector {
            Some(vector) => {
                let deflection = UnitQuaternion::from_euler_angles(
                    vector.deflection.x,
                    vector.deflection.y,
                    0.0,
                );
                deflection * self.thrust_axis
            }
            None => self.thrust_axis,
        }
    }
}
