use std::time::Instant;

use nalgebra::Vector3;
use rapier3d::prelude::RigidBody;

use crate::engine::utils::lerps::lerp;
use crate::game::scenes::play::plane::controls::PlaneControls;

use super::rolling_rate::isa_density;

// ===========================================================================
// Pitch fly-by-wire  -  normal-g (Nz) command law, F-16 style.
// ===========================================================================
//
// WHAT IT IS
// ----------
// Unlike a Cessna, the pilot's pitch stick does NOT set an elevator angle. It
// sets a TARGET G-LOAD:
//     stick centred      ->  +1 g   (hands-off => the jet flies straight and
//                                     level at any speed, no trimming)
//     stick full back    ->  +9 g   (structural pull limit)
//     stick full forward ->  -2 g   (push limit)
// and the system continuously moves the all-moving stabilator to whatever
// angle actually produces that g at the current speed / altitude / AoA.
//
// HOW IT WORKS (and why THIS version, after several that didn't)
// -------------------------------------------------------------
// It is a pure INTEGRAL trim loop - nothing more:
//
//     g_error   = g_command - g_measured
//     stab_trim += SIGN * Ki * g_error * dt            (the whole controller)
//     elevator   = clamp(stab_trim, +/- authority)
//
// i.e. it automates exactly what a pilot does with a trim wheel: if we're
// pulling less g than commanded, wind the stabilator a hair further; if more,
// back it off; hold when it matches. That's it.
//
// Why not a "real" PID? Earlier attempts added proportional and derivative
// (dg/dt) feedback on the g error. Both create a FAST feedback path, and on
// this airframe - marginal short-period damping, a g signal that has to be
// filtered, one render-frame of latency - that fast path went ~180 deg out of
// phase and drove a sustained ~1 Hz +/-4 g limit cycle (it's in the logs). A
// pure integrator has NO fast path: it can only ramp, so it can't oscillate
// against the airframe. It's less crisp, but it's stable and it's correct,
// and "correct and boring" beats "sporty and diverging".
//
// Speed / altitude adaptation is automatic and needs no schedule: the same
// stabilator angle makes more g when you're fast, so the integrator simply
// settles at a smaller trim when fast and a larger one when slow. The only
// thing scheduled on dynamic pressure is Ki itself (below), purely so the
// loop's SPEED of adjustment doesn't get twitchy up at high q.
//
// AoA is used two ways: (1) the g command is progressively clipped as AoA
// approaches the limit, so the jet can't be pulled into a departure and, as a
// side effect, can't reach 9 g until it's above corner speed; (2) nothing
// else - AoA doesn't feed the trim loop directly.
//
// SIGN
// ----
// Grounded in a measured fact about THIS airframe, not a derivation:
// hand-trimmed level 1 g needs trim.pitch ~= -0.16, which Plane::update turns
// into elevator control_input = +0.16; with control_input 0 the same airframe
// sits at strongly negative g. So: MORE POSITIVE control_input => MORE g.
// Therefore g_error > 0 (need more g) must drive the trim MORE POSITIVE, so
// SIGN = +1.0. If the jet runs away to a g limit the instant FBW engages,
// flip SIGN to -1.0 - it is the only sign knob.
const SIGN: f32 = 1.0;

// Integral gain, quoted at the reference dynamic pressure. With the
// feed-forward below carrying the fast response, this only has to null out
// steady model error, so it stays slow (= stable).
const KI: f32 = 0.06;

// Feed-forward gain: an IMMEDIATE slab deflection proportional to how much g
// the stick is demanding away from 1 g, bypassing the (slow) g feedback
// entirely. This is what makes full stick ~instantly command a hard pull
// instead of waiting for the integrator to wind in. control_input units per
// g; scaled the same way KI is (down at high q). The integrator then trims
// out whatever this over/undershoots.
const KFF: f32 = 0.07;

// Ki is scaled by Q_REF/q, clamped to [SCHED_MIN, 1.0] - i.e. only ever slowed
// down at high q, never sped up. Keeps the adjustment rate roughly constant
// instead of getting aggressive as control power grows with speed.
const Q_REF: f32 = 0.5 * 1.225 * 160.0 * 160.0; // q at ~160 m/s / ~310 kn, sea level
const SCHED_MIN: f32 = 0.25;

// g command gradient.
const G_CMD_CENTER: f32 = 1.0;
const G_CMD_MAX: f32 = 9.0;
const G_CMD_MIN: f32 = -2.0;

// The effective g command is slew-limited toward the stick's demand at this
// rate (g per second), seeded to the measured g on engage. Fast enough now
// that stick response feels immediate (full range in ~0.3 s), slow enough to
// take the discontinuity off an engage from a wild g / a stick slam.
const G_CMD_SLEW_PER_S: f32 = 60.0;

