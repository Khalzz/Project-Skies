use nalgebra::{UnitQuaternion, Vector3};
use serde::Deserialize;

/// A plane's landing gear - `data.ron`'s `gear: ( ... )`: its wheels, and
/// how far the steerable ones turn.
///
/// A wheel's spring and damper can be worked out from the jet's weight
/// instead of typed in (see `resolve`): `static_compression` is how far
/// every strut sinks under its share of the weight sitting still, and
/// `damping_ratio` how much of the damping that just stops a strut
/// bouncing ("critical") each gets. A wheel can set either for itself, or
/// give its own `stiffness`/`damping` outright.
#[derive(Debug, Clone, Deserialize)]
pub struct GearSpec {
    pub wheels: Vec<WheelSpec>,
    /// How far each strut sinks at rest under its share of the weight (m) -
    /// the same on every wheel keeps the jet's attitude on the ground the
    /// one its wheel positions give. Sets how hard the springs are: the
    /// bounce is sqrt(9.81 / this) rad/s (0.15 m: ~1.3 Hz).
    #[serde(default)]
    pub static_compression: Option<f32>,
    /// Fraction of critical damping (1.0: just doesn't bounce). Under ~0.5 a
    /// strut bobs; over 1 it's harsh over bumps.
    #[serde(default = "default_damping_ratio")]
    pub damping_ratio: f32,
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
    /// Spring force at full compression (N) - left out (0), it's worked out
    /// from the weight this wheel carries (see `GearSpec::resolve`).
    #[serde(default)]
    pub stiffness: f32,
    /// Damping against vertical speed (N per m/s) - left out (0), it's
    /// worked out from the spring and the weight this wheel carries.
    #[serde(default)]
    pub damping: f32,
    /// This wheel's own `GearSpec::static_compression`.
    #[serde(default)]
    pub static_compression: Option<f32>,
    /// This wheel's own `GearSpec::damping_ratio`.
    #[serde(default)]
    pub damping_ratio: Option<f32>,
    /// Where the wheel's middle sits with the gear up (m, the jet's frame) -
    /// in its bay. Left out, it tucks straight up to `position`. Only the
    /// look: the suspension stays mounted at `position`.
    #[serde(default)]
    pub retracted_position: Option<Vector3<f32>>,
    /// How the wheel is turned with the gear up: degrees about the jet's X
    /// (left), Y (up) and Z (nose) axes, applied in that order, about the
    /// wheel's own middle. Left out, it isn't turned.
    #[serde(default)]
    pub retracted_rotation: Option<Vector3<f32>>,
    /// Turns with the rudder pedals (the nose wheel).
    #[serde(default)]
    pub steerable: bool,
    /// Has a brake.
    #[serde(default)]
    pub braked: bool,
}

impl WheelSpec {
    /// `retracted_rotation` as a rotation in the jet's frame - X, then Y,
    /// then Z. Identity when it's left out.
    pub fn retracted_turn(&self) -> UnitQuaternion<f32> {
        let Some(degrees) = self.retracted_rotation else { return UnitQuaternion::identity() };
        UnitQuaternion::from_axis_angle(&Vector3::z_axis(), degrees.z.to_radians())
            * UnitQuaternion::from_axis_angle(&Vector3::y_axis(), degrees.y.to_radians())
            * UnitQuaternion::from_axis_angle(&Vector3::x_axis(), degrees.x.to_radians())
    }
}

fn default_damping_ratio() -> f32 {
    0.7
}

const GRAVITY: f64 = 9.81;

impl GearSpec {
    /// Fills in every wheel's `stiffness` and `damping` it didn't give
    /// itself, from the jet: `mass` (kg, tanks full - burning fuel off later
    /// just lets it sit a little higher, as a real strut would) and its
    /// `center_of_mass` (the jet's frame). Each wheel's share of the weight
    /// is what holds the jet still on the ground: the shares add up to the
    /// weight and balance it in pitch and roll (see `static_loads`).
    pub fn resolve(&mut self, mass: f32, center_of_mass: Vector3<f32>) -> Result<(), String> {
        if self.wheels.iter().all(|wheel| wheel.stiffness > 0.0 && wheel.damping > 0.0) {
            return Ok(());
        }
        let loads = static_loads(&self.wheels, mass, center_of_mass)?;
        for (wheel, load) in self.wheels.iter_mut().zip(loads) {
            // A strut's force is its compressed fraction of
            // `suspension_length` times `stiffness`, so its rate (N/m) is
            // stiffness / suspension_length.
            if wheel.stiffness <= 0.0 {
                let sag = wheel.static_compression.or(self.static_compression).ok_or_else(|| {
                    format!("gear wheel '{}': no `stiffness` and no `static_compression` (its own or the gear's) to work one out from", wheel.mesh)
                })?;
                if !(sag > 0.0 && sag < wheel.suspension_length) {
                    return Err(format!("gear wheel '{}': static_compression {sag} m has to be more than 0 and less than its suspension_length {}", wheel.mesh, wheel.suspension_length));
                }
                wheel.stiffness = (load / sag as f64 * wheel.suspension_length as f64) as f32;
            }
            if wheel.damping <= 0.0 {
                let rate = wheel.stiffness as f64 / wheel.suspension_length as f64;
                let critical = 2.0 * (rate * load / GRAVITY).sqrt();
                wheel.damping = (wheel.damping_ratio.unwrap_or(self.damping_ratio) as f64 * critical) as f32;
            }
        }
        Ok(())
    }
}

