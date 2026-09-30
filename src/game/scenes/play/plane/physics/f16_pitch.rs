//! F-16 toy pitch / turn-rate model (illustrative, NOT official data).
//!
//! Rust port of `f16_pitch_rate_model.py`. Calibrated to the flight-manual
//! "Turn Performance - Sea Level" chart (22,000 lb, drag index 0).
//!
//! Max pitch rate is set by whichever limit hits first:
//!   1. Lift limit: at low speed the wing can't make enough lift for many g's.
//!   2. g limit: above corner speed the FCS caps load factor (~9 g), so pitch
//!      rate falls as speed rises.
//!
//! Units: true airspeed in m/s, altitude in meters, rates in deg/s.
//! No external crates. `f16_pitch.rs` can be dropped in as a module.

pub const G: f64 = 9.80665;
pub const LB_TO_KG: f64 = 0.453592;
pub const FT_TO_M: f64 = 0.3048;
pub const KT_TO_MS: f64 = 0.514444;

const P0: f64 = 101_325.0; // sea-level pressure, Pa
const A0: f64 = 340.294; // sea-level speed of sound, m/s

// ---------------------------------------------------------------------------
// Parameters
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct PitchParams {
    pub mass_kg: f64,
    pub wing_area_m2: f64,
    /// FCS positive load-factor cap.
    pub g_limit: f64,
    /// FCS negative load-factor cap (magnitude, e.g. 3.0 for -3 g).
    pub neg_g_limit: f64,
    /// Nose-down lift is weaker than nose-up; fraction of CLmax available.
    pub neg_cl_ratio: f64,
    /// Rounds the corner where the lift limit meets the g limit.
    pub corner_softness: f64,
    /// Airspeed limit, knots calibrated airspeed.
    pub max_kcas: f64,
    /// Mach limit.
    pub max_mach: f64,
}

impl Default for PitchParams {
    fn default() -> Self {
        Self {
            mass_kg: 22_000.0 * LB_TO_KG,
            wing_area_m2: 27.87,
            g_limit: 9.0,
            neg_g_limit: 3.0,
            neg_cl_ratio: 0.6, // illustrative guess, not from the chart
            corner_softness: 0.04,
            max_kcas: 800.0,
            max_mach: 2.05,
        }
    }
}

// ---------------------------------------------------------------------------
// Atmosphere and airspeed conversions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct Atmosphere {
    pub density: f64,        // kg/m^3
    pub pressure: f64,       // Pa
    pub speed_of_sound: f64, // m/s
}

/// ISA standard atmosphere (troposphere + isothermal stratosphere).
pub fn isa(altitude_m: f64) -> Atmosphere {
    let h = altitude_m.min(11_000.0);
    let t = 288.15 - 0.0065 * h;
    let mut rho = 1.225 * (t / 288.15).powf(4.2559);
    let mut p = P0 * (1.0 - 2.25577e-5 * h).powf(5.25588);
    if altitude_m > 11_000.0 {
        let f = (-(altitude_m - 11_000.0) / 6341.6).exp();
        rho *= f;
        p *= f;
    }
    Atmosphere {
        density: rho,
        pressure: p,
        speed_of_sound: (1.4 * 287.05 * t).sqrt(),
    }
}

/// Pitot impact pressure (isentropic below Mach 1, Rayleigh above).
pub fn impact_pressure(mach: f64, static_pressure: f64) -> f64 {
    if mach < 1.0 {
        static_pressure * ((1.0 + 0.2 * mach * mach).powf(3.5) - 1.0)
    } else {
        let m2 = mach * mach;
        static_pressure * (166.92158 * m2.powf(3.5) / (7.0 * m2 - 1.0).powf(2.5) - 1.0)
    }
}

/// Calibrated airspeed (m/s) for a given Mach and altitude.
pub fn cas_from_mach(mach: f64, altitude_m: f64) -> f64 {
    let qc = impact_pressure(mach, isa(altitude_m).pressure);
    // Find the sea-level Mach that gives the same impact pressure.
    let (mut lo, mut hi) = (0.0_f64, 5.0_f64);
    for _ in 0..60 {
        let mid = 0.5 * (lo + hi);
        if impact_pressure(mid, P0) < qc { lo = mid } else { hi = mid }
    }
    0.5 * (lo + hi) * A0
}

// ---------------------------------------------------------------------------
// Monotone cubic (PCHIP) interpolation, same scheme as SciPy's PchipInterpolator
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Pchip {
    x: Vec<f64>,
    y: Vec<f64>,
    d: Vec<f64>,
}

