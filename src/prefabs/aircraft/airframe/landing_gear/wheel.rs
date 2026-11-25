use nalgebra::{UnitQuaternion, Vector3};

use crate::prefabs::aircraft::airframe::landing_gear::ground_surfaces::GroundSurface;

const ROLLING_SLIP_SPEED: f32 = 0.1;
const LATERAL_SLIP_SPEED: f32 = 0.2;
pub const BREAKAWAY_SPEED: f32 = 0.02;
const HOLD_STRETCH: f32 = 0.15;
const HOLD_SETTLE_SPEED: f32 = 0.15;

#[derive(Debug, Clone)]
pub struct Wheel {
    pub mesh: String,
    pub radius: f32,
    pub steerable: bool,
    pub braked: bool,
}

impl Wheel {
    pub fn new(mesh: impl Into<String>) -> Self {
        Self {
            mesh: mesh.into(),
            radius: 0.0,
            steerable: false,
            braked: false,
        }
    }

    // If the wheel its steerable, it should allow deffinition of NWS for example through this execution
    pub fn heading(
        &self,
        rotation: &UnitQuaternion<f32>,
        steer_angle: f32,
    ) -> Option<Vector3<f32>> {
        let angle = if self.steerable { steer_angle } else { 0.0 };
        let world = rotation * Vector3::new(angle.sin(), 0.0, -angle.cos());
        Vector3::new(world.x, 0.0, world.z).try_normalize(1e-4)
    }

    // Defines forces of rolling
    pub fn tyre_force(
        &self,
        velocity: Vector3<f32>,
        heading: Vector3<f32>,
        load: f32,
        brake: f32,
        surface: &GroundSurface,
        side_limit: f32,
        hold: Option<f32>,
    ) -> Vector3<f32> {
        let side = Vector3::y().cross(&heading);
        let brake = if self.braked {
            brake.clamp(0.0, 1.0)
        } else {
            0.0
        };

        // The force that stops the rolling of the wheels (includes brake)
        let rolling = (surface.rolling_resistance + brake * surface.brake_friction) * load;

        // Direction of the wheel rounding
        let speed = velocity.dot(&heading);
        let longitudinal = match hold {
            // Stuck: pulled back to where it stopped, up to what it can hold
            Some(stretch) => {
                let limit = self.holding_limit(load, brake, surface);
                (-(stretch / HOLD_STRETCH + speed / HOLD_SETTLE_SPEED) * load).clamp(-limit, limit)
            }
            None => -(speed / ROLLING_SLIP_SPEED).clamp(-1.0, 1.0) * rolling,
        };

        // The friction for lateral forces of the wheel (imagine drift force)
        let lateral = -(velocity.dot(&side) / LATERAL_SLIP_SPEED).clamp(-1.0, 1.0)
            * surface.lateral_friction
            * load;
        let lateral = lateral.clamp(-side_limit, side_limit);

        heading * longitudinal + side * lateral
    }

    // A stopped tyre sits flattened on the ground and holds harder than a rolling one
    pub fn holding_limit(&self, load: f32, brake: f32, surface: &GroundSurface) -> f32 {
        let brake = if self.braked { brake.clamp(0.0, 1.0) } else { 0.0 };
        (surface.breakaway_resistance + brake * surface.brake_friction) * load
    }
}