// AoA limiter. Above AOA_SOFT_DEG the commanded g is blended toward
// AOA_LIMIT_G, full override at AOA_HARD_DEG.
const AOA_SOFT_DEG: f32 = 18.0;
const AOA_HARD_DEG: f32 = 24.0;
const AOA_LIMIT_G: f32 = 0.0;

// How much stabilator the loop may command. control_input scales to 25 deg at
// 1.0, so 0.5 == 12.5 deg - enough for 9 g, capped so nothing runs away hard.
const MAX_AUTHORITY: f32 = 0.5;

// Seed for the trim integrator on engage (~ the measured hand-trim value), so
// it starts near-trimmed rather than winding the whole thing in from zero.
const TRIM_SEED: f32 = 0.16;

// Output slew limit (control_input per second) - a final smoothing / safety
// rail. Fast enough not to be the bottleneck on stick response, slow enough
// to keep the slab from snapping.
const MAX_OUTPUT_SLEW_PER_S: f32 = 16.0;

// Below this airspeed (m/s) the loop is inert: output 0, integrator bled off.
const MIN_ACTIVE_SPEED: f32 = 20.0;

// Real per-call dt is clamped here before it drives the integrator.
const MAX_DT: f32 = 0.05;

// g-measurement low-pass rate (per second) - matches the HUD g-meter's own
// lerp(g, target, dt * 10.0).
const G_FILTER_RATE: f32 = 10.0;

const GRAVITY_MS2: f32 = 9.81;

// Print loop internals to stderr ~10x/sec while tuning.
const DEBUG_LOG: bool = false;

/// F-16-style normal-g command pitch fly-by-wire. See the module-level block
/// comment for the full rationale; the loop itself is a pure integral trim.
pub struct PitchFlcs {
    /// The stabilator trim the integrator has wound in (control_input units).
    /// This is the entire controller state.
    trim: f32,
    /// Slew-limited effective g command (seeded to measured g on engage).
    g_cmd_ramp: f32,
    /// Low-pass-filtered measured normal-g.
    g_filtered: f32,
    /// Final slew-limited output.
    output: f32,
    /// True until the first active tick has seeded the ramp / trim.
    needs_seed: bool,
    last_linvel: Option<Vector3<f32>>,
    linvel_dt_accum: f32,
    last_instant: Option<Instant>,
    debug_accum: f32,
    pub last_g_cmd: f32,
    pub last_g_meas: f32,
}

impl PitchFlcs {
    pub fn new() -> Self {
        Self {
            trim: TRIM_SEED,
            g_cmd_ramp: 1.0,
            g_filtered: 1.0,
            output: TRIM_SEED,
            needs_seed: true,
            last_linvel: None,
            linvel_dt_accum: 0.0,
            last_instant: None,
            debug_accum: 0.0,
            last_g_cmd: 1.0,
            last_g_meas: 1.0,
        }
    }

