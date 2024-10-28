// Vertex shader

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) tex_coords: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
}

@vertex
fn vs_main(
    model: VertexInput,
) -> VertexOutput {
    var out: VertexOutput;
    out.tex_coords = model.tex_coords;
    out.clip_position = vec4<f32>(model.position, 1.0);
    return out;
}

// Fragment shader

@group(0) @binding(0)
var t_shadow: texture_2d<f32>;
@group(0) @binding(1)
var s_shadow: sampler;
// Define a uniform structure for near and far values
@group(0) @binding(2)
var<uniform> u_near_far: vec2<f32>; // u_near_far.x = near, u_near_far.y = far

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let near = u_near_far.x; // Get near value from uniform
    let far = u_near_far.y;   // Get far value from uniform
    
    // Sample the depth texture
    let depth = textureSample(t_shadow, s_shadow, in.tex_coords).x;

    // Linearize the depth value from the depth texture
    let z_ndc = depth * 2.0 - 1.0; // Convert depth back to NDC space (-1 to 1)
    let z_view = (2.0 * near * far) / (far + near - z_ndc * (far - near)); // Linearize the depth value

    // Normalize the depth to [0, 1] for visualization
    let depth_normalized = (z_view - near) / (far - near);
    
    // Clamp the normalized depth to [0, 1] to avoid negative values
    let depth_clamped = clamp(depth_normalized, 0.0, 1.0);

    // Return the depth value as a grayscale color
    return vec4<f32>(vec3<f32>(depth_clamped), 1.0);
}