//! The cockpit camera's head, following the G meter
//! (`FlightData::lateral_g`/`longitudinal_g`/`body_g`):
//! - X (side) G: moves the head sideways AND rolls it, toward the side it's
//!   thrown;
//! - Z (forward/back) G: moves the head forward/back - speeding up pushes it
//!   back, slowing down throws it forward (much further);
//! - Y (seat-axis) G, beyond level flight's 1 G: pitches the head - more G
//!   tips it down, negative G up.
//! Through a damped spring, so the head moves like a neck rather than
//! jumping straight to each reading.
//!
//! Every number worth tweaking is in `HEAD_MOTION` below. All of it is at
//! 100% on the "Cockpit head G reaction" setting - the setting scales the
//! final movement (200% = twice as far, 0% = a fully static head).

use nalgebra::{UnitQuaternion, Vector3};

/// Side G: how far the head moves and rolls per G, and the most of each -
/// approached smoothly (`soft_limit`), never snapped against.
pub struct SideReaction {
    /// Meters per G.
    pub shift_per_g: f32,
    pub max_shift: f32,
    /// Degrees of roll per G.
    pub roll_deg_per_g: f32,
    pub max_roll_deg: f32,
}

/// Forward/back G: how far the head moves per G, and the most it moves.
pub struct TravelReaction {
    /// Meters per G.
    pub shift_per_g: f32,
    pub max_shift: f32,
}

/// Seat-axis G: how far the head pitches per G beyond 1 G, and the most.
pub struct PitchReaction {
    pub pitch_deg_per_g: f32,
    pub max_pitch_deg: f32,
}

pub struct HeadMotionTuning {
    /// X: side G - position and roll.
    pub side: SideReaction,
    /// Z: speeding up - head back.
    pub accelerating: TravelReaction,
    /// Z: slowing down - head forward, much further than `accelerating`.
    pub decelerating: TravelReaction,
    /// Y: seat-axis G - pitch.
    pub vertical: PitchReaction,
    /// The spring: how fast the head follows (Hz) and how much it's damped
    /// (1.0 = settles with no overshoot, lower = more bounce).
    pub spring_frequency_hz: f32,
    pub spring_damping: f32,
    /// Every reading is clamped to this (G, either way, per axis) before it
    /// reaches the spring - keeps a crash's huge spikes from flinging it.
    pub max_input_g: f32,
}

pub const HEAD_MOTION: HeadMotionTuning = HeadMotionTuning {
    side: SideReaction { shift_per_g: 0.04, max_shift: 0.06, roll_deg_per_g: 6.0, max_roll_deg: 10.0 },
    accelerating: TravelReaction { shift_per_g: 0.04, max_shift: 0.06 },
    decelerating: TravelReaction { shift_per_g: 0.12, max_shift: 0.15 },
    vertical: PitchReaction { pitch_deg_per_g: 0.75, max_pitch_deg: 4.0 },
    spring_frequency_hz: 3.0,
    spring_damping: 0.5,
    max_input_g: 9.0,
};

/// The spring never steps more than this at once (s), however long the frame.
const MAX_SPRING_STEP: f32 = 1.0 / 240.0;

/// The G meter's readings this frame - see `FlightData`'s fields of the same
/// meaning.
#[derive(Clone, Copy)]
pub struct HeadInput {
    /// `FlightData::lateral_g` (G, + toward the plane's +X = left wing).
    pub lateral: f32,
    /// `FlightData::longitudinal_g` (G, + = speeding up).
    pub forward: f32,
    /// Felt seat-axis G, `FlightData::body_g.y` (1 = level flight).
    pub vertical_g: f32,
}

impl Default for HeadInput {
    fn default() -> Self {
        Self { lateral: 0.0, forward: 0.0, vertical_g: 1.0 }
    }
}

/// Where the head is this frame, in the plane's own frame.
pub struct HeadPose {
    /// Eye position offset from the resting cockpit camera (plane-local m).
    pub offset: Vector3<f32>,
    /// Head roll and pitch, applied under the pilot's own look direction.
    pub tilt: UnitQuaternion<f32>,
}

pub struct HeadMotion {
    // (lateral, vertical G beyond 1, forward) as the spring currently has it.
    load: Vector3<f32>,
    load_velocity: Vector3<f32>,
}

impl HeadMotion {
    pub fn new() -> Self {
        Self { load: Vector3::zeros(), load_velocity: Vector3::zeros() }
    }

    pub fn update(&mut self, input: HeadInput, delta_time: f32) {
        let limit = HEAD_MOTION.max_input_g;
        let target = Vector3::new(input.lateral, input.vertical_g - 1.0, input.forward).map(|g| g.clamp(-limit, limit));

        let omega = HEAD_MOTION.spring_frequency_hz * std::f32::consts::TAU;
        let steps = (delta_time / MAX_SPRING_STEP).ceil().max(1.0);
        let dt = delta_time / steps;
        for _ in 0..steps as usize {
            let acceleration = (target - self.load) * omega * omega - self.load_velocity * (2.0 * HEAD_MOTION.spring_damping * omega);
            self.load_velocity += acceleration * dt;
            self.load += self.load_velocity * dt;
        }
    }

    /// The head's pose for the current spring state, at `scale` of the
    /// tuning (the "Cockpit head G reaction" setting, 1.0 = 100%).
    pub fn pose(&self, scale: f32) -> HeadPose {
        let tuning = &HEAD_MOTION;
        let (side_g, extra_g, forward_g) = (self.load.x, self.load.y, self.load.z);

        // X: the head is thrown opposite the side G, and rolls that way -
        // rotating about +Z tips the top of the head toward -X, so a
        // positive side G (throwing it toward -X) is a positive roll.
        let shift_x = -soft_limit(side_g * tuning.side.shift_per_g, tuning.side.max_shift) * scale;
        let roll_deg = soft_limit(side_g * tuning.side.roll_deg_per_g, tuning.side.max_roll_deg) * scale;

        // Z: thrown opposite the forward G - back when speeding up, forward
        // (further) when slowing down.
        let travel = if forward_g < 0.0 { &tuning.decelerating } else { &tuning.accelerating };
        let shift_z = -soft_limit(forward_g * travel.shift_per_g, travel.max_shift) * scale;

        // Y: rotating about +X tips the top of the head toward +Z - the view
        // down - so more G is a positive pitch.
        let pitch_deg = soft_limit(extra_g * tuning.vertical.pitch_deg_per_g, tuning.vertical.max_pitch_deg) * scale;

        let roll = UnitQuaternion::from_axis_angle(&Vector3::z_axis(), roll_deg.to_radians());
        let pitch = UnitQuaternion::from_axis_angle(&Vector3::x_axis(), pitch_deg.to_radians());

        HeadPose { offset: Vector3::new(shift_x, 0.0, shift_z), tilt: roll * pitch }
    }
}

/// `value`, eased into +-`max` instead of clipped at it: about linear for
/// small values, bending over smoothly as it nears the limit.
fn soft_limit(value: f32, max: f32) -> f32 {
    if max <= 0.0 {
        return 0.0;
    }
    max * (value / max).tanh()
}
