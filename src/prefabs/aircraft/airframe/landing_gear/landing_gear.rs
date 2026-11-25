use nalgebra::{UnitQuaternion, Vector3};
use pankarta_software::scene_manager::node::properties::Transform3D;
use pankarta_software::{physics::PhysicsWorld, scene_manager::node::properties::Model};
use rapier3d::prelude::RigidBody;

use crate::prefabs::aircraft::airframe::landing_gear::ground_surfaces::GroundSurface;
use crate::prefabs::aircraft::airframe::landing_gear::LegSpec;
use crate::prefabs::aircraft::flight_controls::controls::gear::GearLever;

use super::gear_leg::{Contact, GearLeg};
use super::wheel::BREAKAWAY_SPEED;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GearState {
    Up,
    Extending,
    Down,
    Retracting,
}

const RAY_HEADROOM: f32 = 1.0;
const SIDE_STRETCH: f32 = 0.05; // sideways bend at which the tyre gives all its grip, meters
const RELAXATION_LENGTH: f32 = 1.0; // how far it rolls to straighten out, meters

/*
    This allows me to define the .into so from gear lever we can get a landing state
    avoiding having the possibility of starting with the landing gear "extending or retracting"
*/
impl From<GearLever> for GearState {
    fn from(lever: GearLever) -> Self {
        match lever {
            GearLever::Up => GearState::Up,
            GearLever::Down => GearState::Down,
        }
    }
}

pub struct LandingGear {
    gear_state: GearState, // The real airframe actual state (up, ext, dwn, rtr)
    gear_lever: GearLever, // The landing gear lever inside the cockpit (the control)
    deployed: f32,
    transit_time: f32,
    legs: Vec<GearLeg>,
    steer_angle: f32,
    grounded: bool, // a leg's reach finds the ground: the lever locks, like a weight-on-wheels switch
}

impl LandingGear {
    pub fn new(state: GearLever) -> LandingGear {
        let legs = vec![
            GearLeg::new(
                LegSpec::new("wheel-f", Vector3::new(0.0, 0.0, -5.2), 2.2)
                    .spring(203_000.0, 35_000.0)
                    .stroke(0.4)
                    .radius(0.218)
                    .steerable(),
            ),
            GearLeg::new(
                LegSpec::new("wheel-lb", Vector3::new(-1.4, 0.0, -1.2), 2.2)
                    .spring(287_000.0, 50_000.0)
                    .stroke(0.35)
                    .radius(0.357)
                    .braked()
                    .retracted(Transform3D {
                        position: Vector3::new(0.0, 0.0, -1.2),
                        rotation: UnitQuaternion::from_euler_angles(0.0, 0.0, 90f32.to_radians()),
                        ..Default::default()
                    }),
            ),
            GearLeg::new(
                LegSpec::new("wheel-rb", Vector3::new(1.4, 0.0, -1.2), 2.2)
                    .spring(287_000.0, 50_000.0)
                    .stroke(0.35)
                    .radius(0.357)
                    .braked()
                    .retracted(Transform3D {
                        position: Vector3::new(0.0, 0.0, -1.2),
                        rotation: UnitQuaternion::from_euler_angles(
                            0.0,
                            0.0,
                            (-90f32).to_radians(),
                        ),
                        ..Default::default()
                    }),
            ),
        ];

        LandingGear {
            gear_state: state.into(),
            gear_lever: state,
            // Starts where its lever is: fully down or fully up.
            deployed: match state {
                GearLever::Down => 1.0,
                GearLever::Up => 0.0,
            },
            transit_time: 5.0,
            legs: legs,
            steer_angle: 0.0,
            grounded: false,
        }
    }

    pub fn get_state(&self) -> GearState {
        self.gear_state
    }

    pub fn grounded(&self) -> bool {
        self.grounded
    }

    pub fn lever(&self) -> GearLever {
        self.gear_lever
    }

    pub fn up(&mut self) {
        // If the landing gear its already up dont do anything
        if self.gear_lever == GearLever::Up || self.grounded {
            return;
        }

        self.gear_lever = GearLever::Up;
        self.gear_state = GearState::Retracting;
    }

    pub fn down(&mut self) {
        if self.gear_lever == GearLever::Down || self.grounded {
            return;
        }

        self.gear_lever = GearLever::Down;
        self.gear_state = GearState::Extending;
    }

    pub fn toggle(&mut self) {
        match self.gear_lever {
            GearLever::Up => {
                // If the landing gear its already up, extend it
                self.gear_state = GearState::Extending;
                self.gear_lever = GearLever::Down
            }
            GearLever::Down => {
                // If the landing gear its already down, retract it
                self.gear_state = GearState::Retracting;
                self.gear_lever = GearLever::Up
            }
        }
    }

