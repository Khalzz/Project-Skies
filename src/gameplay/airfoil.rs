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

    // Sample function to get Cl and Cd based on alpha
    pub fn sample(&self, alpha: f32) -> (f32, f32) {
        let len = self.data.len();
        
        // Get raw float index
        let float_index = self.alpha_to_float_index(alpha); // see below
        
        // Clamp to valid range
        let float_index = float_index.clamp(0.0, (len - 1) as f32);
        
        let lower = float_index.floor() as usize;
        let upper = (lower + 1).min(len - 1);
        let t = float_index.fract(); // interpolation factor 0..1
        
        let a = &self.data[lower];
        let b = &self.data[upper];
        
        let cl = a.y + (b.y - a.y) * t;
        let cd = a.z + (b.z - a.z) * t;
        
        (cl, cd)
    }

    fn alpha_to_index(&self, alpha: f32) -> usize {
        // we get the range between the maximum alpha on the data and the minimum
        let range: f32 = self.max_alpha - self.min_alpha;

        // if the range is 0, means that the max and min alpha are equal in numerical value
        if range == 0.0 {
            return 0; // Avoid division by zero
        }
        
        let normalized_alpha = (alpha - self.min_alpha) / range;
        let scaled_index = (normalized_alpha * (self.data.len() as f32 - 1.0)).round();
        scaled_index as usize
    }

    fn alpha_to_float_index(&self, alpha: f32) -> f32 {
        let range = self.max_alpha - self.min_alpha;
        if range == 0.0 {
            return 0.0;
        }
        let normalized_alpha = (alpha - self.min_alpha) / range;
        normalized_alpha * (self.data.len() as f32 - 1.0)
        // no .round() — keep the fractional part for interpolation
    }
}