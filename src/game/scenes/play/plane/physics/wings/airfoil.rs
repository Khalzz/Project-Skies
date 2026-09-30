use nalgebra::Vector3;
use ron::from_str;

/// # Air Foil
///
/// This structure is dedicated to the generation of "lift coefficient" and "drag coefficient" based on data given from http://airfoiltools.com/search/index
/// 
/// **Meaning:** Described as the lateral structure of a wing designed to get the most favourable ratio of lift to drag in flight.
/// 
/// ## Contents:
/// - **data**: A vector 3 containing the next values [alpha (angle of attack), lift coefficient, drag coefficient]
/// - **min_alpha**: minimum angle of attack in the data given
/// - **max_alpha**: maximum angle of attack in the data given
/// 
/// The data values are used to get the exact lift and drag coefficient for each angle of attack and you have to search them for each aircraft to use.
/// 
/// ### Examples
/// 
/// - **F-16**: uses **NACA 64A204**
/// - **F-14**: uses **NACA 64A-112**
/// - **SU-27**: uses **NACA 64A-212**

#[derive(Debug, Clone)]
pub struct AirFoil {
    pub min_alpha: f32,
    pub max_alpha: f32,
    pub data: Vec<Vector3<f32>>
} 

impl AirFoil {
    // this is called once
    pub fn new(data_path: String) -> Self {
        let curve = match std::fs::read_to_string(data_path) {
            Ok(file_contents) => {
                match from_str::<Vec<nalgebra::Vector3<f32>>>(&file_contents) {
                    Ok(data) => {
                        data
                    },
                    _ => {
                        vec![]
                    }
                }
            },
            _ => {
                vec![]
            }
        };

        AirFoil { min_alpha: curve[0].x, max_alpha: curve[curve.len() - 1].x, data: curve }
    }

    /// Cl and Cd at `alpha` (deg), interpolated between the two rows around
    /// it by their own alpha values - so rows don't need to be evenly spaced
    /// (the old row-index mapping assumed they were, and read the wrong row
    /// on a table that wasn't). Clamped to the first/last row outside the
    /// table. Rows must be in increasing alpha.
    pub fn sample(&self, alpha: f32) -> (f32, f32) {
        let len = self.data.len();
        if len == 0 {
            return (0.0, 0.0);
        }
        if alpha <= self.min_alpha {
            return (self.data[0].y, self.data[0].z);
        }
        if alpha >= self.max_alpha {
            return (self.data[len - 1].y, self.data[len - 1].z);
        }

        // First row at or past `alpha` - the one before it is below it.
        let upper = self.data.partition_point(|row| row.x < alpha).clamp(1, len - 1);
        let a = &self.data[upper - 1];
        let b = &self.data[upper];
        let span = b.x - a.x;
        let t = if span > 0.0 { (alpha - a.x) / span } else { 0.0 };

        (a.y + (b.y - a.y) * t, a.z + (b.z - a.z) * t)
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn table(rows: &[(f32, f32, f32)]) -> AirFoil {
        let data: Vec<Vector3<f32>> = rows.iter().map(|&(a, cl, cd)| Vector3::new(a, cl, cd)).collect();
        AirFoil { min_alpha: data[0].x, max_alpha: data[data.len() - 1].x, data }
    }

    #[test]
    fn interpolates_on_uneven_rows() {
        // 0.5 deg steps, then a 2 deg one - the old row-index mapping read
        // the wrong row here.
        let foil = table(&[(0.0, 0.0, 0.01), (0.5, 0.05, 0.01), (1.0, 0.1, 0.01), (3.0, 0.3, 0.03)]);
        let (cl, cd) = foil.sample(2.0);
        assert!((cl - 0.2).abs() < 1e-5 && (cd - 0.02).abs() < 1e-5);
        let (cl, _) = foil.sample(0.75);
        assert!((cl - 0.075).abs() < 1e-5);
    }

    #[test]
    fn clamps_outside_the_table() {
        let foil = table(&[(-1.0, -0.1, 0.02), (0.0, 0.0, 0.01), (1.0, 0.1, 0.02)]);
        assert_eq!(foil.sample(-10.0), (-0.1, 0.02));
        assert_eq!(foil.sample(10.0), (0.1, 0.02));
    }
}
