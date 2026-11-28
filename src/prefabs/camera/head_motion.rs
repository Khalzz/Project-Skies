/*
    The cockpit camera's head, thrown around by the G the pilot feels, through a damped spring so it moves like a
    neck: side G moves and rolls it, speeding up pushes it back and slowing down throws it forward, and G beyond 1
    tips it down. In the aircraft's frame (nose -Z, +X right, +Y up). Every number worth tweaking is in
    HEAD_MOTION; the "Cockpit head G reaction" setting scales the result.
*/

use nalgebra::{UnitQuaternion, Vector3};

pub struct HeadMotionTuning {
    pub side_shift: (f32, f32),    // meters per G, most meters
    pub side_roll: (f32, f32),     // degrees per G, most degrees
    pub accelerating: (f32, f32),  // head back: meters per G, most meters
    pub decelerating: (f32, f32),  // head forward, much further
    pub pitch: (f32, f32),         // degrees per G beyond 1, most degrees
    pub spring_frequency_hz: f32,
    pub spring_damping: f32,       // 1 settles without overshoot, lower bounces
    pub max_input_g: f32,          // keeps a crash's spikes from flinging it
}

pub const HEAD_MOTION: HeadMotionTuning = HeadMotionTuning {
    side_shift: (0.04, 0.06),
    side_roll: (6.0, 10.0),
    accelerating: (0.04, 0.06),
    decelerating: (0.12, 0.15),
    pitch: (0.75, 4.0),
    spring_frequency_hz: 3.0,
    spring_damping: 0.5,
    max_input_g: 9.0,
};

const MAX_SPRING_STEP: f32 = 1.0 / 240.0;

// What the pilot feels, in G
#[derive(Clone, Copy)]
pub struct HeadInput {
    pub lateral: f32,    // + toward the right (+X)
    pub forward: f32,    // + speeding up
    pub vertical_g: f32, // 1 in level flight
}

pub struct HeadPose {
    pub offset: Vector3<f32>,        // from the resting eye, in the aircraft's frame
    pub tilt: UnitQuaternion<f32>,   // under the pilot's own look
    pub roll_deg: f32,
}

pub struct HeadMotion {
    load: Vector3<f32>, // (side, vertical beyond 1, forward) as the spring has it
    load_velocity: Vector3<f32>,
}

impl HeadMotion {
    pub fn new() -> Self {
        Self {
            load: Vector3::zeros(),
            load_velocity: Vector3::zeros(),
        }
    }

    pub fn update(&mut self, input: HeadInput, delta_time: f32) {
        let limit = HEAD_MOTION.max_input_g;
        let target = Vector3::new(input.lateral, input.vertical_g - 1.0, input.forward)
            .map(|g| g.clamp(-limit, limit));
        let omega = HEAD_MOTION.spring_frequency_hz * std::f32::consts::TAU;
        let steps = (delta_time / MAX_SPRING_STEP).ceil().max(1.0);
        let dt = delta_time / steps;
        for _ in 0..steps as usize {
            let acceleration = (target - self.load) * omega * omega
                - self.load_velocity * (2.0 * HEAD_MOTION.spring_damping * omega);
            self.load_velocity += acceleration * dt;
            self.load += self.load_velocity * dt;
        }
    }

    pub fn pose(&self, scale: f32) -> HeadPose {
        let tuning = &HEAD_MOTION;
        let (side_g, extra_g, forward_g) = (self.load.x, self.load.y, self.load.z);
        let reaction = |g: f32, (per_g, max): (f32, f32)| soft_limit(g * per_g, max) * scale;

        // Thrown opposite the side G, and rolled that way (about +Z tips the top of the head toward -X)
        let shift_x = -reaction(side_g, tuning.side_shift);
        let roll_deg = reaction(side_g, tuning.side_roll);

        // Back (+Z) when speeding up, forward when slowing down
        let travel = if forward_g < 0.0 { tuning.decelerating } else { tuning.accelerating };
        let shift_z = reaction(forward_g, travel);

        // More G tips the view down (about +X the view turns up, so the opposite)
        let pitch_deg = -reaction(extra_g, tuning.pitch);

        let roll = UnitQuaternion::from_axis_angle(&Vector3::z_axis(), roll_deg.to_radians());
        let pitch = UnitQuaternion::from_axis_angle(&Vector3::x_axis(), pitch_deg.to_radians());
        HeadPose {
            offset: Vector3::new(shift_x, 0.0, shift_z),
            tilt: roll * pitch,
            roll_deg,
        }
    }
}

// Eased into ±max instead of clipped at it
fn soft_limit(value: f32, max: f32) -> f32 {
    if max <= 0.0 {
        return 0.0;
    }
    max * (value / max).tanh()
}
