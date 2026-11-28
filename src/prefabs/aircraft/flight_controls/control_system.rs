use crate::prefabs::aircraft::{
    flight_controls::{
        controls::gear::GearLever,
        fbw::FlyByWire,
        flight_controls::{Cockpit, Commands},
    },
    sensors::sensors::Sensors,
};

// The F-16's flight computer holds the speedbrakes to 43 of their 60 degrees with the gear down,
// so the lower petals don't strike the runway on touchdown
const GEAR_DOWN_SPEEDBRAKE: f32 = 43.0 / 60.0;

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
            speedbrake: cockpit.speedbrake.request,
            leading_edge: 0.0, // nothing moves them without a flight computer
            brakes: cockpit.brakes.command(),
            // The nose wheel follows the pedals straight, not through the flight controls' yaw damping
            nose_steering: cockpit.nws.then_some(cockpit.pedals.yaw),
        };

        match self {
            ControlSystem::Direct => direct,
            ControlSystem::FlyByWire(fbw) => {
                let axes = fbw.compute(cockpit, sensors, dt);
                let leading_edge = fbw.leading_edge_flaps(sensors, cockpit.flaps.le_flaps_lock, dt);
                let flaps = match cockpit.gear_lever {
                    GearLever::Down => 1.0,
                    GearLever::Up if cockpit.flaps.alt_flaps => 1.0,
                    GearLever::Up => 0.0,
                };
                let speedbrake = match cockpit.gear_lever {
                    GearLever::Down => direct.speedbrake.min(GEAR_DOWN_SPEEDBRAKE),
                    GearLever::Up => direct.speedbrake,
                };
                Commands {
                    pitch: axes.pitch,
                    roll: axes.roll,
                    yaw: axes.yaw,
                    flaps,
                    speedbrake,
                    leading_edge,
                    ..direct
                }
            }
            ControlSystem::Augmented => todo!(),
        }
    }
}
