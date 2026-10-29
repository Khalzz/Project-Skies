// The template every Ui shader is built from for UI images (see engine::
// rendering::shader_library) - the image's color, through the `fragment` hook
// pasted where the HOOKS line is. Group 0 is the material's own, 1 the camera
// (unused here), 2 the image, 3 the light.

// Position arrives already in clip space (converted CPU-side, same convention as
// ui_template.wgsl), so the vertex shader just passes it through.
struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) alpha: f32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) alpha: f32,
}

@vertex
fn vs_main(model: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = vec4<f32>(model.position, 1.0);
    out.uv = model.uv;
    out.alpha = model.alpha;
    return out;
}

@group(2) @binding(0)
var image_texture: texture_2d<f32>;
@group(2) @binding(1)
var image_sampler: sampler;

// The light (group 3) carries the time - what `engine` is filled from.
struct Light {
    position: vec3<f32>,
    color: vec3<f32>,
    time: f32,
    camera_position: vec3<f32>,
    ambient: vec3<f32>,
}

@group(3) @binding(0)
var<uniform> light: Light;

// What the hook can read, as `engine` - see standard_template.wgsl's.
struct Engine {
    time: f32,
    camera_position: vec3<f32>,
    sun_direction: vec3<f32>,
    sun_color: vec3<f32>,
    // The light from the whole sky, falling on every surface whichever
    // way it faces.
    ambient_color: vec3<f32>,
}
var<private> engine: Engine;

fn engine_values() -> Engine {
    return Engine(light.time, light.camera_position, normalize(light.position), light.color, light.ambient);
}

// The `fragment` hook's pixel (see shader_library's module doc):
// - pixel: where it is on the target, in pixels from the top left;
// - uv: 0..1 across the element's box from its top left (an image's: across
//   the image);
// - color: what the engine draws there - fill, border, frosted blur, the
//   soft edge in its alpha. A hook returning it unchanged changes nothing.
struct UiIn {
    pixel: vec2<f32>,
    uv: vec2<f32>,
    color: vec4<f32>,
}

//@HOOKS

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    engine = engine_values();
    var color = textureSample(image_texture, image_sampler, in.uv);
    color.a = color.a * in.alpha;
    return fragment(UiIn(in.clip_position.xy, in.uv, color));
}
