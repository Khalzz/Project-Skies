//! The pilot's body under G - greying out and blacking out under sustained
//! positive G, redding out under negative G, and passing out (G-LOC) for a
//! few seconds if either is pushed all the way. Tuned like flight games
//! usually do it (DCS, IL-2, Ace Combat's "hard" modes): not the instant G,
//! but how long it's been held past what the pilot can take. A crash hard
//! enough (see IMPACT_FATAL_G) kills the pilot: the view fades to black and
//! stays there.

use crate::engine::rendering::render_pipeline::blur_renderer::ScreenEffects;

/// Positive G the pilot takes indefinitely - a fighter pilot in an anti-G
/// suit, doing the anti-G straining manoeuvre: roughly 4.5-5 G relaxed
/// with no suit, about +1 G from the suit and +2 from straining. Past it,
/// `blackout` builds up - faster the farther past.
const POSITIVE_TOLERANCE_G: f32 = 7.0;
/// Seconds to black out when held at POSITIVE_TOLERANCE_G + 4 (11 G). The
/// build rate scales with how far past the tolerance: 9 G takes 10 s (about
/// what a suited, straining pilot holds 9 G for), 10 G ~6.7 s, 13 G ~3.3 s.
const POSITIVE_ONSET_SECONDS: f32 = 5.0;
/// How fast `blackout` drains back (per second) once under the tolerance.
const POSITIVE_RECOVERY_RATE: f32 = 0.3;

/// Negative G the pilot takes indefinitely - far less than positive. The
/// G-suit doesn't help here: it only squeezes blood up from the legs, which
/// is no use when negative G is pushing it into the head.
const NEGATIVE_TOLERANCE_G: f32 = -1.5;
/// Seconds to red out when held 1.5 G past it (-3 G).
const NEGATIVE_ONSET_SECONDS: f32 = 3.0;
const NEGATIVE_RECOVERY_RATE: f32 = 0.4;

/// Once `blackout` or `redout` reaches 1 the pilot is out for this long -
/// no control, screen fully black/red.
const UNCONSCIOUS_SECONDS: f32 = 5.0;
/// Where `blackout`/`redout` restarts when coming to - the view clears
/// from there as it drains (see the RECOVERY_RATEs).
const WAKE_UP_LEVEL: f32 = 0.75;

/// A crash averaging more than this G (any direction) over the matching
/// impact window kills the pilot - see FlightData::impact_g and its
/// IMPACT_WINDOWS_SECONDS (0.1 / 0.5 / 1 s). The longer it lasts, the less
/// it takes, roughly like the Eiband tolerance charts for a strapped-in
/// body: a sharp 40 G hit, 20 G for half a second, 15 G for a second.
const IMPACT_FATAL_G: [f32; 3] = [40.0, 20.0, 15.0];
/// Once killed, the view goes black over DEATH_FADE_SECONDS (0 = a hard
/// cut), stays black for DEATH_HOLD_SECONDS (the game switches to the wreck
/// camera meanwhile - see `death_view_hidden`), then clears again over
/// DEATH_RETURN_SECONDS, now watching from outside.
const DEATH_FADE_SECONDS: f32 = 0.1;
const DEATH_HOLD_SECONDS: f32 = 1.0;
const DEATH_RETURN_SECONDS: f32 = 1.5;
/// Impacts are ignored for this long after the pilot starts feeling the
/// plane again (spawn, end of a cinematic) - the plane's velocity may jump
/// then (a teleport), which reads as a huge but fake acceleration.
const IMPACT_GRACE_SECONDS: f32 = 0.5;

pub struct Pilot {
    /// 0 = fine .. 1 = blacked out (positive G).
    pub blackout: f32,
    /// 0 = fine .. 1 = redded out (negative G).
    pub redout: f32,
    /// Seconds left passed out, 0 when awake.
    unconscious: f32,
    /// Killed by a crash - see IMPACT_FATAL_G.
    dead: bool,
    /// Seconds since killed - drives the fade out and back (DEATH_*).
    dead_for: f32,
    /// How long the pilot has been feeling the plane, for IMPACT_GRACE_SECONDS.
    feeling_for: f32,
}

impl Pilot {
    pub fn new() -> Self {
        Self { blackout: 0.0, redout: 0.0, unconscious: 0.0, dead: false, dead_for: 0.0, feeling_for: 0.0 }
    }

    /// Killed, and the view is fully black right now - the moment to cut to
    /// another camera without it being seen.
    pub fn death_view_hidden(&self) -> bool {
        self.dead && self.blackout >= 1.0
    }

    /// Passed out or dead - no control over the plane.
    pub fn is_unconscious(&self) -> bool {
        self.unconscious > 0.0 || self.dead
    }

    /// Killed in a crash.
    pub fn is_dead(&self) -> bool {
        self.dead
    }

