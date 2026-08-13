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

// Fragment shader - one 9-tap separable Gaussian pass. BlurRender chains six of
// these (three H+V pairs, each with a wider step than the last - see
// BlurRender::render) into one smooth, dense, ghost-free "fully blurred" texture:
// unlike a single pass of sparse point-samples spread over a large radius (which
// shows each sample as a visible echo of whatever's under it - exactly the
// "element repeated" artifact this replaced), repeatedly re-blurring an
// already-smoothed image keeps every pass densely, continuously sampled, so the
// compounded result reads as genuine soft blur instead of ghosting.

@group(0) @binding(0)
var t_source: texture_2d<f32>;
@group(0) @binding(1)
var s_source: sampler;
// xy: per-tap sample step, in UV space (this pass's step size / source
// resolution, along whichever axis this pass blurs). zw: unused padding, kept
// for uniform buffer alignment.
@group(0) @binding(2)
var<uniform> u_direction: vec4<f32>;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let dir = u_direction.xy;
    var color = textureSample(t_source, s_source, in.tex_coords) * 0.227027;
    color += textureSample(t_source, s_source, in.tex_coords + dir * 1.0) * 0.1945946;
    color += textureSample(t_source, s_source, in.tex_coords - dir * 1.0) * 0.1945946;
    color += textureSample(t_source, s_source, in.tex_coords + dir * 2.0) * 0.1216216;
    color += textureSample(t_source, s_source, in.tex_coords - dir * 2.0) * 0.1216216;
    color += textureSample(t_source, s_source, in.tex_coords + dir * 3.0) * 0.054054;
    color += textureSample(t_source, s_source, in.tex_coords - dir * 3.0) * 0.054054;
    color += textureSample(t_source, s_source, in.tex_coords + dir * 4.0) * 0.016216;
    color += textureSample(t_source, s_source, in.tex_coords - dir * 4.0) * 0.016216;
    return color;
}
