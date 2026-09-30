use nalgebra::Vector3;
use serde::Deserialize;

/// A plane's landing gear - `data.ron`'s `gear: ( ... )`: its wheels, and
/// how far the steerable ones turn.
#[derive(Debug, Clone, Deserialize)]
pub struct GearSpec {
    pub wheels: Vec<WheelSpec>,
    /// Optional - see `SteeringSpec` for the defaults.
    #[serde(default)]
    pub steering: SteeringSpec,
}

/// One wheel: a suspension strut casting straight down from `position`.
#[derive(Debug, Clone, Deserialize)]
pub struct WheelSpec {
    /// The model's mesh for this wheel - moved with the suspension, and up
    /// out of the way when the gear retracts (see GearMeshes).
    pub mesh: String,
    /// Where the strut is mounted, in meters in the jet's frame (+X left,
    /// +Y up, +Z nose). The wheel hangs `suspension_length` below it.
    pub position: Vector3<f32>,
    /// How far down the strut reaches, fully extended (m) - set it so the
    /// modelled tyre's bottom sits just above its end, or the jet rests
    /// lower than its model's wheels.
    pub suspension_length: f32,
    /// Spring force at full compression (N).
    pub stiffness: f32,
    /// Damping against vertical speed (N per m/s).
    pub damping: f32,
    /// Turns with the rudder pedals (the nose wheel).
    #[serde(default)]
    pub steerable: bool,
    /// Has a brake.
    #[serde(default)]
    pub braked: bool,
}

/// Nose-wheel steering authority, scheduled on ground speed: full lock at
/// taxi speed, tapering to a few degrees by takeoff-roll speed, so full
/// pedal at speed doesn't snap the jet sideways.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct SteeringSpec {
    /// Full lock at taxi speed (degrees).
    pub max_angle_deg: f32,
    /// What's left at speed (degrees).
    pub high_speed_angle_deg: f32,
    /// Ground speeds (m/s) the taper runs between.
    pub taper_start_speed: f32,
    pub taper_end_speed: f32,
}

impl Default for SteeringSpec {
    fn default() -> Self {
        // The F-16's nose-wheel steering is ~±32°.
        Self { max_angle_deg: 32.0, high_speed_angle_deg: 4.0, taper_start_speed: 8.0, taper_end_speed: 40.0 }
    }
}

impl SteeringSpec {
    /// Steering lock (degrees) at `ground_speed` (m/s).
    pub fn max_angle_at(&self, ground_speed: f32) -> f32 {
        let span = (self.taper_end_speed - self.taper_start_speed).max(1e-3);
        let t = ((ground_speed - self.taper_start_speed) / span).clamp(0.0, 1.0);
        self.max_angle_deg + (self.high_speed_angle_deg - self.max_angle_deg) * t
    }
}
