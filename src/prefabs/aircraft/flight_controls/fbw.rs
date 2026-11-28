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
const PITCH_I: f32 = 0.5; // command per g·second, what holds the surfaces once the g is met
const PITCH_DAMPING: f32 = 1.0; // command per rad/s of pitch rate
const PITCH_FEEDFORWARD: f32 = 0.08; // command per g asked over 1 g: what that g roughly takes, so it isn't left to the integral
const AOA_BAND: f32 = 8.0; // degrees under the limit where the g demand starts fading
const G_SHAPING: f32 = 0.25; // seconds the g demand takes to follow the stick, so a sudden stick doesn't overshoot

const LANDING_PITCH_RATE: f32 = 0.31; // rad/s (18 deg/s) at full stick with the gear down
const MODE_CHANGE: f32 = 1.5; // seconds the laws take to hand over when the gear handle moves
const LANDING_P: f32 = 1.5; // command per rad/s of pitch rate error
const LANDING_I: f32 = 1.0; // command per rad of pitch rate error built up

/*
    Leading edge flaps (NASA TP-1538, the F-16's flight control law, also in Stevens & Lewis):
        δ_lef = 1.38 · (2s + 7.25)/(s + 7.25) · α − 9.05 · q̄/p_s + 1.45      degrees, 0..25
    The lead-lag on α (2 − 7.25/(s + 7.25)) doubles a sudden change, so they droop ahead of a pull and
    not behind it; steady, it's just 1.38·α. q̄/p_s is ½γM² = 0.7·M², so they come up as the jet goes
    faster. With weight on the wheels they're held 2 degrees up. The LE FLAPS switch on LOCK holds
    them where they are.
*/
const LEF_ALPHA: f32 = 1.38;
const LEF_MACH: f32 = 9.05;
const LEF_BIAS: f32 = 1.45;
const LEF_LAG: f32 = 7.25; // rad/s, the lead-lag's corner
const LEF_TRAVEL: (f32, f32) = (25.0, 2.0); // degrees (down, up)
const LEF_GROUND: f32 = -2.0;

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
    g_demand: f32,   // the g asked for, following the stick at G_SHAPING
    pitch_trim: f32, // the built-up part of the pitch command, what holds a g or a rate once it's met
    landing: f32,    // how far into the takeoff and landing law it is: 0 up and away, 1 gear down
    lef_alpha: f32,  // α through the lead-lag's lag, degrees
    lef_command: f32, // the last leading edge flap command, what LOCK holds
}

impl FlyByWire {
    pub fn new(g_limits: (f32, f32), aoa_limit: f32, roll_rate: f32) -> FlyByWire {
        FlyByWire {
            g_limits,
            aoa_limit,
            roll_rate,
            g_demand: 1.0,
            pitch_trim: 0.0,
            landing: 1.0,
            lef_alpha: 0.0,
            lef_command: 0.0,
        }
    }

    // The leading edge flaps' command, as a share of their travel (+ down, - up)
    pub fn leading_edge_flaps(&mut self, sensors: &Sensors, locked: bool, dt: f32) -> f32 {
        let alpha = sensors.alpha;
        // The filter keeps running while locked, so going back to AUTO picks up from the jet's α
        self.lef_alpha += (alpha - self.lef_alpha) * (1.0 - (-LEF_LAG * dt).exp());
        if locked {
            return self.lef_command;
        }

        let degrees = if sensors.weight_on_wheels {
            self.lef_alpha = alpha;
            LEF_GROUND
        } else {
            let lead_lag = 2.0 * alpha - self.lef_alpha;
            let q_over_p = 0.7 * sensors.mach * sensors.mach;
            (LEF_ALPHA * lead_lag - LEF_MACH * q_over_p + LEF_BIAS).clamp(0.0, LEF_TRAVEL.0)
        };
        self.lef_command = if degrees >= 0.0 {
            degrees / LEF_TRAVEL.0
        } else {
            degrees / LEF_TRAVEL.1
        };
        self.lef_command
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
            self.g_demand = 1.0;
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
        self.g_demand += (demand - self.g_demand) * (1.0 - (-dt / G_SHAPING).exp());
        let g_error = self.g_demand - sensors.load_factor;
        let g_law = (
            (PITCH_FEEDFORWARD * (self.g_demand - 1.0) + PITCH_P * g_error - PITCH_DAMPING * sensors.rates.x)
                * schedule,
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
