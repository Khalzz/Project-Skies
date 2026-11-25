/*
    FBW its a system that through a disconnection betwen the stick and the aircraft surfaces, allows a "connection"

    for example instead of connecting the stick to the elevators through hidraulic activation it uses a computer
    that tells the plane how much angle should make for doing a specific ammount of g's

    Each physics step it turns the stick into a demand (g in pitch, roll rate in roll), compares it with what the
    sensors measure and moves the surfaces to close the gap, holding them wherever that keeps the demand met.
    The surfaces do more the faster the air, so every gain is scaled by dynamic pressure; slow there's nothing to
    measure yet and the stick drives the surfaces directly, as it does with weight on the wheels. With the gear
    handle down (takeoff and landing) the stick asks for a pitch rate instead of g, which is gentler slow.
*/

use crate::prefabs::aircraft::{
    flight_controls::{controls::gear::GearLever, flight_controls::Cockpit},
    sensors::sensors::Sensors,
};

const Q_REFERENCE: f32 = 10_000.0; // Pa (~250 kt at sea level): the gains are tuned here
const Q_DIRECT: (f32, f32) = (1_500.0, 5_000.0); // Pa: stick direct below, full laws above
const MAX_SCHEDULE: f32 = 2.0; // slow, the gains grow at most this much over the reference

const PITCH_P: f32 = 0.15; // command per g of error
const PITCH_I: f32 = 0.4; // command per g·second, what holds the surfaces once the g is met
const PITCH_DAMPING: f32 = 0.5; // command per rad/s of pitch rate
const AOA_BAND: f32 = 5.0; // degrees under the limit where the g demand starts fading

const LANDING_PITCH_RATE: f32 = 0.31; // rad/s (18 deg/s) at full stick with the gear down
const MODE_CHANGE: f32 = 1.5; // seconds the laws take to hand over when the gear handle moves
const LANDING_P: f32 = 1.5; // command per rad/s of pitch rate error
const LANDING_I: f32 = 1.0; // command per rad of pitch rate error built up

const ROLL_P: f32 = 1.2; // command per rad/s of roll rate error
const YAW_DAMPING: f32 = 0.6; // command per rad/s of yaw rate

pub struct Axes {
    pub pitch: f32,
    pub roll: f32,
    pub yaw: f32,
}

pub struct FlyByWire {
    pub g_limits: (f32, f32), // (pushing, pulling)
    pub aoa_limit: f32,       // degrees
    pub roll_rate: f32,       // rad/s at full stick
    pitch_trim: f32, // the built-up part of the pitch command, what holds a g or a rate once it's met
    landing: f32,    // how far into the takeoff and landing law it is: 0 up and away, 1 gear down
}

impl FlyByWire {
    pub fn new(g_limits: (f32, f32), aoa_limit: f32, roll_rate: f32) -> FlyByWire {
        FlyByWire {
            g_limits,
            aoa_limit,
            roll_rate,
            pitch_trim: 0.0,
            landing: 1.0,
        }
    }

    pub fn compute(&mut self, cockpit: &Cockpit, sensors: &Sensors, dt: f32) -> Axes {
        let direct = Axes {
            pitch: cockpit.stick.pitch,
            roll: cockpit.stick.roll,
            yaw: cockpit.pedals.yaw,
        };

        // On the ground nothing can be corrected against the runway: the stick drives the surfaces, and the
        // trim follows it so the laws pick up from the same command after liftoff
        if sensors.weight_on_wheels {
            self.pitch_trim = direct.pitch;
            return direct;
        }

        let q = sensors.dynamic_pressure;
        let engaged = ((q - Q_DIRECT.0) / (Q_DIRECT.1 - Q_DIRECT.0)).clamp(0.0, 1.0);
        let schedule = (Q_REFERENCE / q.max(1.0)).min(MAX_SCHEDULE);

        // The gear handle picks the law; moving it hands one over to the other gradually
        let wanted = if cockpit.gear_lever == GearLever::Down { 1.0 } else { 0.0 };
        self.landing += (wanted - self.landing).clamp(-dt / MODE_CHANGE, dt / MODE_CHANGE);

        // Takeoff and landing: stick to pitch rate
        let rate_error = cockpit.stick.pitch * LANDING_PITCH_RATE - sensors.rates.x;
        let landing_law = (LANDING_P * rate_error * schedule, LANDING_I * rate_error);

        // Up and away: stick to g, faded out as the angle of attack nears its limit
        let stick = cockpit.stick.pitch;
        let mut demand = if stick >= 0.0 {
            1.0 + stick * (self.g_limits.1 - 1.0)
        } else {
            1.0 - stick * (self.g_limits.0 - 1.0)
        };
        if demand > 1.0 {
            let room = ((self.aoa_limit - sensors.alpha) / AOA_BAND).clamp(0.0, 1.0);
            demand = 1.0 + (demand - 1.0) * room;
        }
        let g_error = demand - sensors.load_factor;
        let g_law = (
            (PITCH_P * g_error - PITCH_DAMPING * sensors.rates.x) * schedule,
            PITCH_I * g_error,
        );

        let mix = |landing: f32, flying: f32| landing * self.landing + flying * (1.0 - self.landing);
        let pitch = mix(landing_law.0, g_law.0);
        self.pitch_trim += mix(landing_law.1, g_law.1) * dt * schedule * engaged;
        self.pitch_trim = self.pitch_trim.clamp(-1.0, 1.0);
        let pitch = pitch + self.pitch_trim;

        // Roll: stick to roll rate
        let roll = ROLL_P * (cockpit.stick.roll * self.roll_rate - sensors.rates.y) * schedule;

        // Yaw: the pedals, with the yaw motion damped out
        let yaw = cockpit.pedals.yaw - YAW_DAMPING * sensors.rates.z * schedule;

        let blend = |law: f32, direct: f32| law.clamp(-1.0, 1.0) * engaged + direct * (1.0 - engaged);
        Axes {
            pitch: blend(pitch, direct.pitch),
            roll: blend(roll, direct.roll),
            yaw: blend(yaw, direct.yaw),
        }
    }
}