    /// Wipe all state. Call (or just stop calling `update`) when disengaged so
    /// a later engage starts clean.
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Returns the elevator `control_input` (-1..1) for both elevator wings
    /// this tick. Call every physics tick while FBW pitch is engaged.
    pub fn update(&mut self, rigidbody: &RigidBody, plane_controls: &PlaneControls) -> f32 {
        // --- real wall-clock dt (this is called many times per 120 Hz step) -
        let now = Instant::now();
        let dt = match self.last_instant {
            Some(prev) => (now - prev).as_secs_f32().min(MAX_DT),
            None => 0.0,
        };
        self.last_instant = Some(now);

        let rotation = *rigidbody.rotation();
        let linvel = *rigidbody.linvel();
        let airspeed = linvel.magnitude();
        let plane_up = rotation * Vector3::y();

        // --- fresh normal-g from linvel deltas ---------------------------
        // linvel only changes on an actual physics step; accumulate real time
        // and divide the velocity delta by THAT, not this call's tiny dt.
        self.linvel_dt_accum += dt;
        if let Some(prev) = self.last_linvel {
            if prev != linvel && self.linvel_dt_accum > 1e-5 {
                let gravity = Vector3::new(0.0, -GRAVITY_MS2, 0.0);
                let acceleration = (linvel - prev) / self.linvel_dt_accum;
                let felt = acceleration - gravity; // excludes gravity, like the HUD g-meter
                let g_now = felt.dot(&plane_up) / GRAVITY_MS2;
                let a = (self.linvel_dt_accum * G_FILTER_RATE).clamp(0.0, 1.0);
                self.g_filtered = lerp(self.g_filtered, g_now, a);
                self.last_linvel = Some(linvel);
                self.linvel_dt_accum = 0.0;
            }
        } else {
            self.last_linvel = Some(linvel);
            self.linvel_dt_accum = 0.0;
        }
        let g_meas = self.g_filtered;
        self.last_g_meas = g_meas;

        // --- too slow to fly: inert, bleed the integrator ----------------
        if airspeed < MIN_ACTIVE_SPEED {
            self.trim -= self.trim * (dt * 2.0).clamp(0.0, 1.0);
            self.output -= self.output * (dt * 5.0).clamp(0.0, 1.0);
            self.g_cmd_ramp = g_meas;
            self.needs_seed = true;
            self.last_g_cmd = G_CMD_CENTER;
            return self.output;
        }

        // --- stick -> commanded g --------------------------------------
        // plane_controls.elevator is eased raw stick here (Plane::update does
        // NOT fold pitch trim in while FBW is engaged). It's NEGATED: the
        // pilot wants stick-forward / W = nose down = push = negative g, and
        // W reads as +1 (see Plane::update). So stick_g = -elevator: W -> -1
        // -> -2 g, S -> +1 -> +9 g.
        let stick_g = (-plane_controls.elevator).clamp(-1.0, 1.0);
        let mut g_cmd = if stick_g >= 0.0 {
            G_CMD_CENTER + stick_g * (G_CMD_MAX - G_CMD_CENTER)
        } else {
            G_CMD_CENTER + stick_g * (G_CMD_CENTER - G_CMD_MIN)
        };
        g_cmd = g_cmd.clamp(G_CMD_MIN, G_CMD_MAX);

        // --- AoA limiter: clip the g command near the AoA limit ----------
        // True AoA (positive = nose above the relative wind), same convention
        // as Plane::apply_physics_feedback's aoa_y.
        let body_velocity = rotation.inverse() * linvel;
        let mut aoa_deg = 0.0;
        if body_velocity.magnitude() > 0.5 {
            aoa_deg = (-body_velocity.y).atan2(body_velocity.z).to_degrees();
            if aoa_deg > AOA_SOFT_DEG {
                let over =
                    ((aoa_deg - AOA_SOFT_DEG) / (AOA_HARD_DEG - AOA_SOFT_DEG)).clamp(0.0, 1.0);
                g_cmd = g_cmd.min(lerp(g_cmd, AOA_LIMIT_G, over));
            }
        }

        // --- seed on engage, then slew the effective command -----------
        if self.needs_seed {
            self.g_cmd_ramp = g_meas;
            self.trim = TRIM_SEED;
            self.output = TRIM_SEED;
            self.needs_seed = false;
        }
        let ramp_step = G_CMD_SLEW_PER_S * dt;
        self.g_cmd_ramp += (g_cmd - self.g_cmd_ramp).clamp(-ramp_step, ramp_step);
        self.last_g_cmd = self.g_cmd_ramp;

        // --- controller: feed-forward (fast) + integral trim (slow) ------
        let altitude_m = rigidbody.translation().y;
        let q = 0.5 * isa_density(altitude_m) * airspeed * airspeed;
        let sched = (Q_REF / q.max(1.0)).clamp(SCHED_MIN, 1.0);
        let ki = KI * sched;

        // Feed-forward: immediate slab deflection for the demanded g, no
        // waiting on the loop. Trimmed out by the integrator if it's off.
        let ff = SIGN * KFF * sched * (self.g_cmd_ramp - G_CMD_CENTER);

        // Integral trim: nulls the steady g error left after ff. Held while
        // the command is still slewing (ff hasn't landed) so it doesn't wind
        // up transient lag, and held when the total output is already pinned
        // to the authority limit.
        let g_err = self.g_cmd_ramp - g_meas;
        let ramping = (g_cmd - self.g_cmd_ramp).abs() > 1.0;
        let saturated = (self.trim + ff).abs() >= MAX_AUTHORITY - 1e-3;
        if !ramping && !saturated {
            self.trim += SIGN * ki * g_err * dt;
        }
        self.trim = self.trim.clamp(-MAX_AUTHORITY, MAX_AUTHORITY);

        // --- final slew-limited output --------------------------------
        let target = (self.trim + ff).clamp(-MAX_AUTHORITY, MAX_AUTHORITY);
        let max_step = MAX_OUTPUT_SLEW_PER_S * dt;
        self.output += (target - self.output).clamp(-max_step, max_step);
        self.output = self.output.clamp(-MAX_AUTHORITY, MAX_AUTHORITY);

        if DEBUG_LOG {
            self.debug_accum += dt;
            if self.debug_accum >= 0.1 {
                self.debug_accum = 0.0;
                eprintln!(
                    "[flcs] spd={:5.0} q={:7.0} ki={:.3} | stick_g={:+.2} g_cmd={:+.2}->{:+.2} g_meas={:+.2} g_hud={:+.2} err={:+.2} aoa={:+.1} | ff={:+.3} trim={:+.3} out={:+.3}",
                    airspeed, q, ki, stick_g, g_cmd, self.g_cmd_ramp, g_meas,
                    plane_controls.g_meter, g_err, aoa_deg, ff, self.trim, self.output
                );
            }
        }

        self.output
    }
}
