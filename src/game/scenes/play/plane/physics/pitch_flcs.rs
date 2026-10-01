use nalgebra::Vector3;
use rapier3d::prelude::RigidBody;

use crate::engine::utils::lerps::lerp;
use crate::game::scenes::play::plane::controls::PlaneControls;

use super::wings::wing::AIR_DENSITY;

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
// HOW IT WORKS
// ------------
// The real jet's law: a g command with pitch-rate feedback (its rate gyros),
// all scaled by how much g a degree of slab makes at this speed (~q):
//
//     slab_per_g = SLAB_PER_G_TIMES_Q / q                  (the airframe)
//     ff         = slab_per_g * (g_command - 1)             (lands it at once)
//     g_model   -> g_command, lagged G_MODEL_LAG_S          (how fast it gets there)
//     trim      += KI_RATE * slab_per_g * (g_model - g_measured) * dt
//     damping    = -KQ * slab_per_g * (the pitch rate beyond what the
//                                      commanded g's turn needs, as g)
//     elevator   = clamp(trim + ff + damping, +/- authority)
//
// The feed-forward puts the slab about where the commanded g needs it
// straight away; the integrator trims out what the model misses (toward the
// lagged g_model, so it doesn't wind up while the g is still building and
// overshoot); the damping holds the nose's short-period bobbing down, which
// is what lets the other two be fast. Measured on this airframe in
// plane::flight_tests: ~8 g in 0.8 s at Mach 1, no overshoot, -2 g in ~1 s.
//
// Why pitch RATE and not g for the damping: earlier versions fed back the
// g error itself (proportional/derivative) and drove a ~1 Hz +/-4 g limit
// cycle - the g signal is filtered and a step late, so that fast path ended
// up out of phase. Pitch rate comes straight off the body, unfiltered.
//
// q is worked out with the same air density the wings use (wing::
// AIR_DENSITY) - a q the wings don't feel would put the slab per g off by
// the difference (it overshot at altitude when this used real ISA air).
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

// The airframe's pitch control power: how much stabilator (control_input)
// each g away from 1 g takes, times dynamic pressure q (Pa) - the slab's
// moment grows with q, so the slab a g needs shrinks as 1/q. Measured on
// this airframe in the flight tests (plane::flight_tests): ~0.046 per g at
// Mach 1 / q ~55 kPa. Everything below is scaled by it, so the loop behaves
// the same at every speed.
const SLAB_PER_G_TIMES_Q: f32 = 2500.0;
// Lowest q the scaling is worked out at (Pa, ~60 m/s at sea level) - below
// it the slab a g needs stops growing, so the loop doesn't go wild slow.
const MIN_SCHEDULE_Q: f32 = 2200.0;

// Feed-forward: an IMMEDIATE slab deflection for the g demanded away from
// 1 g - the airframe's own slab per g (above) times this. 1 = exactly what
// the model says; the integrator trims out what it misses.
const KFF: f32 = 1.0;

// Integral trim: how fast (per second) it winds the slab toward the g still
// missing - in slab-per-g units, so ~this many g of error a second at any
// speed.
const KI_RATE: f32 = 2.0;

// Pitch-rate damping - the real jet's rate-gyro feedback. Against how much
// faster (or slower) the nose is rotating than the commanded g needs (a
// steady turn's own pitch rate isn't fought), as the g that excess rate
// would be worth, times the slab per g. This is what lets the feed-forward
// and integrator be fast without the short-period bobbing.
const KQ: f32 = 1.0;

// The g response the loop aims for: the command, followed with this lag (s)
// - about how fast the airframe really gets there. The integrator trims
// toward THIS, not the raw command, so it doesn't wind up while the g is
// still building (and overshoot when it arrives).
const G_MODEL_LAG_S: f32 = 0.25;

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
// 1.0 - the real slab's full travel (it used to be capped at half, 12.5 deg,
// which left the jet ~3 g short at corner speed).
const MAX_AUTHORITY: f32 = 1.0;

// Seed for the trim integrator on engage (~ the measured hand-trim value), so
// it starts near-trimmed rather than winding the whole thing in from zero.
const TRIM_SEED: f32 = 0.16;

// Output slew limit (control_input per second) - a final smoothing / safety
// rail. Fast enough not to be the bottleneck on stick response, slow enough
// to keep the slab from snapping.
const MAX_OUTPUT_SLEW_PER_S: f32 = 16.0;

// Below this airspeed (m/s) the loop is inert: output 0, integrator bled off.
const MIN_ACTIVE_SPEED: f32 = 20.0;

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
    /// The g the jet should be pulling by now - the command through
    /// G_MODEL_LAG_S; what the integrator trims toward.
    g_model: f32,
    /// Low-pass-filtered measured normal-g.
    g_filtered: f32,
    /// Final slew-limited output.
    output: f32,
    /// True until the first active tick has seeded the ramp / trim.
    needs_seed: bool,
    last_linvel: Option<Vector3<f32>>,
    linvel_dt_accum: f32,
    debug_accum: f32,
    pub last_g_cmd: f32,
    pub last_g_meas: f32,
}

