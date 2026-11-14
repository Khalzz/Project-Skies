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

struct Aircraft {
    airframe: Airframe,
}

impl Aircraft {
    pub fn new() -> Aircraft {
        Aircraft {
            airframe: Airframe::new(),
        }
    }
}

impl Behavior for Aircraft {}
