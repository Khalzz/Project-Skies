// 2D nodes' shapes (see rendering::shapes_2d) - one instance per shape, a
// quad turned on screen, its edge worked out per pixel from the shape's
// distance field and smoothed over one pixel.

struct Screen {
    // xy: the target's size in pixels.
    size: vec4<f32>,
};

@group(0) @binding(0) var<uniform> screen: Screen;

struct Instance {
    // Pixels, from the top left.
    @location(0) center: vec2<f32>,
    @location(1) half_size: vec2<f32>,
    // cos and sin of its rotation (clockwise, y down).
    @location(2) rotation: vec2<f32>,
    // x: 0 a rectangle, 1 a circle (radius half_size.x).
    @location(3) kind: vec2<f32>,
    @location(4) color: vec4<f32>,
};

struct VertexOut {
    @builtin(position) position: vec4<f32>,
    // Where in the shape, in pixels from its middle, before turning.
    @location(0) local: vec2<f32>,
    @location(1) @interpolate(flat) half_size: vec2<f32>,
    @location(2) @interpolate(flat) kind: f32,
    @location(3) color: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32, instance: Instance) -> VertexOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    // A pixel bigger all round, for the smoothed edge.
    let local = corners[index] * (instance.half_size + vec2<f32>(1.0, 1.0));
    let c = instance.rotation.x;
    let s = instance.rotation.y;
    let pixel = instance.center + vec2<f32>(local.x * c - local.y * s, local.x * s + local.y * c);

    var out: VertexOut;
    out.position = vec4<f32>(pixel.x / screen.size.x * 2.0 - 1.0, 1.0 - pixel.y / screen.size.y * 2.0, 0.0, 1.0);
    out.local = local;
    out.half_size = instance.half_size;
    out.kind = instance.kind.x;
    out.color = instance.color;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    // How far outside the shape's edge this pixel is (negative: inside).
    var distance: f32;
    if in.kind < 0.5 {
        let d = abs(in.local) - in.half_size;
        distance = length(max(d, vec2<f32>(0.0, 0.0))) + min(max(d.x, d.y), 0.0);
    } else {
        distance = length(in.local) - in.half_size.x;
    }
    let coverage = clamp(0.5 - distance, 0.0, 1.0);
    if coverage <= 0.0 {
        discard;
    }
    return vec4<f32>(in.color.rgb, in.color.a * coverage);
}
