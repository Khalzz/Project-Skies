// Draws the particles particles_update.wgsl simulates - one camera-facing
// (or velocity-stretched, or flat) quad per particle, straight out of the
// particle buffer: instance = particle slot, 6 vertices = its quad. See
// engine::particles::renderer.
//
// Particle / EmitterParams / Frame must match particles_update.wgsl.

struct Particle {
    position: vec3<f32>,
    age: f32,
    velocity: vec3<f32>,
    lifetime: f32,
    rotation: f32,
    spin: f32,
    seed: f32,
    intensity: f32,
    emitter: u32,
    // The emitter's velocity when it spawned - streaks are drawn along its
    // motion relative to this (see Facing::Velocity).
    carrier_x: f32,
    carrier_y: f32,
    carrier_z: f32,
};

struct EmitterParams {
    forces: vec4<f32>,
    look: vec4<f32>,
    shape: vec4<f32>,
    flags: vec4<f32>,
    scales: vec4<f32>,
    size: array<vec4<f32>, 2>,
    color: array<vec4<f32>, 8>,
};

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
var<storage, read> particles: array<Particle>;
@group(0) @binding(1)
var<storage, read> emitters: array<EmitterParams>;
@group(0) @binding(2)
var<uniform> frame: Frame;

@group(1) @binding(0)
var<uniform> camera: CameraUniform;

// The scene's depth (opaque + water), for soft fading - see
// DepthRender::foam_depth_copy.
@group(2) @binding(0)
var t_scene_depth: texture_2d<f32>;

@group(3) @binding(0)
var t_sprite: texture_2d<f32>;
@group(3) @binding(1)
var s_sprite: sampler;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    // -1..1 across the quad
    @location(0) corner: vec2<f32>,
    // linear rgb (brightness applied) + alpha
    @location(1) color: vec4<f32>,
    // camera-relative
    @location(2) world_position: vec3<f32>,
    @location(3) @interpolate(flat) emitter: u32,
    @location(4) @interpolate(flat) flipbook_frame: f32,
    // the quad's own axes, for a rounded lighting normal
    @location(5) axis_right: vec3<f32>,
    @location(6) axis_up: vec3<f32>,
};

// Over-life curves are 8 samples - read straight from the buffer (a
// runtime index into a copied array isn't allowed).
fn sample_size(emitter: u32, t: f32) -> f32 {
    let x = clamp(t, 0.0, 1.0) * 7.0;
    let i = min(u32(x), 6u);
    let a = emitters[emitter].size[i / 4u][i % 4u];
    let b = emitters[emitter].size[(i + 1u) / 4u][(i + 1u) % 4u];
    return mix(a, b, x - f32(i));
}

fn sample_color(emitter: u32, t: f32) -> vec4<f32> {
    let x = clamp(t, 0.0, 1.0) * 7.0;
    let i = min(u32(x), 6u);
    return mix(emitters[emitter].color[i], emitters[emitter].color[i + 1u], x - f32(i));
}

