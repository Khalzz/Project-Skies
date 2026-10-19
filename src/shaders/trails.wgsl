// Draws trails - ribbons ParticleRenderer builds on the CPU each frame
// (camera-relative, already facing the camera), see
// engine::particles::renderer. Frame must match particles_update.wgsl.

struct Frame {
    camera_right: vec4<f32>,
    camera_up: vec4<f32>,
    camera_forward: vec4<f32>,
    camera_delta: vec4<f32>,
    wind: vec4<f32>,
    sun_direction: vec4<f32>,
    sun_color: vec4<f32>,
};

struct CameraUniform {
    view_proj: mat4x4<f32>,
    view_pos: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> frame: Frame;

@group(1) @binding(0)
var<uniform> camera: CameraUniform;

@group(2) @binding(0)
var t_scene_depth: texture_2d<f32>;

@group(3) @binding(0)
var t_sprite: texture_2d<f32>;
@group(3) @binding(1)
var s_sprite: sampler;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    // linear rgb (brightness applied) + alpha
    @location(2) color: vec4<f32>,
    // lit (0/1), soft fade distance (m), shape (0 camera-facing wisp,
    // 1 upright curtain, 2 low-poly ridge), distance along the trail in points
    // (sticks to them)
    @location(3) params: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) params: vec4<f32>,
    // camera-relative
    @location(3) world_position: vec3<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = camera.view_proj * vec4<f32>(in.position, 1.0);
    out.uv = in.uv;
    out.color = in.color;
    out.params = in.params;
    out.world_position = in.position;
    return out;
}

fn linearize_depth(raw: f32, near: f32, far: f32) -> f32 {
    let depth = 1.0 - raw;
    let z_ndc = depth * 2.0 - 1.0;
    return (2.0 * near * far) / (far + near - z_ndc * (far - near));
}

fn hash(x: f32) -> f32 {
    return fract(sin(x * 127.1) * 43758.5453);
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Each triangle's own flat normal, from how its surface changes across
    // the screen - before any branching (derivatives need uniform control
    // flow). Used by the low-poly ridge.
    let face_normal = normalize(cross(dpdx(in.world_position), dpdy(in.world_position)));

    let texel = textureSampleLevel(t_sprite, s_sprite, in.uv, 0.0);
    var rgb = in.color.rgb * texel.rgb;
    var alpha = in.color.a * texel.a;
    let shape = in.params.z;
    if (shape > 1.5) {
        // Low-poly ridge: solid, fading into the water at its base and a
        // little toward its peak - the facets do the rest.
        let v = in.uv.y;
        alpha *= smoothstep(0.0, 0.08, v) * (1.0 - 0.35 * v);
    } else if (shape > 0.5) {
        // Upright curtain (spray): solid from the water up, with a jagged,
        // low-poly top edge - straight lines between a random height per
        // step along it (params.w counts along the trail, stuck to its
        // points, so the pattern moves with them) - and uneven streaks.
        let v = in.uv.y;
        let along = in.params.w * 3.0;
        let step = floor(along);
        let top = mix(0.45 + 0.55 * hash(step), 0.45 + 0.55 * hash(step + 1.0), fract(along));
        let body = 1.0 - smoothstep(top - 0.04, top, v);
        let streaks = 0.6 + 0.4 * hash(step + 37.0);
        let into_water = smoothstep(0.0, 0.05, v);
        alpha *= body * streaks * into_water * (1.0 - 0.5 * v);
    } else {
        // Soft toward its edges, so it reads as a wisp, not a flat strip.
        let across = abs(in.uv.y * 2.0 - 1.0);
        alpha *= 1.0 - across * across;
    }

    if (in.params.x > 0.5) {
        if (shape > 1.5) {
            // Flat-shaded per face - turned toward the camera (both sides of
            // a face are seen), lit by how squarely it faces the sun.
            let to_camera = normalize(-in.world_position);
            let normal = select(-face_normal, face_normal, dot(face_normal, to_camera) > 0.0);
            let diffuse = max(dot(normal, frame.sun_direction.xyz), 0.0);
            rgb *= frame.sun_direction.w + frame.sun_color.rgb * diffuse * 1.1;
        } else {
            // A thin wisp - lit mostly by the sky, brighter the higher the sun.
            rgb *= frame.sun_direction.w + frame.sun_color.rgb * (0.4 + 0.6 * max(frame.sun_direction.y, 0.0));
        }
    }

    let soft_distance = in.params.y;
    if (soft_distance > 0.0) {
        let near = frame.camera_right.w;
        let far = frame.camera_up.w;
        let scene_raw = textureLoad(t_scene_depth, vec2<i32>(in.clip_position.xy), 0).x;
        let gap = linearize_depth(scene_raw, near, far) - linearize_depth(in.clip_position.z, near, far);
        alpha *= clamp(gap / soft_distance, 0.0, 1.0);
    }
    return vec4<f32>(rgb * alpha, alpha);
}
