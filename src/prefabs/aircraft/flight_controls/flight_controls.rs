/*
    Flight controls its everything from the pilot's hands to the commands: the cockpit (what the pilot moves) and
    the control system (what turns that into commands: direct linkage, augmentation or fly by wire). The surfaces
    themselves belong to the airframe.
*/

use pankarta_software::prelude::Input;

use crate::ext::input::InputExt;

use crate::prefabs::aircraft::flight_controls::control_system::ControlSystem;
use crate::prefabs::aircraft::flight_controls::controls::flaps::Flaps;
use crate::prefabs::aircraft::flight_controls::controls::gear::GearLever;
use crate::prefabs::aircraft::flight_controls::controls::pedals::Pedals;
use crate::prefabs::aircraft::flight_controls::controls::stick::Stick;
use crate::prefabs::aircraft::flight_controls::controls::throttle::Throttle;
use crate::prefabs::aircraft::sensors::sensors::Sensors;

/*
    Commands its a "control layer" designated mainly to allow the handle of "the middle man" on input related
    logic, for example systems like fly by wire mostly work affecting this commands systems, so

    "The input ask something, the command reads that something, and it returns a answer to give that value" and
    since the value could change for example for the fbw, instead of changing the stick directly, just change
    what the stick makes, or WHAT IT COMMANDS.
*/
#[derive(Clone, Copy, Default)]
pub struct Commands {
    pub pitch: f32,    // -1..1
    pub roll: f32,     // -1..1
    pub yaw: f32,      // -1..1
    pub flaps: f32,    // 0..1
    pub throttle: f32, // 0..1
    pub gear: GearLever,
}

// All cockpit related control inputs
pub struct Cockpit {
    pub throttle: Throttle,
    pub stick: Stick,
    pub pedals: Pedals,
    pub gear_lever: GearLever,
    pub flaps: Flaps,
}

pub struct FlightControls {
    cockpit: Cockpit,
    control_system: ControlSystem,
    flaps: f32, // where the flaps are, moving toward their command at FLAP_RATE
}

const FLAP_RATE: f32 = 0.5; // share of full travel per second

impl FlightControls {
    pub fn new(starting_throttle: f32, control_system: ControlSystem) -> FlightControls {
        FlightControls {
            cockpit: Cockpit {
                throttle: Throttle::new(starting_throttle),
                stick: Stick {
                    pitch: 0.0,
                    roll: 0.0,
                },
                pedals: Pedals::new(),
                gear_lever: GearLever::Down,
                flaps: Flaps {
                    request: 0.0,
                    alt_flaps: false,
                },
            },
            control_system,
            flaps: 1.0,
        }
    }

    pub fn read_input(&mut self, input: &Input, dt: f32) {
        self.cockpit
            .throttle
            .nudge(input.action_axis("throttle_down", "throttle_up"), dt);
        self.cockpit
            .throttle
            .follow_lever(input.absolute_value("throttle_axis"));
        self.cockpit.stick.follow(
            input.action_axis("pitch_up", "pitch_down"),
            input.action_axis("roll_left", "roll_right"),
            dt,
        );
        self.cockpit
            .pedals
            .follow(input.action_axis("rudder_left", "rudder_right"), dt);

        if input.is_action_just_pressed("toggle_landing_gear") {
            self.cockpit.gear_lever.toggle();
        }
    }

    // The gear handle can't stay where the gear refused to go (on the ground)
    pub fn follow_gear(&mut self, gear: GearLever) {
        self.cockpit.gear_lever = gear;
    }

    // What the control system makes of the cockpit; the flaps then travel toward their command
    pub fn commands(&mut self, sensors: &Sensors, dt: f32) -> Commands {
        let mut commands = self.control_system.compute(&self.cockpit, sensors, dt);
        self.flaps += (commands.flaps - self.flaps).clamp(-FLAP_RATE * dt, FLAP_RATE * dt);
        commands.flaps = self.flaps;
        commands
    }
}
