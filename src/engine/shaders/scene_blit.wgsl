// Vertex shader - fullscreen quad, same technique as depth_map.wgsl.

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) tex_coords: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
}

@vertex
fn vs_main(model: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.tex_coords = model.tex_coords;
    out.clip_position = vec4<f32>(model.position, 1.0);
    return out;
}

// Fragment shader - copies scene_color onto the swapchain (see
// BlurRender::render), applying the frame's ScreenEffects on the way (all
// zero = a plain copy). The background_blur effect isn't computed here - it
// happens per-fragment in text_shader.wgsl, sampling scene_color directly at
// whatever radius each UI node asks for.

@group(0) @binding(0)
var t_scene: texture_2d<f32>;
@group(0) @binding(1)
var s_scene: sampler;

// See BlurRender's ScreenEffects.
struct ScreenEffects {
    // desaturation (0..1), tunnel (0..1), screen aspect (w/h), darkening (0..1)
    params: vec4<f32>,
    // rgb the tunnel closes in with
    tunnel_color: vec4<f32>,
    // flat color over the scene, alpha = how much
    tint: vec4<f32>,
};

@group(0) @binding(2)
var<uniform> effects: ScreenEffects;

// How wide the tunnel's soft edge is, as a fraction of the distance from the
// screen's center to a corner - the dark fades in over that whole distance,
// so it reads as vision dimming from the edges, not a hard circle closing.
const TUNNEL_BLUR: f32 = 1.1;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    var color = textureSample(t_scene, s_scene, in.tex_coords);
    let desaturation = effects.params.x;
    let tunnel = effects.params.y;
    let aspect = effects.params.z;
    let darkening = effects.params.w;

    // Black and white: toward the pixel's own brightness.
    let luma = dot(color.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    color = vec4<f32>(mix(color.rgb, vec3<f32>(luma), desaturation), color.a);

    // The whole view dimming evenly.
    color = vec4<f32>(color.rgb * (1.0 - darkening), color.a);

    // Tunnel vision: round (aspect-corrected), with a very wide soft edge
    // (TUNNEL_BLUR). Its inner edge moves from the screen's corners (tunnel
    // 0 - nothing covered) inward, until even the center is past the soft
    // edge (tunnel 1 - all covered).
    let from_center = (in.tex_coords - vec2<f32>(0.5)) * vec2<f32>(aspect, 1.0) * 2.0;
    let corner = length(vec2<f32>(aspect, 1.0));
    let blur = corner * TUNNEL_BLUR;
    let inner = mix(corner, -blur, tunnel);
    let covered = smoothstep(inner, inner + blur, length(from_center));
    color = vec4<f32>(mix(color.rgb, effects.tunnel_color.rgb, covered), color.a);

    color = vec4<f32>(mix(color.rgb, effects.tint.rgb, effects.tint.a), color.a);
    return color;
}
