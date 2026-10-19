// Procedural "cloudless sea sky" - a clear-day atmospheric gradient with a
// sun disc and glow, no cubemap. Drawn exactly like skybox.wgsl (a cube
// centred on the camera; each vertex position doubles as the view direction),
// just with a computed colour instead of a texture sample. See
// SkyboxRender::new_procedural.

struct CameraUniform {
    view_proj: mat4x4<f32>,
};
@group(0) @binding(0)
var<uniform> camera: CameraUniform;

struct SkyUniform {
    // xyz = normalized sun direction in world space; w unused
    sun_direction: vec4<f32>,
    // deep blue overhead
    zenith_color: vec4<f32>,
    // pale, hazy blue-white down at the horizon
    horizon_color: vec4<f32>,
    // sun disc / glow tint
    sun_color: vec4<f32>,
    // pale blue-white haze band right on the horizon line
    haze_color: vec4<f32>,
    // the sea's own color at the horizon (see water.wgsl's SEA_HORIZON_COLOR)
    sea_horizon_color: vec4<f32>,
};
@group(1) @binding(0)
var<uniform> sky: SkyUniform;

struct VertexInput {
    @location(0) position: vec3<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) direction: vec3<f32>,
};

@vertex
fn vs_main(model: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    // camera.view_proj treats the camera as sitting at the origin (see
    // CameraUniform::update_view_proj), so the cube's local position already
    // is the view direction.
    out.direction = model.position;
    out.clip_position = camera.view_proj * vec4<f32>(model.position, 1.0);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let dir = normalize(in.direction);
    let sun_dir = normalize(sky.sun_direction.xyz);

    // Vertical gradient, pale blue at the horizon to deep blue overhead -
    // blended on square-rooted colors (roughly perceptual) rather than raw
    // linear ones, so the middle keeps the photos' clear cyan-blue instead of
    // going grey. pow(up, 0.5) spends most of the brightening in the lower
    // sky, where the atmosphere is thickest.
    let up = clamp(dir.y, 0.0, 1.0);
    let t = pow(up, 0.5);
    let gradient = mix(sqrt(sky.horizon_color.rgb), sqrt(sky.zenith_color.rgb), t);
    var sky_color = gradient * gradient;

    // Haze band right on the horizon line - ~5 degrees tall, pale blue-white,
    // brighter than the sea just below it: together they make the soft but
    // clear line the photos show. Lower the 12.0 for a taller band.
    let horizon_haze = exp(-abs(dir.y) * 12.0);
    sky_color = mix(sky_color, sky.haze_color.rgb, horizon_haze * 0.6);

    // Looking BELOW the horizon (past the water's edge at grazing angles) -
    // the sea's own horizon color, so any sliver there reads as sea.
    let below = clamp(-dir.y, 0.0, 1.0);
    sky_color = mix(sky_color, sky.sea_horizon_color.rgb, smoothstep(0.0, 0.02, below));

    // Sun: a tight bright disc plus a broad warm glow that lifts the whole
    // quadrant around it.
    let s = clamp(dot(dir, sun_dir), 0.0, 1.0);
    let disc = smoothstep(0.9997, 0.99992, s);
    let glow = pow(s, 260.0) * 0.7 + pow(s, 8.0) * 0.10;
    sky_color = sky_color + sky.sun_color.rgb * (disc + glow);

    return vec4<f32>(sky_color, 1.0);
}