@vertex
fn vs_main(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> VertexOutput {
    var out: VertexOutput;
    let p = particles[instance];
    if (p.lifetime <= 0.0 || p.age < 0.0 || p.age >= p.lifetime) {
        // Not alive - collapse to a point (draws nothing).
        out.clip_position = vec4<f32>(0.0, 0.0, 0.0, 1.0);
        return out;
    }
    let e = emitters[p.emitter];
    let t = p.age / p.lifetime;

    var size = sample_size(p.emitter, t);
    if (e.scales.y > 0.5) {
        size *= p.intensity;
    }
    var color = sample_color(p.emitter, t);
    if (e.scales.x > 0.5) {
        color.a *= p.intensity;
    }

    // Two triangles: (-1,-1) (1,-1) (1,1) and (-1,-1) (1,1) (-1,1).
    // Vertices 0..5 -> corners 0, 1, 2, 0, 2, 3.
    let corner_index = select(select(vertex - 2u, 0u, vertex == 3u), vertex, vertex < 3u);
    let corner = vec2<f32>(
        select(-1.0, 1.0, corner_index == 1u || corner_index == 2u),
        select(-1.0, 1.0, corner_index >= 2u),
    );
    let half_size = size * 0.5;

    var right: vec3<f32>;
    var up: vec3<f32>;
    var half_length = half_size;
    let facing = e.look.y;
    if (facing < 0.5) {
        // Billboard, spun by its own rotation.
        let c = cos(p.rotation);
        let s = sin(p.rotation);
        right = frame.camera_right.xyz * c + frame.camera_up.xyz * s;
        up = frame.camera_up.xyz * c - frame.camera_right.xyz * s;
    } else if (facing < 1.5) {
        // Streaked along its motion relative to what threw it (the
        // emitter's velocity when it spawned), as seen from the camera -
        // spray off a fast plane streaks the way it moves seen from the
        // plane, not along its path over the water.
        let motion = p.velocity - vec3<f32>(p.carrier_x, p.carrier_y, p.carrier_z);
        let to_camera = normalize(-p.position);
        let along = motion - to_camera * dot(motion, to_camera);
        let along_length = length(along);
        up = select(frame.camera_up.xyz, along / max(along_length, 1e-5), along_length > 1e-4);
        right = normalize(cross(up, to_camera));
        half_length = half_size + length(motion) * e.look.z * 0.5;
    } else {
        // Flat on the horizontal.
        let c = cos(p.rotation);
        let s = sin(p.rotation);
        right = vec3<f32>(c, 0.0, s);
        up = vec3<f32>(-s, 0.0, c);
    }

    let world = p.position + right * (corner.x * half_size) + up * (corner.y * half_length);
    out.clip_position = camera.view_proj * vec4<f32>(world, 1.0);
    out.corner = corner;
    out.color = vec4<f32>(color.rgb * e.look.x, color.a);
    out.world_position = world;
    out.emitter = p.emitter;
    let frames = max(e.shape.y * e.shape.z, 1.0);
    out.flipbook_frame = floor(p.age * e.shape.w) % frames;
    out.axis_right = right;
    out.axis_up = up;
    return out;
}

// Same conversion as water.wgsl's: a raw reversed-Z depth value to a
// distance along the view axis.
fn linearize_depth(raw: f32, near: f32, far: f32) -> f32 {
    let depth = 1.0 - raw;
    let z_ndc = depth * 2.0 - 1.0;
    return (2.0 * near * far) / (far + near - z_ndc * (far - near));
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Before any branching - derivatives need uniform control flow.
    let radius = length(in.corner);
    let edge_width = fwidth(radius);

    let e = emitters[in.emitter];
    let r2 = radius * radius;
    var rgb = in.color.rgb;
    var alpha = in.color.a;
    let to_camera = normalize(-in.world_position);

    // The sprite - always sampled (at mip 0, so it's fine outside uniform
    // control flow), then used or not depending on the shape.
    let uv = in.corner * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    let columns = max(e.shape.y, 1.0);
    let rows = max(e.shape.z, 1.0);
    let cell = vec2<f32>(in.flipbook_frame % columns, floor(in.flipbook_frame / columns));
    let flipbook_uv = (cell + uv) / vec2<f32>(columns, rows);
    let shape = e.shape.x;
    let texel = textureSampleLevel(t_sprite, s_sprite, select(uv, flipbook_uv, shape > 1.5 && shape < 2.5), 0.0);

    // The lighting normal - a rounded ball by default.
    let bulge = sqrt(clamp(1.0 - r2, 0.0, 1.0));
    var normal = normalize(in.axis_right * in.corner.x + in.axis_up * in.corner.y + to_camera * bulge);

    if (shape < 0.5) {
        let edge = clamp(1.0 - r2, 0.0, 1.0);
        alpha *= edge * sqrt(edge);
    } else if (shape > 2.5) {
        // Low-poly: a hard-edged polygon, split into an outer and an inner
        // ring of flat facets (the inner one turned half a facet, so they
        // read as triangles), each with one flat normal.
        let sides = max(e.shape.y, 3.0);
        let sector = 6.2831853 / sides;
        let angle = atan2(in.corner.y, in.corner.x) + 3.14159265;
        let facet = floor(angle / sector);
        let from_facet_center = angle - (facet + 0.5) * sector;
        let edge_radius = cos(sector * 0.5) / cos(from_facet_center);
        alpha *= 1.0 - smoothstep(edge_radius - edge_width, edge_radius, radius);

        let inner = radius < edge_radius * 0.5;
        let facet_angle = select((facet + 0.5) * sector, floor(angle / sector + 0.5) * sector, inner) - 3.14159265;
        let tilt = select(0.85, 0.4, inner);
        let outward = in.axis_right * cos(facet_angle) + in.axis_up * sin(facet_angle);
        normal = normalize(outward * tilt + to_camera * sqrt(1.0 - tilt * tilt));
    } else {
        rgb *= texel.rgb;
        alpha *= texel.a;
    }

    // Sunlit: shaded like a little sphere (or its facets), or flat when
    // lying horizontal.
    if (e.flags.x > 0.5) {
        if (e.look.y > 1.5) {
            normal = vec3<f32>(0.0, 1.0, 0.0);
        }
        let diffuse = max(dot(normal, frame.sun_direction.xyz), 0.0);
        rgb *= frame.sun_direction.w + frame.sun_color.rgb * diffuse;
    }

    // Soft: fades out as it nears whatever's behind it.
    let soft_distance = e.look.w;
    if (soft_distance > 0.0) {
        let near = frame.camera_right.w;
        let far = frame.camera_up.w;
        let scene_raw = textureLoad(t_scene_depth, vec2<i32>(in.clip_position.xy), 0).x;
        let gap = linearize_depth(scene_raw, near, far) - linearize_depth(in.clip_position.z, near, far);
        alpha *= clamp(gap / soft_distance, 0.0, 1.0);
    }

    // Premultiplied - the same output works for additive and alpha blending.
    return vec4<f32>(rgb * alpha, alpha);
}
