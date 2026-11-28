use pankarta_software::prelude::*;

use crate::prefabs::aircraft::airframe::Airframe;
use crate::prefabs::aircraft::avionics::avionics::Avionics;
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
    pub avionics: Avionics,
    pub powerplant: Powerplant,
    pub flight_controls: FlightControls,
    pub cameras: Cameras,
    pub sensors: Sensors,
    pub debug: bool, // draws its forces and aero gizmos (F5 in the sandbox)
    pub(crate) chocks_placed: bool, // the chocks' model is on the ground at its wheels
}

impl Aircraft {
    // The F-16 unless another preset is asked for
    pub fn new() -> Aircraft {
        Aircraft::f16()
    }

    pub fn prefab(mut self, id: String, transform: Transform3D) -> Node {
        // The node takes the colliders; the aircraft doesn't need them after spawning
        let colliders = self.airframe.take_colliders();

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

impl Aircraft {
    /*
        The chocks as the crew leaves them: a block in front of and behind every wheel, on the ground where the
        wheels stand, its own node so it stays there in the world. Put down when the gear is chocked (with every
        wheel on the ground to know where), picked up when it isn't. Only what's seen: what they hold is the
        tyres' (see Wheel::chock_hold).
    */
    fn show_chocks(&mut self, node: &Node, scene: &mut SceneAccess) {
        let id = format!("{}-chocks", node.id);
        let gear = &self.airframe.landing_gear;
        if !gear.chocked() {
            if self.chocks_placed {
                scene.despawn(id);
                self.chocks_placed = false;
            }
            return;
        }
        if self.chocks_placed {
            return;
        }
        let (Some(wheels), Some(transform)) = (gear.wheels_on_ground(), node.get_property::<Transform3D>()) else {
            return;
        };
        scene.spawn(chocks(&id, *transform, &wheels));
        self.chocks_placed = true;
    }
}

const CHOCK_LENGTH: f32 = 0.22; // m, along the tyre's rolling
const CHOCK_WIDTH: f32 = 0.4; // m, across, wider than the tyre

// The blocks, `transform` the jet's where they were put
fn chocks(id: &str, transform: Transform3D, wheels: &[(Vector3<f32>, f32)]) -> Node {
    use crate::prefabs::aircraft::airframe::landing_gear::landing_gear::CHOCK_HEIGHT;
    let mut model = Model::new().material(Material::standard().color(UiColor::Rgb(205, 170, 35)));
    for (index, &(axle, radius)) in wheels.iter().enumerate() {
        // Touching the tyre where it's CHOCK_HEIGHT off the ground: that far from under the axle
        let height = CHOCK_HEIGHT.min(radius * 0.9);
        let reach = (radius * radius - (radius - height) * (radius - height)).sqrt();
        for (side, name) in [(-1.0, "front"), (1.0, "back")] {
            let center = axle + Vector3::new(0.0, height / 2.0 - radius, side * (reach + CHOCK_LENGTH / 2.0));
            model = model.part(
                format!("chock-{index}-{name}"),
                Mesh::cube().transform(Transform3D {
                    position: center,
                    scale: Vector3::new(CHOCK_WIDTH, height, CHOCK_LENGTH),
                    ..Default::default()
                }),
            );
        }
    }
    Node::new(id)
        .with_property(Transform3D {
            position: transform.position,
            rotation: transform.rotation,
            ..Default::default()
        })
        .with_property(model)
}

impl Behavior for Aircraft {
    fn update(&mut self, node: &mut Node, scene: &mut SceneAccess, app: &mut App) {
        if node.get_property::<Player>().is_some() {
            self.flight_controls
                .read_input(&app.input, app.time.delta_time);
            // The ground crew: chocks in (stopped on the wheels) or out
            if app.input.is_action_just_pressed("toggle_chocks") {
                self.airframe.landing_gear.toggle_chocks();
            }
        }
        self.show_chocks(node, scene);

        if let Some(model) = node.get_property_mut::<Model>() {
            self.airframe.landing_gear.place_wheels(model, 14.0);
            self.airframe.place_surfaces(model);
            self.powerplant.place_flames(model, app.time.delta_time);
        }

        if !self.debug {
            return;
        }
        let transform = node.get_property::<Transform3D>().unwrap();

        app.gizmos.channel("aircraft", |g| {
            let center_of_mass =
                transform.position + transform.rotation * self.airframe.mass.center_of_mass;

            g.icon(center_of_mass, Icon::Mass).on_top().label("CoM");
        });

        self.airframe.debug(app, transform.into());
    }

    fn fixed_update(&mut self, node: &mut Node, scene: &mut SceneAccess, app: &mut App) {
        // Rapier keeps added forces between steps, so each step starts from zero
        if let Some(body) = app.physics.body_mut(&node.id) {
            body.reset_forces(false);
            body.reset_torques(false);
        }

        let dt = app.physics.timestep();
        let atmosphere = Atmosphere::of(scene, &app.origin);

        if let Some(body) = app.physics.body(&node.id) {
            let grounded = self.airframe.landing_gear.grounded();
            self.sensors.measure(body, &atmosphere, grounded, dt);
        }
        self.avionics.update(&self.sensors);
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
