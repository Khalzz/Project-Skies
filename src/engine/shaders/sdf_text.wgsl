// SDF text - one instance per glyph, a quad on its string's plane, filled
// from the glyph's distance field (see engine::rendering::sdf_text). Each
// instance draws 6 vertices.

struct CameraUniform {
    view_proj: mat4x4<f32>,
    view_pos: vec4<f32>,
};

struct TextUniform {
    // The camera's world position - instances are in world space, the
    // camera's view_proj expects positions relative to the camera.
    camera_position: vec4<f32>,
    // Screen size in pixels (x, y).
    screen: vec4<f32>,
    // x: how many of `mask`'s edges are used - 0, no mask. y: 1 when the
    // glyphs are in screen pixels (from the top left), not the world.
    mask_info: vec4<f32>,
    // A convex outline on screen to draw inside: each edge (a, b, c) keeps a
    // pixel (x, y) while a*x + b*y + c >= 0.
    mask: array<vec4<f32>, 32>,
};

@group(0) @binding(0)
var<uniform> text: TextUniform;
@group(0) @binding(1)
var t_atlas: texture_2d<f32>;
@group(0) @binding(2)
var s_atlas: sampler;

@group(1) @binding(0)
var<uniform> camera: CameraUniform;

struct GlyphInstance {
    // The string's top left, and one em along and down its lines.
    @location(0) origin: vec3<f32>,
    @location(1) axis_x: vec3<f32>,
    @location(2) axis_y: vec3<f32>,
    // The glyph's quad from `origin`, in ems: x0, y0, x1, y1.
    @location(3) rect: vec4<f32>,
    // Its corners in the atlas: u0, v0, u1, v1.
    @location(4) uv: vec4<f32>,
    @location(5) color: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
};

// A glyph whose string starts behind the camera isn't drawn.
const NEAR: f32 = 0.01;

fn to_clip(point: vec3<f32>) -> vec4<f32> {
    if (text.mask_info.y > 0.5) {
        return vec4<f32>(point.x / text.screen.x * 2.0 - 1.0, 1.0 - point.y / text.screen.y * 2.0, 0.5, 1.0);
    }
    return camera.view_proj * vec4<f32>(point - text.camera_position.xyz, 1.0);
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32, glyph: GlyphInstance) -> VertexOutput {
    var output: VertexOutput;
    output.color = glyph.color;

    var corners_x = array<f32, 6>(0.0, 1.0, 0.0, 0.0, 1.0, 1.0);
    var corners_y = array<f32, 6>(0.0, 0.0, 1.0, 1.0, 0.0, 1.0);
    let corner = vec2<f32>(corners_x[vertex_index], corners_y[vertex_index]);

    if (to_clip(glyph.origin).w < NEAR) {
        output.position = vec4<f32>(0.0, 0.0, 0.0, 0.0);
        return output;
    }
    let local = mix(glyph.rect.xy, glyph.rect.zw, corner);
    output.position = to_clip(glyph.origin + glyph.axis_x * local.x + glyph.axis_y * local.y);
    output.uv = mix(glyph.uv.xy, glyph.uv.zw, corner);
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    // The field is 0.5 on the edge, more inside. Its change over one screen
    // pixel is how soft the edge is drawn - so it stays one pixel wide at
    // any size or slant. Sampled before the mask, so the derivatives are
    // taken with every pixel of the quad still running.
    let distance = textureSample(t_atlas, s_atlas, input.uv).r;
    let pixel = max(fwidth(distance), 1e-5);
    let coverage = clamp((distance - 0.5) / pixel + 0.5, 0.0, 1.0);

    let edges = u32(text.mask_info.x);
    for (var i = 0u; i < edges; i = i + 1u) {
        let edge = text.mask[i];
        if (dot(edge.xy, input.position.xy) + edge.z < 0.0) {
            discard;
        }
    }
    if (coverage <= 0.0) {
        discard;
    }
    return vec4<f32>(input.color.rgb, input.color.a * coverage);
}
