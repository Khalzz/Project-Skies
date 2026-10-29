// The template every Lines shader is built from (see engine::rendering::
// shader_library): thick lines - one instance per segment, widened to a fixed
// width in pixels on screen (see engine::rendering::thick_lines). Each
// instance draws 6 vertices: a quad from `start` to `end`, `width` pixels
// across. The `fragment` hook, pasted where the HOOKS line is, gets each
// pixel's color. Group 0 is the material's own, 1 the camera, 2 the lines'
// (below), 3 the light.

struct CameraUniform {
    view_proj: mat4x4<f32>,
    view_pos: vec4<f32>,
};

struct LineUniform {
    // The camera's world position - instances are in world space, the
    // camera's view_proj expects positions relative to the camera.
    camera_position: vec4<f32>,
    // Screen size in pixels (x, y).
    screen: vec4<f32>,
    // x: how many of `mask`'s edges are used - 0, no mask. y: 1 when the
    // lines are in screen pixels (from the top left), not the world.
    mask_info: vec4<f32>,
    // A convex outline on screen to draw inside: each edge (a, b, c) keeps a
    // pixel (x, y) while a*x + b*y + c >= 0.
    mask: array<vec4<f32>, 32>,
};

@group(2) @binding(0)
var<uniform> lines: LineUniform;

@group(1) @binding(0)
var<uniform> camera: CameraUniform;

struct LineInstance {
    @location(0) start: vec3<f32>,
    @location(1) end: vec3<f32>,
    @location(2) color: vec4<f32>,
    @location(3) width: f32,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    // How far across the line this is from its middle, and half its width
    // (pixels) - for the soft edge, so fractional widths read as such.
    @location(1) across_pixels: f32,
    @location(2) half_width: f32,
};

// Moves `a` along to `b` until it's in front of the camera (clip w > NEAR) -
// a segment running behind the camera is cut where it crosses, not flipped.
const NEAR: f32 = 0.01;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32, line: LineInstance) -> VertexOutput {
    var output: VertexOutput;
    output.color = line.color;

    var a = camera.view_proj * vec4<f32>(line.start - lines.camera_position.xyz, 1.0);
    var b = camera.view_proj * vec4<f32>(line.end - lines.camera_position.xyz, 1.0);
    if (lines.mask_info.y > 0.5) {
        // Screen pixels straight to clip space.
        a = vec4<f32>(line.start.x / lines.screen.x * 2.0 - 1.0, 1.0 - line.start.y / lines.screen.y * 2.0, 0.5, 1.0);
        b = vec4<f32>(line.end.x / lines.screen.x * 2.0 - 1.0, 1.0 - line.end.y / lines.screen.y * 2.0, 0.5, 1.0);
    }
    if (a.w < NEAR && b.w < NEAR) {
        // All behind the camera - nothing to draw.
        output.position = vec4<f32>(0.0, 0.0, 0.0, 0.0);
        return output;
    }
    if (a.w < NEAR) {
        a = mix(a, b, (NEAR - a.w) / (b.w - a.w));
    }
    if (b.w < NEAR) {
        b = mix(b, a, (NEAR - b.w) / (a.w - b.w));
    }

    // The segment's direction on screen, in pixels, and square to it.
    let half_screen = lines.screen.xy * 0.5;
    let screen_a = a.xy / a.w * half_screen;
    let screen_b = b.xy / b.w * half_screen;
    var direction = screen_b - screen_a;
    if (length(direction) < 1e-4) {
        direction = vec2<f32>(1.0, 0.0);
    }
    direction = normalize(direction);
    let across = vec2<f32>(-direction.y, direction.x);

    // Two triangles: corners (end, side) - end 0 = start, 1 = end.
    var ends = array<f32, 6>(0.0, 1.0, 0.0, 0.0, 1.0, 1.0);
    var sides = array<f32, 6>(-1.0, -1.0, 1.0, 1.0, -1.0, 1.0);
    let at_end = ends[vertex_index];
    let side = sides[vertex_index];

    var base = a;
    var along = -1.0;
    if (at_end > 0.5) {
        base = b;
        along = 1.0;
    }
    // Half the width either side (and a pixel more, for the soft edge), and
    // half past each end so joints meet.
    let half_width = line.width * 0.5;
    let reach = half_width + 1.0;
    let offset_pixels = across * side * reach + direction * along * half_width;
    output.position = vec4<f32>(base.xy + offset_pixels / half_screen * base.w, base.z, base.w);
    output.across_pixels = side * reach;
    output.half_width = half_width;
    return output;
}

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
// - color: what the engine draws there - the line's color, its soft edge in
//   its alpha. Returned unchanged, it changes nothing;
// - across: how far from the line's middle, in pixels (- one side, + the
//   other);
// - half_width: half the line's width, in pixels.
struct LineIn {
    pixel: vec2<f32>,
    color: vec4<f32>,
    across: f32,
    half_width: f32,
}

//@HOOKS

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    engine = engine_values();
    let edges = u32(lines.mask_info.x);
    for (var i = 0u; i < edges; i = i + 1u) {
        let edge = lines.mask[i];
        if (dot(edge.xy, input.position.xy) + edge.z < 0.0) {
            discard;
        }
    }
    // How much of this pixel the line covers, across it.
    let coverage = clamp(input.half_width + 0.5 - abs(input.across_pixels), 0.0, 1.0);
    return fragment(LineIn(input.position.xy, vec4<f32>(input.color.rgb, input.color.a * coverage), input.across_pixels, input.half_width));
}
