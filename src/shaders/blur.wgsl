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

// Dual Kawase blur (Marius Bjørge, "Bandwidth-Efficient Rendering", 2015) -
// see BlurRender. `fs_down` halves the image, `fs_up` doubles it back; each
// step is a few bilinear taps placed between texels, so every tap averages
// four of them for free. The blur comes from the chain of resolutions, not
// from wide kernels - its cost barely grows with the radius.

@group(0) @binding(0)
var t_source: texture_2d<f32>;
@group(0) @binding(1)
var s_source: sampler;
// xy: half a texel of t_source, in UV. zw: unused padding, kept for uniform
// buffer alignment.
@group(0) @binding(2)
var<uniform> u_half_texel: vec4<f32>;

fn tap(uv: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(t_source, s_source, uv, 0.0);
}

// The center, and the four corners half a texel out - each corner tap is
// the average of a 2x2 block.
@fragment
fn fs_down(in: VertexOutput) -> @location(0) vec4<f32> {
    let h = u_half_texel.xy;
    var color = tap(in.tex_coords) * 4.0;
    color += tap(in.tex_coords + vec2<f32>(-h.x, -h.y));
    color += tap(in.tex_coords + vec2<f32>(h.x, -h.y));
    color += tap(in.tex_coords + vec2<f32>(-h.x, h.y));
    color += tap(in.tex_coords + vec2<f32>(h.x, h.y));
    return color / 8.0;
}

// A tent over the lower-resolution image: four taps a texel out along the
// axes, four half a texel out on the diagonals, weighted twice.
@fragment
fn fs_up(in: VertexOutput) -> @location(0) vec4<f32> {
    let h = u_half_texel.xy;
    var color = tap(in.tex_coords + vec2<f32>(-h.x * 2.0, 0.0));
    color += tap(in.tex_coords + vec2<f32>(h.x * 2.0, 0.0));
    color += tap(in.tex_coords + vec2<f32>(0.0, -h.y * 2.0));
    color += tap(in.tex_coords + vec2<f32>(0.0, h.y * 2.0));
    color += tap(in.tex_coords + vec2<f32>(-h.x, -h.y)) * 2.0;
    color += tap(in.tex_coords + vec2<f32>(h.x, -h.y)) * 2.0;
    color += tap(in.tex_coords + vec2<f32>(-h.x, h.y)) * 2.0;
    color += tap(in.tex_coords + vec2<f32>(h.x, h.y)) * 2.0;
    return color / 12.0;
}
