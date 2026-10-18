// The template every Fullscreen shader is built from (see engine::rendering::
// shader_library): one triangle covering the target, and a `fragment` hook
// the shader fills in - pasted where the HOOKS line is - run for every pixel.
// Its inputs (the images it names in its .ron's `inputs` - "scene", "hud",
// "blurred" - each a `texture_2d` of that name with `<name>_sampler`), its
// params and textures are declared for it in group 0; group 1 is the light,
// which carries the time.

struct Light {
    position: vec3<f32>,
    color: vec3<f32>,
    time: f32,
    camera_position: vec3<f32>,
}

@group(1) @binding(0)
var<uniform> light: Light;

// What the hook can read, as `engine` - see standard_template.wgsl's.
struct Engine {
    time: f32,
    camera_position: vec3<f32>,
    sun_direction: vec3<f32>,
    sun_color: vec3<f32>,
}
var<private> engine: Engine;

fn engine_values() -> Engine {
    return Engine(light.time, light.camera_position, normalize(light.position), light.color);
}

// The `fragment` hook's pixel: where it is - 0..1 across the target from its
// top left, and in pixels.
struct ScreenIn {
    uv: vec2<f32>,
    pixel: vec2<f32>,
}

//@HOOKS

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// One triangle twice the target's size - (0,0), (2,0), (0,2) in uv - so the
// target is all inside it.
@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: VertexOutput;
    out.clip_position = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    engine = engine_values();
    return fragment(ScreenIn(in.uv, in.clip_position.xy));
}
