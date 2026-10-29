// Lines (see rendering::lines) - each point in the world, made relative to
// the camera here, in the shader, where every 3D draw works.

struct Light {
    position: vec3<f32>,
    color: vec3<f32>,
    time: f32,
    camera_position: vec3<f32>,
}

struct CameraUniform {
    view_proj: mat4x4<f32>,
    view_pos: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> light: Light;

@group(1) @binding(0)
var<uniform> camera: CameraUniform;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.position = camera.view_proj * vec4<f32>(input.position - light.camera_position, 1.0);
    output.color = vec4<f32>(input.color, 1.0);
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return input.color;
}