impl PitchFlcs {
    pub fn new() -> Self {
        Self {
            trim: TRIM_SEED,
            g_cmd_ramp: 1.0,
            g_model: 1.0,
            g_filtered: 1.0,
            output: TRIM_SEED,
            needs_seed: true,
            last_linvel: None,
            linvel_dt_accum: 0.0,
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
    /// this tick. Call every physics tick while FBW pitch is engaged - `dt`
    /// is the fixed physics step (see `AircraftPhysics::fixed_update`).
    pub fn update(&mut self, rigidbody: &RigidBody, plane_controls: &PlaneControls, dt: f32) -> f32 {

        let rotation = *rigidbody.rotation();
        let linvel = *rigidbody.linvel();
        let airspeed = linvel.magnitude();
        let plane_up = rotation * Vector3::y();

        // --- fresh normal-g from linvel deltas ---------------------------
        // Called once per physics step, so linvel normally changes every
        // call; the accumulator only matters if it ever doesn't.
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
            self.g_model = g_meas;
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
        // as Instrumentation::update's aoa_y.
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
            self.g_model = g_meas;
            self.trim = TRIM_SEED;
            self.output = TRIM_SEED;
            self.needs_seed = false;
        }
        let ramp_step = G_CMD_SLEW_PER_S * dt;
        self.g_cmd_ramp += (g_cmd - self.g_cmd_ramp).clamp(-ramp_step, ramp_step);
        self.last_g_cmd = self.g_cmd_ramp;

        // --- controller: feed-forward + integral trim + pitch-rate damping -
        // The same air the wings fly in (see wing::AIR_DENSITY - today sea
        // level at every altitude) - a q the wings don't feel would put the
        // slab per g off by the difference.
        let q = 0.5 * AIR_DENSITY * airspeed * airspeed;
        // The slab one g takes at this q - see SLAB_PER_G_TIMES_Q.
        let slab_per_g = SLAB_PER_G_TIMES_Q / q.max(MIN_SCHEDULE_Q);

        // Feed-forward: immediate slab deflection for the demanded g, no
        // waiting on the loop. Trimmed out by the integrator if it's off.
        let ff = SIGN * KFF * slab_per_g * (self.g_cmd_ramp - G_CMD_CENTER);

        // Pitch-rate damping. Nose-up rate: the body's +X is the left wing,
        // so a + rate about it is nose DOWN. The rate the commanded g needs
        // (the turn's own rate) is left alone - only the excess is damped,
        // worth (excess rate x airspeed / g) in g.
        let body_rate = rotation.inverse() * *rigidbody.angvel();
        let nose_up_rate = -body_rate.x;
        let needed_rate = (self.g_cmd_ramp - plane_up.y) * GRAVITY_MS2 / airspeed;
        let excess_g = (nose_up_rate - needed_rate) * airspeed / GRAVITY_MS2;
        let damping = -SIGN * KQ * slab_per_g * excess_g;

        // Integral trim: nulls the steady g error left after ff. Held while
        // the command is still slewing (ff hasn't landed) so it doesn't wind
        // up transient lag, and held when the total output is already pinned
        // to the authority limit.
        self.g_model += (self.g_cmd_ramp - self.g_model) * (dt / G_MODEL_LAG_S).clamp(0.0, 1.0);
        let g_err = self.g_model - g_meas;
        let ramping = (g_cmd - self.g_cmd_ramp).abs() > 1.0;
        let saturated = (self.trim + ff).abs() >= MAX_AUTHORITY - 1e-3;
        if !ramping && !saturated {
            self.trim += SIGN * KI_RATE * slab_per_g * g_err * dt;
        }
        self.trim = self.trim.clamp(-MAX_AUTHORITY, MAX_AUTHORITY);

        // --- final slew-limited output --------------------------------
        let target = (self.trim + ff + damping).clamp(-MAX_AUTHORITY, MAX_AUTHORITY);
        let max_step = MAX_OUTPUT_SLEW_PER_S * dt;
        self.output += (target - self.output).clamp(-max_step, max_step);
        self.output = self.output.clamp(-MAX_AUTHORITY, MAX_AUTHORITY);

        if DEBUG_LOG {
            self.debug_accum += dt;
            if self.debug_accum >= 0.1 {
                self.debug_accum = 0.0;
                eprintln!(
                    "[flcs] spd={:5.0} q={:7.0} slab/g={:.3} | stick_g={:+.2} g_cmd={:+.2}->{:+.2} g_meas={:+.2} err={:+.2} aoa={:+.1} | ff={:+.3} trim={:+.3} damp={:+.3} out={:+.3}",
                    airspeed, q, slab_per_g, stick_g, g_cmd, self.g_cmd_ramp, g_meas,
                    g_err, aoa_deg, ff, self.trim, damping, self.output
                );
            }
        }

        self.output
    }
}
