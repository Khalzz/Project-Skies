use nalgebra::Vector3;

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
    pub fn new(curve: Vec<Vector3<f32>>) -> Self {
        AirFoil { min_alpha: curve[0].x, max_alpha: curve[curve.len() - 1].x, data: curve }
    }

    // Sample function to get Cl and Cd based on alpha
    pub fn sample(&self, alpha: f32) -> (f32, f32) {
        // Scale alpha to find index
        let scaled_index = self.scale(alpha);

        // Clamp index to ensure it's within bounds
        let i = scaled_index.clamp(0, self.data.len() as isize - 1) as usize;

        // Return Cl and Cd from data
        (self.data[i].y, self.data[i].z)
    }

    // Scale method similar to C++ version
    fn scale(&self, alpha: f32) -> isize {
        let range = self.max_alpha - self.min_alpha;
        if range == 0.0 {
            return 0; // Avoid division by zero
        }
        
        // Scale alpha to index range
        let scaled_value = ((alpha - self.min_alpha) / range * (self.data.len() as f32 - 1.0)).round();
        scaled_value as isize
    }
}