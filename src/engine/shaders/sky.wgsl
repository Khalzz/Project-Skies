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

    // Vertical gradient. pow(up, 0.45) keeps most of the dome the zenith
    // colour and compresses the pale band down near the horizon, where real
    // atmospheric haze sits.
    let up = clamp(dir.y, 0.0, 1.0);
    var sky_color = mix(sky.horizon_color.rgb, sky.zenith_color.rgb, pow(up, 0.45));

    // Looking BELOW the true horizon (under the water plane's edge at grazing
    // angles) - settle onto a slightly muted sea-haze. Kept close to the
    // horizon tone (not dark) so any sliver visible past the water reads as
    // continuous haze rather than a dark gap.
    let below = clamp(-dir.y, 0.0, 1.0);
    sky_color = mix(sky_color, sky.horizon_color.rgb * 0.82, smoothstep(0.0, 0.10, below));

    // Soft bright haze band hugging the horizon line itself - this is the
    // band the fogged-out water meets, so keeping it wide and pale makes the
    // seam disappear. Matches water.wgsl's FOG_COLOR direction (pale, ~white).
    let horizon_haze = exp(-abs(dir.y) * 9.0);
    sky_color = mix(sky_color, vec3<f32>(0.86, 0.90, 0.94), horizon_haze * 0.65);

    // Sun: a tight bright disc plus a broad warm glow that lifts the whole
    // quadrant around it.
    let s = clamp(dot(dir, sun_dir), 0.0, 1.0);
    let disc = smoothstep(0.9997, 0.99992, s);
    let glow = pow(s, 260.0) * 0.7 + pow(s, 8.0) * 0.10;
    sky_color = sky_color + sky.sun_color.rgb * (disc + glow);

    return vec4<f32>(sky_color, 1.0);
}
