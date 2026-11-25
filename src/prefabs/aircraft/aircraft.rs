use pankarta_software::prelude::*;

use crate::prefabs::aircraft::airframe::Airframe;
use crate::prefabs::aircraft::flight_controls::flight_controls::FlightControls;
use crate::prefabs::aircraft::powerplant::powerplant::Powerplant;
use crate::prefabs::aircraft::powerplant::EngineRole;
use crate::prefabs::aircraft::sensors::sensors::Sensors;
use crate::prefabs::world::atmosphere::Atmosphere;
use crate::prefabs::camera::camera_view::{CameraView, Cameras};
use crate::scenes::play::battleground::battleground::Player;

/*
    An aircraft its the main playable vehicle, mostly made for flying, it contains multiple systems
    allowing this behaviour

    in itself it should be able of flying obivously but also his capabilities include:

    Airframe: the definition of the main airfoils of the plane, each setting, and each aero calculation
        should or might belong here, also here we contain elements like flaps, airbrakes and landing gear
    Flight controls: a definition of tools designated to control the plane
        (alerons, rudders, elevators and fly by wire).
    Avionics: a definition of electronic systems that affect the flight of the aircraft (HUD, fuel, etc...).
    Pilons: a definition of places of the plane where the player can either attach weapons or other equipables.
    Flight plan: a definition of points in a map defining the "path" a plane should follow.
    Powerplant: the definition of engines, their force and how they work
*/


pub struct Aircraft {
    pub player: bool,
    pub airframe: Airframe,
    pub powerplant: Powerplant,
    pub flight_controls: FlightControls,
    pub cameras: Cameras,
    pub sensors: Sensors,
}

impl Aircraft {
    // The F-16 unless another preset is asked for
    pub fn new() -> Aircraft {
        Aircraft::f16()
    }

    pub fn prefab(mut self, id: String, transform: Transform3D) -> Node {
        // The node takes the colliders; the aircraft doesn't need them after spawning
        let colliders = std::mem::take(&mut self.airframe.fuselage.colliders);

        Node::new(id)
            .with_property(transform)
            .with_property(
                Model::file("planes/f16/model/f16.glb").transform(Transform3D {
                    scale: Vector3::repeat(14.0),
                    ..Default::default()
                }),
            )
            .with_property(
                RigidBodyBuilder::dynamic()
                    .additional_mass_properties(MassProperties::new(
                        self.airframe.mass.center_of_mass.into(),
                        self.airframe.mass.empty,
                        self.airframe.mass.inertia,
                    ))
                    .angular_damping(2.0),
            )
            .with_property(colliders)
            .with_behavior(self)
    }
}

impl Behavior for Aircraft {
    fn update(&mut self, node: &mut Node, scene: &mut SceneAccess, app: &mut App) {
        if node.get_property::<Player>().is_some() {
            self.flight_controls
                .read_input(&app.input, app.time.delta_time);
        }

        if let Some(model) = node.get_property_mut::<Model>() {
            self.airframe.landing_gear.place_wheels(model, 14.0);
            self.airframe.place_surfaces(model);
            self.powerplant.place_flames(model, app.time.delta_time);
        }

        // Debug
        let transform = node.get_property::<Transform3D>().unwrap();

        app.gizmos.channel("aircraft", |g| {
            let center_of_mass =
                transform.position + transform.rotation * self.airframe.mass.center_of_mass;

            g.icon(center_of_mass, Icon::Mass).on_top().label("CoM");
        });

        self.airframe.debug(app, transform.into());
        // Debug
    }

    fn fixed_update(&mut self, node: &mut Node, scene: &mut SceneAccess, app: &mut App) {
        // Rapier keeps added forces between steps, so each step starts from zero
        if let Some(body) = app.physics.body_mut(&node.id) {
            body.reset_forces(false);
            body.reset_torques(false);
        }

        let dt = app.physics.timestep();
        let atmosphere = Atmosphere::of(scene);

        if let Some(body) = app.physics.body(&node.id) {
            let grounded = self.airframe.landing_gear.grounded();
            self.sensors.measure(body, &atmosphere, grounded, dt);
        }
        let commands = self.flight_controls.commands(&self.sensors, dt);
        self.airframe.command(&commands, dt);
        self.flight_controls
            .follow_gear(self.airframe.landing_gear.lever());
        self.powerplant
            .set_lever(EngineRole::Main, commands.throttle);

        self.airframe.step(&node.id, &mut app.physics, &atmosphere, dt);
        self.powerplant.step(&node.id, &mut app.physics, dt);
    }
}
