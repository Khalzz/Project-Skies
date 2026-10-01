// Thick lines - one instance per segment, widened to a fixed width in pixels
// on screen (see engine::rendering::thick_lines). Each instance draws 6
// vertices: a quad from `start` to `end`, `width` pixels across.

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
};

@group(0) @binding(0)
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
    // Half the width either side, and half past each end so joints meet.
    let offset_pixels = across * side * line.width * 0.5 + direction * along * line.width * 0.5;
    output.position = vec4<f32>(base.xy + offset_pixels / half_screen * base.w, base.z, base.w);
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return input.color;
}
