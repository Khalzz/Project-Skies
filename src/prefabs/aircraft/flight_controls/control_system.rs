use crate::prefabs::aircraft::{
    flight_controls::{
        controls::gear::GearLever,
        fbw::FlyByWire,
        flight_controls::{Cockpit, Commands},
    },
    sensors::sensors::Sensors,
};

pub enum ControlSystem {
    Direct,    // Relation 1 to 1 between the stick and the surfaces (hydraulic)
    Augmented, // to understand more
    FlyByWire(FlyByWire),
}

impl ControlSystem {
    pub fn compute(&mut self, cockpit: &Cockpit, sensors: &Sensors, dt: f32) -> Commands {
        let direct = Commands {
            pitch: cockpit.stick.pitch,
            roll: cockpit.stick.roll,
            yaw: cockpit.pedals.yaw,
            flaps: cockpit.flaps.request,
            throttle: cockpit.throttle.position(),
            gear: cockpit.gear_lever,
        };

        match self {
            ControlSystem::Direct => direct,
            ControlSystem::FlyByWire(fbw) => {
                let axes = fbw.compute(cockpit, sensors, dt);
                let flaps = match cockpit.gear_lever {
                    GearLever::Down => 1.0,
                    GearLever::Up if cockpit.flaps.alt_flaps => 1.0,
                    GearLever::Up => 0.0,
                };
                Commands {
                    pitch: axes.pitch,
                    roll: axes.roll,
                    yaw: axes.yaw,
                    flaps,
                    ..direct
                }
            }
            ControlSystem::Augmented => todo!(),
        }
    }
}
