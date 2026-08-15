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

// Fragment shader - plain passthrough, copying scene_color onto the swapchain
// (see BlurRender::render). The actual background_blur effect isn't computed
// here - it happens per-fragment in text_shader.wgsl, sampling scene_color
// directly at whatever radius each UI node asks for.

@group(0) @binding(0)
var t_scene: texture_2d<f32>;
@group(0) @binding(1)
var s_scene: sampler;

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(t_scene, s_scene, in.tex_coords);
}
