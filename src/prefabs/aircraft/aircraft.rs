use pankarta_software::prelude::*;

use crate::prefabs::aircraft::airframe::Airframe;

/*
    An aircraft its the main playable vehicle, mostly made for flying, it contains multiple systems
    allowing this behaviour

    in itself it should be able of flying obivously but also his capabilities include:

    Airframe: the definition of the main airfoils of the plane, each setting, and each aero calculation
        should or might belong here, also here we contain elements like flaps, airbrakes and landing gear
    Flight controls: a definition of tools designated to control the plane (alerons, rudders, elevators and fly by wire).
    Avionics: a definition of electronic systems that affect the flight of the aircraft (HUD, fuel, etc...).
    Pilons: a definition of places of the plane where the player can either attach weapons or other equipables.
    Flight plan: a definition of points in a map defining the "path" a plane should follow.
*/

pub struct Aircraft {
    airframe: Airframe,
}

impl Aircraft {
    pub fn new() -> Aircraft {
        Aircraft {
            airframe: Airframe::new(),
        }
    }

    pub fn prefab(self, id: String) -> Node {
        Node::new(id)
            .with_property(Transform3D {
                position: Vector3::new(40.0, 110.0, 40.0),
                ..Default::default()
            })
            .with_property(
                Model::file("planes/f16/model/f16.glb").transform(Transform3D {
                    scale: Vector3::repeat(14.0),
                    ..Default::default()
                }),
            )
            .with_property(
                Physics::new(RigidBodyData {
                    center_of_mass: Vector3::new(0.0, 0.8, 0.0),
                    ..RigidBodyData::dynamic(3_000.0)
                })
                .collider(
                    ColliderBuilder::cuboid(0.9, 1.0, 6.8)
                        .translation(Vector3::new(0.0, 0.0, 3.0))
                        .friction(0.8),
                ),
            )
            .with_property(self)
    }
}

impl Behavior for Aircraft {}
