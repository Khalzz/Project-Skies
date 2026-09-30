use nalgebra::Vector3;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct WingSpec {
    pub label: String,
    /// The wing's planform, in meters - when given, `pressure_center`,
    /// `wing_area` and `chord` are worked out from it (see `resolve_shape`)
    /// and needn't be written. Without it, they're given directly.
    #[serde(default)]
    pub shape: Option<WingShape>,
    /// Where the lift acts, as (span fraction, chord fraction) of the
    /// `shape` - only to override the computed aerodynamic center (quarter
    /// of the mean aerodynamic chord). Ignored without a `shape`.
    #[serde(default)]
    pub aerodynamic_center: Option<(f32, f32)>,
    /// Where the lift acts, in the jet's frame - computed from `shape` when
    /// there is one.
    #[serde(default)]
    pub pressure_center: Vector3<f32>,
    /// With a `shape`: 0 (left out) = its computed area; anything else
    /// overrides it (e.g. a published reference area).
    #[serde(default)]
    pub wing_area: f32,
    /// With a `shape`: its mean aerodynamic chord, computed.
    #[serde(default)]
    pub chord: f32,
    pub airfoil_path: String,
    pub normal: Vector3<f32>,
    pub is_roll_axis: bool,
    pub stable: bool,
    pub incidence_angle: f32,
    pub control_surface_area: f32,
    pub surfaces: Vec<ControlSurfaceSpec>,
}

impl WingSpec {
    /// Works `pressure_center`, `wing_area` and `chord` out of `shape`, when
    /// there is one - called when a plane's data.ron is loaded. A shaped
    /// wing's `span_axis`-direction comes from `normal`, signed by its span.
    pub fn resolve_shape(&mut self) -> Result<(), String> {
        let Some(shape) = &self.shape else { return Ok(()) };
        shape.validate().map_err(|error| format!("wing '{}': {error}", self.label))?;
        if self.wing_area <= 0.0 {
            self.wing_area = shape.area();
        }
        self.chord = shape.mean_aerodynamic_chord();
        // An all-moving surface (any control_surface_area) moves as a whole:
        // its moving area IS its area. Anything else would scale every
        // control input by the mismatch (control_surface_area / wing_area).
        if self.control_surface_area > 0.0 {
            self.control_surface_area = self.wing_area;
        }
        let (span_fraction, chord_fraction) = self.aerodynamic_center.unwrap_or((shape.mac_span_fraction(), 0.25));
        self.pressure_center = shape.point(&self.normal, span_fraction, chord_fraction);
        Ok(())
    }
}

/// A wing's planform - its real size and shape, in meters in the jet's
/// frame (+Z = nose).
#[derive(Debug, Clone, Deserialize)]
pub enum WingShape {
    /// A straight-tapered wing - the usual way real wings are described. A
    /// rectangle is `root_chord == tip_chord` with no sweep.
    /// - `root_leading_edge`: the leading edge's root corner. For the
    ///   conventional reference area, at the centerline (x = 0), with the
    ///   edges carried in through the fuselage.
    /// - `span`: root to tip along the wing's `normal` - negative for a wing
    ///   reaching the other way (a right wing, +X being left).
    /// - `root_chord`/`tip_chord`: front-to-back depth at each end.
    /// - `sweep_deg`: how far back the leading edge sweeps.
    Trapezoid {
        root_leading_edge: Vector3<f32>,
        span: f32,
        root_chord: f32,
        tip_chord: f32,
        sweep_deg: f32,
    },
}

impl WingShape {
    fn validate(&self) -> Result<(), String> {
        let WingShape::Trapezoid { span, root_chord, tip_chord, .. } = self;
        if *span == 0.0 || *root_chord <= 0.0 || *tip_chord < 0.0 {
            return Err("its Trapezoid needs a non-zero span, a positive root chord and a non-negative tip chord".to_owned());
        }
        Ok(())
    }

    /// Root to tip (the wing's `normal`, signed by its span).
    pub fn span_axis(&self, normal: &Vector3<f32>) -> Vector3<f32> {
        let WingShape::Trapezoid { span, .. } = self;
        normal * span.signum()
    }

    /// The wing's depth `span_fraction` (0 root .. 1 tip) of the way out.
    pub fn chord_at(&self, span_fraction: f32) -> f32 {
        let WingShape::Trapezoid { root_chord, tip_chord, .. } = self;
        root_chord + (tip_chord - root_chord) * span_fraction
    }

    /// The leading edge `span_fraction` of the way out.
    pub fn leading_edge_at(&self, normal: &Vector3<f32>, span_fraction: f32) -> Vector3<f32> {
        let WingShape::Trapezoid { root_leading_edge, span, sweep_deg, .. } = self;
        let out = span.abs() * span_fraction;
        root_leading_edge + self.span_axis(normal) * out - Vector3::z() * (out * sweep_deg.to_radians().tan())
    }

    /// The point `span_fraction` of the way out, `chord_fraction` of the
    /// way back from the leading edge (0) to the trailing edge (1).
    pub fn point(&self, normal: &Vector3<f32>, span_fraction: f32, chord_fraction: f32) -> Vector3<f32> {
        self.leading_edge_at(normal, span_fraction) - Vector3::z() * (chord_fraction * self.chord_at(span_fraction))
    }

    /// Planform area: the average of the root and tip chords, times the span.
    pub fn area(&self) -> f32 {
        let WingShape::Trapezoid { span, root_chord, tip_chord, .. } = self;
        (root_chord + tip_chord) * 0.5 * span.abs()
    }

    /// The area between two span fractions.
    pub fn strip_area(&self, from: f32, to: f32) -> f32 {
        let WingShape::Trapezoid { span, .. } = self;
        (self.chord_at(from) + self.chord_at(to)) * 0.5 * span.abs() * (to - from).abs()
    }

    fn taper(&self) -> f32 {
        let WingShape::Trapezoid { root_chord, tip_chord, .. } = self;
        tip_chord / root_chord
    }

    /// Mean aerodynamic chord - the one chord that stands in for the whole
    /// tapered wing: 2/3 root chord (1 + t + t^2) / (1 + t), t = tip/root.
    pub fn mean_aerodynamic_chord(&self) -> f32 {
        let WingShape::Trapezoid { root_chord, .. } = self;
        let t = self.taper();
        2.0 / 3.0 * root_chord * (1.0 + t + t * t) / (1.0 + t)
    }

    /// How far out (0 root .. 1 tip) that mean chord sits:
    /// (1 + 2t) / (3 (1 + t)). The lift acts at this station, a quarter of
    /// the way back along it (the aerodynamic center).
    pub fn mac_span_fraction(&self) -> f32 {
        let t = self.taper();
        (1.0 + 2.0 * t) / (3.0 * (1.0 + t))
    }

    /// Its outline: root leading edge, tip leading edge, tip trailing edge,
    /// root trailing edge.
    pub fn corners(&self, normal: &Vector3<f32>) -> [Vector3<f32>; 4] {
        [self.point(normal, 0.0, 0.0), self.point(normal, 1.0, 0.0), self.point(normal, 1.0, 1.0), self.point(normal, 0.0, 1.0)]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub enum SurfaceKind {
    Aileron,
    Flap,
    /// On a fin - follows the pedals.
    Rudder,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ControlSurfaceSpec {
    pub label: String,
    pub kind: SurfaceKind,
    pub span_start: f32,
    pub span_end: f32,
    pub chord_ratio: f32,
    pub max_deflection_deg: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AeroSpec {
    pub wings: Vec<WingSpec>,
    pub pilot_position: Vector3<f32>,
}
