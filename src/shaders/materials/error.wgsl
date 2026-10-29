// engine/error - what a model is drawn with when its material's shader can't
// be used (missing, doesn't compile, the wrong kind, a pass the frame plan
// doesn't have... - see engine::rendering::shader_library, which says why
// in the console): pure magenta, flashing between full and half brightness
// twice a second, unlit, no fog - so it can't be missed or taken for a real
// color. Same inputs as the template's (see standard_template.wgsl).

struct CameraUniform {
    view_proj: mat4x4<f32>,
    view_pos: vec4<f32>,
};

struct Transform {
    model_matrix: mat4x4<f32>,
};

struct Light {
    position: vec3<f32>,
    color: vec3<f32>,
    time: f32,
    camera_position: vec3<f32>,
}

@group(1) @binding(0)
var<uniform> camera: CameraUniform;

@group(2) @binding(0)
var<uniform> transform: Transform;

@group(3) @binding(0)
var<uniform> light: Light;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) tex_coords: vec2<f32>,
    @location(2) normal: vec3<f32>,
}

struct InstanceInput {
    @location(5) model_matrix_0: vec4<f32>,
    @location(6) model_matrix_1: vec4<f32>,
    @location(7) model_matrix_2: vec4<f32>,
    @location(8) model_matrix_3: vec4<f32>,
    @location(9) normal_matrix_0: vec3<f32>,
    @location(10) normal_matrix_1: vec3<f32>,
    @location(11) normal_matrix_2: vec3<f32>,
};

@vertex
fn vs_main(model: VertexInput, instance: InstanceInput) -> @builtin(position) vec4<f32> {
    let model_matrix = mat4x4<f32>(
        instance.model_matrix_0,
        instance.model_matrix_1,
        instance.model_matrix_2,
        instance.model_matrix_3,
    );
    return camera.view_proj * model_matrix * transform.model_matrix * vec4<f32>(model.position, 1.0);
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    let brightness = select(0.5, 1.0, fract(light.time * 2.0) < 0.5);
    return vec4<f32>(brightness, 0.0, brightness, 1.0);
}