    pub fn step(&mut self, id: &str, physics: &mut PhysicsWorld, dt: f32, brake: f32) {
        self.move_gear(dt);

        self.find_contacts(id, physics);
        self.grounded = self.legs.iter().any(|leg| leg.contact.is_some());

        if self.deployed <= 0.0 {
            for leg in &mut self.legs {
                leg.contact = None;
                leg.anchor = None;
            }
            return;
        }

        let Some(body) = physics.body_mut(id) else {
            return;
        };
        let rotation = *body.rotation();
        let up = rotation * Vector3::y();
        let contacts = self
            .legs
            .iter()
            .filter(|leg| leg.contact.is_some())
            .count()
            .max(1) as f32;

        for leg in &mut self.legs {
            let Some(contact) = leg.contact.clone() else {
                leg.anchor = None;
                leg.side_stretch = 0.0;
                continue;
            };
            // A half-extended strut can't hold the full weight
            let load = contact.load * self.deployed;
            let mount = body.translation() + rotation * leg.spec.mount;
            body.add_force_at_point(up * load, mount.into(), true);

            let Some(heading) = leg.spec.wheel.heading(&rotation, self.steer_angle) else {
                continue;
            };
            let velocity = body.velocity_at_point(&contact.point.into());

            // The tyres sit far below the center of mass, so full grip would rock the jet past
            // straight every step: they grip only as hard as stops the slide within one step
            let side = Vector3::y().cross(&heading);
            let side_limit = effective_mass(body, contact.point, side) * velocity.dot(&side).abs()
                / dt
                / contacts;

            let surface = &GroundSurface::RUNWAY;
            let hold = leg
                .anchor
                .map(|anchor| (contact.point - anchor).dot(&heading));
            let tyre = leg
                .spec
                .wheel
                .tyre_force(velocity, heading, load, brake, surface, side_limit, hold);

            // The rubber bends as the tyre is pushed sideways and straightens as it rolls on, so a
            // steady push (a crosswind on the fin) is held straight away instead of creeping
            let side_speed = velocity.dot(&side);
            let rolled = velocity.dot(&heading).abs() * dt;
            leg.side_stretch =
                leg.side_stretch * (-rolled / RELAXATION_LENGTH).exp() + side_speed * dt;
            leg.side_stretch = leg.side_stretch.clamp(-SIDE_STRETCH, SIDE_STRETCH);
            let grip = surface.lateral_friction * load;
            let lateral =
                (tyre.dot(&side) - leg.side_stretch / SIDE_STRETCH * grip).clamp(-grip, grip);
            let tyre = tyre + side * (lateral - tyre.dot(&side));
            body.add_force_at_point(tyre, contact.point.into(), true);

            // Pushed harder than it holds, it breaks free; slow enough again, it sticks where it is
            let speed = velocity.dot(&heading);
            if hold.is_some()
                && tyre.dot(&heading).abs() >= leg.spec.wheel.holding_limit(load, brake, surface)
            {
                leg.anchor = None;
                leg.broke_free = true;
            } else if speed.abs() >= BREAKAWAY_SPEED {
                leg.anchor = None;
                leg.broke_free = false;
            } else if leg.anchor.is_none() && !leg.broke_free {
                leg.anchor = Some(contact.point);
            }
        }
    }

    fn move_gear(&mut self, dt: f32) {
        let travel = dt / self.transit_time;
        match self.gear_state {
            GearState::Extending => {
                self.deployed = (self.deployed + travel).min(1.0);
                if self.deployed >= 1.0 {
                    self.gear_state = GearState::Down;
                }
            }
            GearState::Retracting => {
                self.deployed = (self.deployed - travel).max(0.0);
                if self.deployed <= 0.0 {
                    self.gear_state = GearState::Up;
                }
            }
            GearState::Up | GearState::Down => {}
        }
    }

    // Basically a "is touching ground" on the rays of the wheels
    pub fn find_contacts(&mut self, id: &str, physics: &PhysicsWorld) {
        let Some(body) = physics.body(id) else {
            return;
        };
        let down = body.rotation() * -Vector3::y();

        // Each ray starts a little above its mount, so a strut pushed past its stroke still finds the ground
        for leg in &mut self.legs {
            let origin = body.translation() + body.rotation() * leg.spec.mount;
            leg.contact = physics
                .cast_ray(origin - down * RAY_HEADROOM, down, leg.spec.length + RAY_HEADROOM, Some(id))
                .map(|hit| {
                    let compression = leg.spec.length - (hit.distance - RAY_HEADROOM);
                    let closing_speed = body.velocity_at_point(&origin.into()).dot(&down);
                    Contact {
                        ground: hit.node,
                        point: hit.point,
                        compression,
                        load: leg.suspension_force(compression, closing_speed),
                    }
                });
        }
    }

    pub fn place_wheels(&self, model: &mut Model, scale: f32) {
        for leg in &self.legs {
            let Some(mesh) = model.mesh_mut(&leg.spec.wheel.mesh) else {
                continue;
            };
            let pose = leg.pose(self.deployed, self.steer_angle);
            mesh.transform.position = pose.position / scale;
            mesh.transform.rotation = pose.rotation * mesh.original_transform().rotation;
        }
    }
}

// How much mass a push along `direction` at `point` moves: the whole body's, less what it turns instead
fn effective_mass(body: &RigidBody, point: Vector3<f32>, direction: Vector3<f32>) -> f32 {
    let props = body.mass_properties();
    let arm = (point - props.world_com.coords).cross(&direction);
    let turning = props.effective_world_inv_inertia_sqrt * arm;
    let inverse =
        direction.dot(&props.effective_inv_mass.component_mul(&direction)) + turning.norm_squared();
    if inverse > 0.0 {
        1.0 / inverse
    } else {
        0.0
    }
}
