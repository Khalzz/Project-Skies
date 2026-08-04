
/// # To axis
/// 
/// This function transforms 2 boolean inputs into an axis value.
/// 
/// Input:
/// - Minimal_input: boolean
/// - Maximal_input: boolean
/// 
/// Output:
/// - Axis: f32
pub fn to_axis(minimal_input: bool, maximal_input: bool) -> f32 {
    if minimal_input && maximal_input {
        return 0.0;
    }

    if minimal_input {
        return -1.0;
    }

    if maximal_input {
        return 1.0;
    }

    return 0.0;
}