impl Pchip {
    fn new(x: &[f64], y: &[f64]) -> Self {
        let n = x.len();
        assert!(n >= 3 && y.len() == n);
        let h: Vec<f64> = (0..n - 1).map(|i| x[i + 1] - x[i]).collect();
        let m: Vec<f64> = (0..n - 1).map(|i| (y[i + 1] - y[i]) / h[i]).collect();
        let mut d = vec![0.0; n];
        for k in 1..n - 1 {
            if m[k - 1] * m[k] > 0.0 {
                let w1 = 2.0 * h[k] + h[k - 1];
                let w2 = h[k] + 2.0 * h[k - 1];
                d[k] = (w1 + w2) / (w1 / m[k - 1] + w2 / m[k]);
            }
        }
        let end = |h0: f64, h1: f64, m0: f64, m1: f64| {
            let mut e = ((2.0 * h0 + h1) * m0 - h0 * m1) / (h0 + h1);
            if e.signum() != m0.signum() || m0 == 0.0 {
                e = 0.0;
            } else if m0.signum() != m1.signum() && e.abs() > 3.0 * m0.abs() {
                e = 3.0 * m0;
            }
            e
        };
        d[0] = end(h[0], h[1], m[0], m[1]);
        d[n - 1] = end(h[n - 2], h[n - 3], m[n - 2], m[n - 3]);
        Self { x: x.to_vec(), y: y.to_vec(), d }
    }

    /// Evaluate, clamping x to the table range.
    fn eval(&self, x: f64) -> f64 {
        let n = self.x.len();
        let x = x.clamp(self.x[0], self.x[n - 1]);
        let i = match self.x.iter().rposition(|&xi| xi <= x) {
            Some(i) if i < n - 1 => i,
            _ => n - 2,
        };
        let h = self.x[i + 1] - self.x[i];
        let t = (x - self.x[i]) / h;
        let (t2, t3) = (t * t, t * t * t);
        (2.0 * t3 - 3.0 * t2 + 1.0) * self.y[i]
            + (t3 - 2.0 * t2 + t) * h * self.d[i]
            + (-2.0 * t3 + 3.0 * t2) * self.y[i + 1]
            + (t3 - t2) * h * self.d[i + 1]
    }
}

// Max usable lift coefficient vs Mach. Values below Mach 0.55 come from a
// smooth fit to points read (by eye) off the manual chart; above that they
// are hand-picked extrapolations.
const CL_MACH: [f64; 12] = [0.10, 0.20, 0.30, 0.40, 0.50, 0.55, 0.65, 0.80, 1.00, 1.20, 1.40, 2.10];
const CL_VALS: [f64; 12] = [
    1.77851, 1.77851, 1.71999, 1.58174, 1.41520, 1.32133,
    1.22, 1.08, 0.90, 0.80, 0.74, 0.62,
];

// ---------------------------------------------------------------------------
// The model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct F16PitchModel {
    pub params: PitchParams,
    cl_curve: Pchip,
}

impl Default for F16PitchModel {
    fn default() -> Self {
        Self::new(PitchParams::default())
    }
}

/// Numerically stable ln(e^a + e^b).
fn log_add_exp(a: f64, b: f64) -> f64 {
    let m = a.max(b);
    m + ((a - m).exp() + (b - m).exp()).ln()
}

/// Smooth minimum of `n` and `limit` (rounded corner instead of a hard kink).
fn soft_min(n: f64, limit: f64, softness: f64) -> f64 {
    let s = softness * limit;
    -s * log_add_exp(-n / s, -limit / s)
}

impl F16PitchModel {
    pub fn new(params: PitchParams) -> Self {
        Self { params, cl_curve: Pchip::new(&CL_MACH, &CL_VALS) }
    }

    pub fn cl_max(&self, mach: f64) -> f64 {
        self.cl_curve.eval(mach)
    }

    /// Highest Mach allowed at this altitude: lower of the KCAS and Mach limits.
    pub fn max_mach_limit(&self, altitude_m: f64) -> f64 {
        let p = &self.params;
        let limit_cas = p.max_kcas * KT_TO_MS;
        if cas_from_mach(p.max_mach, altitude_m) <= limit_cas {
            return p.max_mach;
        }
        let (mut lo, mut hi) = (0.0, p.max_mach);
        for _ in 0..50 {
            let mid = 0.5 * (lo + hi);
            if cas_from_mach(mid, altitude_m) < limit_cas { lo = mid } else { hi = mid }
        }
        lo
    }

    /// True if the jet is beyond its airspeed/Mach limit (for warnings/damage).
    pub fn is_overspeed(&self, tas_ms: f64, altitude_m: f64) -> bool {
        tas_ms / isa(altitude_m).speed_of_sound > self.max_mach_limit(altitude_m)
    }

