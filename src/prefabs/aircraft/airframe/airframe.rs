use nalgebra::Vector3;
use pankarta_software::{
    app::App,
    physics::{Colliders, PhysicsWorld},
    rendering::gizmos::{GizmoColor, Icon, Pose},
    scene_manager::node::properties::Model,
};

use crate::prefabs::{
    aircraft::{
        airframe::{
            fuselage::{Fuselage, FuselageAero},
            landing_gear::LandingGear,
            wings::{
                body::Motion,
                surface::{AeroForce, Surface},
            },
        },
        flight_controls::{controls::gear::GearLever, flight_controls::Commands},
    },
    world::atmosphere::Atmosphere,
};

/*
    Airframe references all sections related to the "physics of the plane" mostly the aerodynamic ones
    for example the wings contain:

    - Wing: the main surface thats defined as a "airfoil".
    - Aileron: the main surface defined for controling the deviation of lift through the surface.
    - Flaps: the surface dedicated to generate more lift for take off, landing or tighter turns.
*/

const METERS_PER_NEWTON: f32 = 1.0 / 5_000.0;
const KNOTS_PER_MS: f32 = 1.943_844;

pub struct Mass {
    pub empty: f32,
    pub center_of_mass: Vector3<f32>,
    pub inertia: Vector3<f32>, // Ixx, Iyy, Izz, given directly
}

pub struct Airframe {
    pub landing_gear: LandingGear,
    pub surfaces: Vec<Surface>, // todo: wings (surface definition and physics step)
    pub fuselage: Fuselage,
    pub mass: Mass,
    pub aero: Vec<AeroForce>, // what the surfaces made last step
}

impl Airframe {
    pub fn new() -> Airframe {
        Airframe {
            landing_gear: LandingGear::new(GearLever::Down),
            surfaces: vec![],
            fuselage: Fuselage {
                colliders: Colliders::new(),
                aero: FuselageAero::none(),
            },
            mass: Mass {
                empty: 8_700.0,
                center_of_mass: Vector3::new(0.0, 0.0, 0.0),
                inertia: Vector3::new(0.0, 0.0, 0.0),
            },
            aero: vec![],
        }
    }

    pub fn set_surfaces(mut self, surfaces: Vec<Surface>) -> Airframe {
        self.surfaces = surfaces;
        self
    }

    pub fn step(&mut self, id: &str, physics: &mut PhysicsWorld, atmosphere: &Atmosphere, dt: f32) {
        self.landing_gear.step(id, physics, dt, 0.0);

        let Some(body) = physics.body_mut(id) else {
            return;
        };
        let rotation = *body.rotation();
        let position = *body.translation();
        let density = atmosphere.air_at(position.y).density;

        let airflow = |point: Vector3<f32>| {
            let world = position + rotation * point;
            let moving = body.velocity_at_point(&world.into());
            rotation.inverse() * (atmosphere.wind_at(world) - moving)
        };
        self.aero = self
            .surfaces
            .iter()
            .flat_map(|surface| surface.aero_forces(&airflow, density))
            .chain(self.fuselage.aero.aero_forces(&airflow, density))
            .collect();

        for aero in &self.aero {
            let point = position + rotation * aero.point;
            body.add_force_at_point(rotation * (aero.lift + aero.drag), point.into(), true);
        }
    }

    pub fn command(&mut self, commands: &Commands, dt: f32) {
        for surface in &mut self.surfaces {
            surface.command(commands, dt);
        }
        match commands.gear {
            GearLever::Up => self.landing_gear.up(),
            GearLever::Down => self.landing_gear.down(),
        }
    }

    pub fn place_surfaces(&self, model: &mut Model) {
        for surface in &self.surfaces {
            surface.place(model);
        }
    }

    // should be runned once per update
    pub fn debug(&self, app: &mut App, pose: Pose) {
        app.gizmos.channel("aero", |g| {
            for surface in &self.surfaces {
                let outline = surface
                    .body
                    .planform
                    .corners()
                    .map(|corner| pose.point(surface.on_aircraft(corner)));

                g.outline(&outline);

                if let Motion::Fixed { sections } = &surface.body.motion {
                    for section in sections {
                        let outline = section
                            .corners()
                            .map(|corner| pose.point(surface.on_aircraft(corner)));
                        let Motion::Pivoting { hinge } = &section.motion else {
                            g.outline(&outline);
                            continue;
                        };
                        // Orange when it follows a command, cyan when nothing moves it
                        let color = if hinge.mix.is_passive() {
                            GizmoColor::CYAN
                        } else {
                            GizmoColor::ORANGE
                        };
                        g.outline(&outline).color(color).label(format!(
                            "{} {:+.1}°",
                            hinge.mix.describe(),
                            hinge.deflection
                        ));
                    }
                }
                let middle = surface.on_aircraft(surface.body.planform.middle());
                g.icon(pose.point(middle), Icon::Diamond)
                    .on_top()
                    .label(&surface.label);
            }
        });

        app.gizmos.channel("forces", |g| {
            let arrow = |force: Vector3<f32>| pose.rotation * force * METERS_PER_NEWTON;
            for aero in &self.aero {
                let at = pose.point(aero.point);
                let lift = g.line(at, at + arrow(aero.lift));
                lift.color(GizmoColor::GREEN).arrow().on_top();
                if !aero.section {
                    lift.label(format!("α {:+.1}° {:.1} kN", aero.alpha, aero.lift.norm() / 1000.0));
                    let flow = pose.rotation * aero.airflow.try_normalize(1e-3).unwrap_or_default();
                    g.line(at - flow * 3.0, at).color(GizmoColor::CYAN).arrow().dashed();
                }
                g.line(at, at + arrow(aero.drag)).color(GizmoColor::RED).arrow().on_top();
            }

            let lift: Vector3<f32> = self.aero.iter().map(|aero| aero.lift).sum();
            let drag: Vector3<f32> = self.aero.iter().map(|aero| aero.drag).sum();
            let speed = self
                .aero
                .iter()
                .find(|aero| !aero.section)
                .map_or(0.0, |aero| aero.airflow.norm());
            let center = pose.point(self.mass.center_of_mass);
            g.line(center, center + arrow(lift))
                .color(GizmoColor::YELLOW)
                .arrow()
                .on_top()
                .label(format!(
                    "{:.0} kt  lift {:.1} kN  drag {:.1} kN",
                    speed * KNOTS_PER_MS,
                    lift.norm() / 1000.0,
                    drag.norm() / 1000.0
                ));
        });
    }
}
