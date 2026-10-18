// What the pilot's eyes do to the picture (see game::screen_effects): copies
// the world ("scene") onto the screen with the flight HUD ("hud") over it,
// then greys, dims, tunnels and tints both. All zero = a plain copy. The
// play scene's frame plan's Fullscreen step; settings in g_vision.ron.

// See game::screen_effects::ScreenEffects::params.
struct Params {
    // desaturation (0..1), tunnel (0..1), unused, darkening (0..1)
    params: vec4<f32>,
    // rgb the tunnel closes in with
    tunnel_color: vec4<f32>,
    // flat color over the scene, alpha = how much
    tint: vec4<f32>,
    // x: flight HUD opacity (0 off .. 1 as drawn). The fade-in after a
    // cinematic cuts back to the cockpit.
    hud_params: vec4<f32>,
};

// How wide the tunnel's soft edge is, as a fraction of the distance from the
// screen's center to a corner - the dark fades in over that whole distance,
// so it reads as vision dimming from the edges, not a hard circle closing.
const TUNNEL_BLUR: f32 = 1.1;

fn fragment(f: ScreenIn) -> vec4<f32> {
    var color = textureSample(scene, scene_sampler, f.uv);
    // The HUD is stored premultiplied (drawn straight-alpha blended onto
    // transparency), so both its cover and its color scale with the
    // power-on opacity.
    let hud_color = textureSample(hud, hud_sampler, f.uv);
    let hud_opacity = params.hud_params.x;
    let cover = hud_color.a * hud_opacity;
    color = vec4<f32>(color.rgb * (1.0 - cover) + hud_color.rgb * hud_opacity, 1.0);
    let desaturation = params.params.x;
    let tunnel = params.params.y;
    let darkening = params.params.w;
    let size = vec2<f32>(textureDimensions(scene));
    let aspect = size.x / max(size.y, 1.0);

    // Black and white: toward the pixel's own brightness.
    let luma = dot(color.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    color = vec4<f32>(mix(color.rgb, vec3<f32>(luma), desaturation), color.a);

    // The whole view dimming evenly.
    color = vec4<f32>(color.rgb * (1.0 - darkening), color.a);

    // Tunnel vision: round (aspect-corrected), with a very wide soft edge
    // (TUNNEL_BLUR). Its inner edge moves from the screen's corners (tunnel
    // 0 - nothing covered) inward, until even the center is past the soft
    // edge (tunnel 1 - all covered).
    let from_center = (f.uv - vec2<f32>(0.5)) * vec2<f32>(aspect, 1.0) * 2.0;
    let corner = length(vec2<f32>(aspect, 1.0));
    let blur = corner * TUNNEL_BLUR;
    let inner = mix(corner, -blur, tunnel);
    let covered = smoothstep(inner, inner + blur, length(from_center));
    color = vec4<f32>(mix(color.rgb, params.tunnel_color.rgb, covered), color.a);

    color = vec4<f32>(mix(color.rgb, params.tint.rgb, params.tint.a), color.a);
    return color;
}
