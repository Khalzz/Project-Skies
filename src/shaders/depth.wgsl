// Define near and far planes for the depth calculation
const NEAR: f32 = 0.1;  // Near plane distance
const FAR: f32 = 10000.0;  // Far plane distance

struct CameraUniform {
    view_proj: mat4x4<f32>,
};

struct Transform {
    model_matrix: mat4x4<f32>,
};

@group(1) @binding(0)
var<uniform> camera: CameraUniform;

@group(2) @binding(0)
var<uniform> transform: Transform;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) tex_coords: vec2<f32>,
}

struct InstanceInput {
    @location(5) model_matrix_0: vec4<f32>,
    @location(6) model_matrix_1: vec4<f32>,
    @location(7) model_matrix_2: vec4<f32>,
    @location(8) model_matrix_3: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
    @location(1) depth: f32,  // Pass depth to the fragment shader
}

@vertex
fn vs_main(model: VertexInput, instance: InstanceInput) -> VertexOutput {
    let model_matrix = mat4x4<f32>(
        instance.model_matrix_0,
        instance.model_matrix_1,
        instance.model_matrix_2,
        instance.model_matrix_3,
    );

    var out: VertexOutput;
    out.tex_coords = model.tex_coords;

    // Calculate world space position
    let world_position = transform.model_matrix * vec4<f32>(model.position, 1.0);
    
    // Clip space position (with view-projection matrix)
    let clip_position = camera.view_proj * model_matrix * world_position;

    // Get linear depth in normalized device coordinates (NDC)
    let z = clip_position.z / clip_position.w;

    // Adjust the depth range for logarithmic depth and avoid precision issues
    let log_depth = log2(max(z - NEAR + 1.0, 1e-6));

    // Normalize depth to [0, 1] based on the near and far planes
    out.depth = (log_depth - log2(NEAR)) / (log2(FAR + 1.0) - log2(NEAR + 1.0));

    // Set the clip position as usual
    out.clip_position = vec4<f32>(clip_position.xy, clip_position.z, clip_position.w);
    return out;
}

@group(0) @binding(0)
var t_diffuse: texture_2d<f32>;
@group(0) @binding(1)
var s_diffuse: sampler;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(t_diffuse, s_diffuse, in.tex_coords);
}