    /// Load factor the wing can make at max lift (no g cap).
    fn aero_load_factor(&self, tas_ms: f64, altitude_m: f64) -> f64 {
        let atm = isa(altitude_m);
        let q = 0.5 * atm.density * tas_ms * tas_ms;
        let cl = self.cl_max(tas_ms / atm.speed_of_sound);
        q * self.params.wing_area_m2 * cl / (self.params.mass_kg * G)
    }

    /// Max positive load factor: smooth min of lift limit and g limit.
    pub fn max_load_factor(&self, tas_ms: f64, altitude_m: f64) -> f64 {
        let p = &self.params;
        soft_min(self.aero_load_factor(tas_ms, altitude_m), p.g_limit, p.corner_softness)
    }

    /// Max negative load factor magnitude (nose-down). Illustrative.
    pub fn max_neg_load_factor(&self, tas_ms: f64, altitude_m: f64) -> f64 {
        let p = &self.params;
        let n = self.aero_load_factor(tas_ms, altitude_m) * p.neg_cl_ratio;
        soft_min(n, p.neg_g_limit, p.corner_softness)
    }

    /// Max nose-up pitch rate from lift alone: n*g/V (deg/s).
    /// Gravity is left out on purpose - your physics engine adds it.
    /// Goes to zero as speed goes to zero.
    pub fn max_pitch_rate_deg_s(&self, tas_ms: f64, altitude_m: f64) -> f64 {
        let v = tas_ms.max(1e-3);
        (self.max_load_factor(v, altitude_m) * G / v).to_degrees()
    }

    /// Max nose-down pitch rate magnitude (deg/s).
    pub fn max_neg_pitch_rate_deg_s(&self, tas_ms: f64, altitude_m: f64) -> f64 {
        let v = tas_ms.max(1e-3);
        (self.max_neg_load_factor(v, altitude_m) * G / v).to_degrees()
    }

    /// Instantaneous level-turn rate g*sqrt(n^2-1)/V - what the manual plots.
    pub fn level_turn_rate_deg_s(&self, tas_ms: f64, altitude_m: f64) -> f64 {
        let v = tas_ms.max(1e-3);
        let n = self.max_load_factor(v, altitude_m);
        (G * (n * n - 1.0).max(0.0).sqrt() / v).to_degrees()
    }

    /// Pitch-rate target for a stick input in [-1, 1] (positive = nose up).
    pub fn commanded_pitch_rate_deg_s(&self, stick: f64, tas_ms: f64, altitude_m: f64) -> f64 {
        let s = stick.clamp(-1.0, 1.0);
        if s >= 0.0 {
            s * self.max_pitch_rate_deg_s(tas_ms, altitude_m)
        } else {
            s * self.max_neg_pitch_rate_deg_s(tas_ms, altitude_m)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests: values checked against the Python version
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() < tol, "{a} vs {b}");
    }

    #[test]
    fn matches_python_sea_level() {
        let m = F16PitchModel::default();
        let a = isa(0.0).speed_of_sound;
        // (Mach, pitch deg/s, level turn deg/s, g) from the Python script
        let cases = [
            (0.20, 11.8636, 8.5198, 1.4370),
            (0.30, 17.2099, 16.3060, 3.1269),
            (0.60, 24.3231, 24.1669, 8.8385),
            (0.90, 16.5117, 16.4094, 9.0000),
            (1.21, 12.2814, 12.2054, 9.0000),
        ];
        for (mach, pitch, turn, g) in cases {
            let v = mach * a;
            close(m.max_pitch_rate_deg_s(v, 0.0), pitch, 0.02);
            close(m.level_turn_rate_deg_s(v, 0.0), turn, 0.02);
            close(m.max_load_factor(v, 0.0), g, 0.005);
        }
    }

    #[test]
    fn matches_python_15k() {
        let m = F16PitchModel::default();
        let h = 15_000.0 * FT_TO_M;
        let v = 0.85 * isa(h).speed_of_sound;
        close(m.max_pitch_rate_deg_s(v, h), 17.2716, 0.02);
    }

    #[test]
    fn speed_limits() {
        let m = F16PitchModel::default();
        close(m.max_mach_limit(0.0), 1.21, 0.01);
        close(m.max_mach_limit(10_000.0 * FT_TO_M), 1.41, 0.01);
        close(m.max_mach_limit(20_000.0 * FT_TO_M), 1.68, 0.01);
        close(m.max_mach_limit(40_000.0 * FT_TO_M), 2.05, 0.001);
        assert!(m.is_overspeed(1.3 * A0, 0.0));
        assert!(!m.is_overspeed(1.1 * A0, 0.0));
    }

    #[test]
    fn no_pitch_when_stopped() {
        let m = F16PitchModel::default();
        assert!(m.max_pitch_rate_deg_s(0.0, 0.0) < 0.01);
        assert!(m.commanded_pitch_rate_deg_s(1.0, 0.0, 0.0).abs() < 0.01);
    }
}