/// How much of the jet's weight (N) each wheel carries sitting still: the
/// loads that add up to the weight and put no pitch or roll moment about
/// the center of mass. With three wheels that's the only answer; with more,
/// the most even one.
fn static_loads(wheels: &[WheelSpec], mass: f32, center_of_mass: Vector3<f32>) -> Result<Vec<f64>, String> {
    use nalgebra::{Matrix3, Vector3 as V3};
    let weight = mass as f64 * GRAVITY;
    // Each wheel's row: its share of the vertical sum, and its lever from
    // the center of mass along the nose (z) and across the wings (x).
    let rows: Vec<V3<f64>> = wheels.iter()
        .map(|wheel| V3::new(1.0, (wheel.position.z - center_of_mass.z) as f64, (wheel.position.x - center_of_mass.x) as f64))
        .collect();
    let gram: Matrix3<f64> = rows.iter().map(|row| row * row.transpose()).sum();
    let inverse = gram.try_inverse().ok_or("gear: the wheels need to stand on three points not all in a line (nose and two mains)")?;
    let multipliers = inverse * V3::new(weight, 0.0, 0.0);
    let loads: Vec<f64> = rows.iter().map(|row| row.dot(&multipliers)).collect();
    if let Some((wheel, load)) = wheels.iter().zip(&loads).find(|(_, load)| **load <= 0.0) {
        return Err(format!("gear wheel '{}' would carry {load:.0} N - the center of mass has to sit between the wheels (mains behind it, nose ahead)", wheel.mesh));
    }
    Ok(loads)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn wheel(mesh: &str, x: f32, z: f32) -> WheelSpec {
        WheelSpec {
            mesh: mesh.into(),
            position: Vector3::new(x, 0.0, z),
            suspension_length: 2.0,
            stiffness: 0.0,
            damping: 0.0,
            static_compression: None,
            damping_ratio: None,
            retracted_position: None,
            retracted_rotation: None,
            steerable: false,
            braked: false,
        }
    }

    fn gear() -> GearSpec {
        GearSpec {
            wheels: vec![wheel("nose", 0.0, 5.0), wheel("left", -1.5, 1.0), wheel("right", 1.5, 1.0)],
            static_compression: Some(0.2),
            damping_ratio: 0.7,
            steering: SteeringSpec::default(),
        }
    }

    #[test]
    fn the_wheels_share_the_weight_by_their_levers() {
        // Center of mass a quarter of the way from the mains to the nose:
        // the nose carries a quarter, each main three eighths.
        let loads = static_loads(&gear().wheels, 1000.0, Vector3::new(0.0, 0.0, 2.0)).unwrap();
        let weight = 1000.0 * GRAVITY;
        assert!((loads.iter().sum::<f64>() - weight).abs() < 1e-6);
        assert!((loads[0] - weight * 0.25).abs() < 1e-6, "{loads:?}");
        assert!((loads[1] - loads[2]).abs() < 1e-6);
    }

    #[test]
    fn every_strut_sinks_the_same_and_is_damped_alike() {
        let mut gear = gear();
        gear.resolve(1000.0, Vector3::new(0.0, 0.0, 2.0)).unwrap();
        let loads = static_loads(&gear.wheels, 1000.0, Vector3::new(0.0, 0.0, 2.0)).unwrap();
        for (wheel, load) in gear.wheels.iter().zip(loads) {
            let rate = wheel.stiffness as f64 / wheel.suspension_length as f64;
            assert!((load / rate - 0.2).abs() < 1e-4, "{}: sinks {}", wheel.mesh, load / rate);
            let critical = 2.0 * (rate * load / GRAVITY).sqrt();
            assert!((wheel.damping as f64 / critical - 0.7).abs() < 1e-4, "{}", wheel.mesh);
        }
    }

    #[test]
    fn a_wheel_keeps_what_it_gives_itself() {
        let mut gear = gear();
        gear.wheels[0].stiffness = 12_345.0;
        gear.wheels[0].damping = 678.0;
        gear.resolve(1000.0, Vector3::new(0.0, 0.0, 2.0)).unwrap();
        assert_eq!(gear.wheels[0].stiffness, 12_345.0);
        assert_eq!(gear.wheels[0].damping, 678.0);
    }

    #[test]
    fn a_center_of_mass_outside_the_wheels_is_refused() {
        assert!(gear().resolve(1000.0, Vector3::new(0.0, 0.0, -1.0)).is_err());
    }
}
