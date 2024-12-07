use nalgebra::Vector3;

use crate::primitive::manual_vertex::ManualVertex;

/// # Render Basic Line
/// 
/// This function creates a line from point A to point B, and defines the color for it
pub fn render_basic_line(renderizable_lines: &mut Vec<[ManualVertex; 2]>, start_position: Vector3<f32>, start_color: [f32; 3], end_position: Vector3<f32>, end_color: [f32; 3]) {
    let start_vertex = ManualVertex {
        position: start_position.into(),
        color: start_color, // e.g., green for force vectors
    };
    let end_vertex = ManualVertex {
        position: end_position.into(),
        color: end_color,
    };

    renderizable_lines.push([start_vertex, end_vertex])
}