    /// One frame at load factor `g` (the HUD's G, positive = pushed into
    /// the seat) and crash load `impact_g` (see FlightData::impact_g).
    pub fn update(&mut self, g: f32, impact_g: [f32; 3], delta_time: f32) {
        self.feeling_for += delta_time;
        let fatal = impact_g.iter().zip(IMPACT_FATAL_G).any(|(felt, limit)| *felt > limit);
        if !self.dead && self.feeling_for > IMPACT_GRACE_SECONDS && fatal {
            self.dead = true;
            // The death fade runs on `blackout` - start it from however
            // gone the view already is, so it never brightens first: fully
            // black if already passed out (from either side), otherwise
            // the deeper of the two.
            self.blackout = if self.unconscious > 0.0 { 1.0 } else { self.blackout.max(self.redout) };
            self.redout = 0.0;
            self.unconscious = 0.0;
            println!("Pilot killed on impact ({:.0} / {:.0} / {:.0} G over 0.1 / 0.5 / 1 s)", impact_g[0], impact_g[1], impact_g[2]);
        }
        if self.dead {
            self.dead_for += delta_time;
            let returning = self.dead_for - DEATH_FADE_SECONDS - DEATH_HOLD_SECONDS;
            self.blackout = if self.dead_for < DEATH_FADE_SECONDS {
                self.blackout.max(self.dead_for / DEATH_FADE_SECONDS)
            } else if returning < 0.0 {
                1.0
            } else {
                1.0 - returning / DEATH_RETURN_SECONDS
            }
            .clamp(0.0, 1.0);
            return;
        }

        if self.unconscious > 0.0 {
            self.unconscious -= delta_time;
            if self.unconscious <= 0.0 {
                self.unconscious = 0.0;
                self.blackout = self.blackout.min(WAKE_UP_LEVEL);
                self.redout = self.redout.min(WAKE_UP_LEVEL);
            }
            return;
        }

        if g > POSITIVE_TOLERANCE_G {
            self.blackout += (g - POSITIVE_TOLERANCE_G) / 4.0 / POSITIVE_ONSET_SECONDS * delta_time;
        } else {
            self.blackout -= POSITIVE_RECOVERY_RATE * delta_time;
        }
        if g < NEGATIVE_TOLERANCE_G {
            self.redout += (NEGATIVE_TOLERANCE_G - g) / 1.5 / NEGATIVE_ONSET_SECONDS * delta_time;
        } else {
            self.redout -= NEGATIVE_RECOVERY_RATE * delta_time;
        }
        self.blackout = self.blackout.clamp(0.0, 1.0);
        self.redout = self.redout.clamp(0.0, 1.0);

        if self.blackout >= 1.0 || self.redout >= 1.0 {
            self.unconscious = UNCONSCIOUS_SECONDS;
        }
    }

    /// A frame the pilot feels no G - e.g. after a wreck, where the ordinary
    /// G readings are meaningless: vision slowly comes back (unless dead,
    /// and a crash can still be fatal - see IMPACT_FATAL_G).
    pub fn recover(&mut self, impact_g: [f32; 3], delta_time: f32) {
        self.update(1.0, impact_g, delta_time);
    }

    /// A frame the plane isn't being simulated (a cinematic moving it) -
    /// nothing is felt, and the impact grace restarts, since the plane may
    /// be teleported when it hands back.
    pub fn rest(&mut self, delta_time: f32) {
        self.update(1.0, [0.0; 3], delta_time);
        self.feeling_for = 0.0;
    }

    /// What the pilot sees, as screen effects:
    /// - blacking out (or killed): the colors drain to black and white,
    ///   while a round tunnel closes in from the edges and the whole view
    ///   dims, until everything is black;
    /// - redding out: a dark red tunnel closing in, with the whole view
    ///   washing red.
    pub fn screen_effects(&self) -> ScreenEffects {
        let smoothstep = |edge0: f32, edge1: f32, x: f32| {
            let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };

        let mut effects = ScreenEffects::default();
        if self.redout > self.blackout {
            effects.tunnel = smoothstep(0.1, 1.0, self.redout);
            effects.tunnel_color = [0.12, 0.0, 0.0, 1.0];
            effects.tint = [0.35, 0.0, 0.0, smoothstep(0.25, 1.0, self.redout) * 0.6];
        } else {
            // All three start together; the colors are fully gone a bit
            // before the end, and the tunnel and the dimming finish
            // together on black as the pilot blacks out.
            const START: f32 = 0.05;
            effects.desaturation = smoothstep(START, 0.8, self.blackout);
            effects.tunnel = smoothstep(START, 1.0, self.blackout);
            effects.darkening = smoothstep(START, 1.0, self.blackout);
            effects.tunnel_color = [0.0, 0.0, 0.0, 1.0];
        }
        effects
    }
}
