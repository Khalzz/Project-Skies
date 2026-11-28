// Aerial perspective: the air between the eye and what it sees. What's seen is dimmed by the air it crossed
// (more in blue, so far things turn blue), greyed as the air thickens, and replaced by the air's own glow. The air
// thins with height, so from high up the view clears, and looking down into the haze layer hazes more than
// looking along it. The glow is the sky's color at that angle: on the horizon its pale haze band (so thick air
// meets the sky without a seam), looking down a deeper, dimmer blue, and brighter toward the sun.

struct Params {
    ray_right: vec4<f32>,   // the camera's right, times tan(half the horizontal fov)
    ray_up: vec4<f32>,      // its up, times tan(half the vertical fov)
    ray_forward: vec4<f32>, // its forward; w: its height above the sea (m)
    depth: vec4<f32>,       // x, y: the projection's m22, m23 (distance = m23 / (raw + m22)); z: farthest sea (m)
    sun: vec4<f32>,         // toward the sun (unit)
    sun_color: vec4<f32>,   // the sky's sun glow tint
    sky_horizon: vec4<f32>, // the sky's colors (sky.wgsl): horizon, zenith, and its haze band (w: how much)
    sky_zenith: vec4<f32>,
    sky_haze: vec4<f32>,
    rayleigh: vec4<f32>,    // the air's extinction at sea level per meter (rgb); w: scale height (m)
    mie: vec4<f32>,         // x: the haze's extinction at sea level per meter; y: scale height (m)
    look: vec4<f32>,        // x: the glow's brightness looking straight down; y: how much thick air greys
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var depth_texture: texture_depth_2d;
@group(0) @binding(2) var scene: texture_2d<f32>;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    var out: VertexOutput;
    out.clip_position = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    out.uv = uv;
    return out;
}

// The air along a ray from height `height`, climbing `rise` per meter, for `distance` meters, against one
// exponentially thinning layer of scale height `scale`: how much air there is, as meters of sea-level air
fn column(height: f32, rise: f32, distance: f32, scale: f32) -> f32 {
    let k = max(rise * distance / scale, -80.0);
    var shape = 1.0 - 0.5 * k + k * k / 6.0;
    if (abs(k) > 1e-3) {
        shape = (1.0 - exp(-k)) / k;
    }
    return distance * exp(-max(height, 0.0) / scale) * shape;
}

// The air's glow along `direction`: the sky as sky.wgsl draws it at that angle above the horizon (mirrored for
// looking down, and dimmer the steeper), with the sun's glow
fn glow(direction: vec3<f32>) -> vec3<f32> {
    let elevation = abs(direction.y);
    let gradient = mix(sqrt(params.sky_horizon.rgb), sqrt(params.sky_zenith.rgb), sqrt(elevation));
    var color = mix(gradient * gradient, params.sky_haze.rgb, exp(-elevation * 12.0) * params.sky_haze.w);
    color *= mix(1.0, params.look.x, smoothstep(0.0, 0.5, -direction.y));

    let toward_sun = clamp(dot(direction, params.sun.xyz), 0.0, 1.0);
    let sun = pow(toward_sun, 260.0) * 0.7 + pow(toward_sun, 8.0) * 0.1;
    return color + params.sun_color.rgb * sun;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(in.clip_position.xy);
    let seen = textureLoad(scene, pixel, 0);

    let ndc = vec2<f32>(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0);
    let ray = params.ray_forward.xyz + ndc.x * params.ray_right.xyz + ndc.y * params.ray_up.xyz;
    let direction = normalize(ray);
    let height = params.ray_forward.w;

    // How far the ray goes: to what was drawn there or, through the sky, down to the sea
    let raw = textureLoad(depth_texture, pixel, 0);
    var distance = 0.0;
    if (raw > 0.0) {
        distance = length(ray) * params.depth.y / (raw + params.depth.x);
    } else if (direction.y < 0.0) {
        distance = min(max(height, 0.0) / -direction.y, params.depth.z);
    }
    if (distance <= 0.0) {
        return seen;
    }

    let rayleigh = params.rayleigh.xyz * column(height, direction.y, distance, params.rayleigh.w);
    let mie = vec3<f32>(params.mie.x * column(height, direction.y, distance, params.mie.y));
    var transmittance = exp(-(rayleigh + mie));

    // Through the sky below the horizon, the air closes in fully right at the horizon line, where the sky's own
    // haze band takes over
    if (raw <= 0.0) {
        transmittance *= smoothstep(0.0, 0.02, -direction.y);
    }

    let veil = 1.0 - dot(transmittance, vec3<f32>(1.0 / 3.0));
    let grey = vec3<f32>(dot(seen.rgb, vec3<f32>(0.2126, 0.7152, 0.0722)));
    let faded = mix(seen.rgb, grey, veil * params.look.y);
    return vec4<f32>(faded * transmittance + glow(direction) * (1.0 - transmittance), seen.a);
}
