// The water example's surface - a Surface shader's hooks (see
// engine::rendering::shader_library), settings in water.ron. The waves are
// the same ones the buoyancy in water.rs floats things on: `params.waves`
// is filled from its WAVES, and `wave_height` here is its `wave_height`.

struct Params {
    // Each wave: direction x, direction z, wavelength (m), height (m).
    waves: array<vec4<f32>, 3>,
    shallow: vec4<f32>,
    deep: vec4<f32>,
}

// The depth drawn so far (water.ron's `reads: [Depth]`) - read with
// textureLoad, one exact texel.
@group(0) @binding(16)
var scene_depth: texture_2d<f32>;
@group(0) @binding(17)
var scene_depth_sampler: sampler;

const GRAVITY: f32 = 9.81;
const TAU: f32 = 6.2831853;
const SKY: vec3<f32> = vec3<f32>(0.60, 0.75, 0.90);
const FOAM: vec3<f32> = vec3<f32>(0.95, 0.97, 1.0);

// A wave's phase at `p` (world x, z) at time `t`: deep-water waves, the
// longer the faster.
fn wave_phase(wave: vec4<f32>, p: vec2<f32>, t: f32) -> f32 {
    let k = TAU / wave.z;
    return k * dot(normalize(wave.xy), p) - sqrt(GRAVITY * k) * t;
}

fn wave_height(p: vec2<f32>, t: f32) -> f32 {
    var height = 0.0;
    for (var i = 0; i < 3; i++) {
        let wave = params.waves[i];
        height += wave.w * sin(wave_phase(wave, p, t));
    }
    return height;
}

// How steep the water is at `p`: its height's change along x and z.
fn wave_slope(p: vec2<f32>, t: f32) -> vec2<f32> {
    var slope = vec2<f32>(0.0);
    for (var i = 0; i < 3; i++) {
        let wave = params.waves[i];
        let k = TAU / wave.z;
        slope += wave.w * k * normalize(wave.xy) * cos(wave_phase(wave, p, t));
    }
    return slope;
}

fn vertex(v: VertexIn) -> VertexIn {
    var moved = v;
    let world = v.position + engine.camera_position;
    moved.position.y += wave_height(world.xz, engine.time);
    return moved;
}

fn fragment(f: FragmentIn) -> vec4<f32> {
    let slope = wave_slope(f.world_position.xz, engine.time);
    let normal = normalize(vec3<f32>(-slope.x, 1.0, -slope.y));

    // How much water is between this point of the surface and whatever is
    // behind it: this pixel's depth in the scene, minus the surface's own
    // (the template's linear_depth turns each into a distance).
    let clip = camera.view_proj * vec4<f32>(f.position, 1.0);
    let ndc = clip.xyz / clip.w;
    let size = vec2<f32>(textureDimensions(scene_depth));
    let pixel = vec2<i32>(clamp((vec2<f32>(ndc.x, -ndc.y) * 0.5 + 0.5) * size, vec2<f32>(0.0), size - 1.0));
    let behind = linear_depth(textureLoad(scene_depth, pixel, 0).x);
    let thickness = max(behind - linear_depth(ndc.z), 0.0);
    let deepness = 1.0 - exp(-thickness * 0.35);

    let to_camera = normalize(-f.position);
    let diffuse = max(dot(normal, engine.sun_direction), 0.0);
    var color = mix(params.shallow.rgb, params.deep.rgb, deepness) * (engine.ambient_color + (0.55 + 0.45 * diffuse) * engine.sun_color);
    // More sky reflected the flatter it's looked at.
    let fresnel = pow(1.0 - max(dot(normal, to_camera), 0.0), 5.0);
    color = mix(color, SKY, fresnel * 0.6);
    let glint = pow(max(dot(normal, normalize(engine.sun_direction + to_camera)), 0.0), 120.0);
    color += engine.sun_color * glint;
    // Foam where the water is thin - around whatever pokes through it.
    let foam = 1.0 - smoothstep(0.0, 0.35, thickness);
    color = mix(color, FOAM, foam * 0.8);

    let alpha = clamp(mix(params.shallow.a, params.deep.a, deepness) + foam * 0.5 + fresnel * 0.3, 0.0, 1.0);
    return vec4<f32>(color, alpha);
}
