
// the elements brought here by the render buffer
struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) rect: vec4<f32>,
    @location(3) border_color: vec4<f32>,
    @location(4) corner_radius: f32,
    @location(5) border_width: f32,
    @location(6) background_blur: f32,
    // Bitmask (bit0=left,1=right,2=top,3=bottom) of which sides draw a border -
    // see BorderEdges::to_bits (Rust side). 15 (all 4 bits) means "use the
    // original rounded-corner SDF border below"; anything else switches to
    // straight per-edge bands instead - see the fragment shader's own comment.
    @location(7) border_edges: u32,
    // [top, left, bottom, right] (same packing as `rect` above), in the same
    // screen-pixel space as `rect`/`clip_position.xy` - see UiNode::
    // set_scrollable/node_content_preparation's clip_rect (Rust side). Any
    // fragment outside this gets discarded (see fs_main below) - a huge
    // sentinel rect (Self::NO_CLIP, Rust side) is the "no clip" default, not a
    // smaller real one, so this never triggers for the overwhelming majority
    // of nodes that were never inside a scrollable container.
    @location(8) clip_rect: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) rect: vec4<f32>,
    @location(2) border_color: vec4<f32>,
    @location(3) corner_radius: f32,
    @location(4) border_width: f32,
    @location(5) background_blur: f32,
    // flat: an integer vertex output can't be linearly interpolated (WGSL would
    // reject this without it) - harmless here since every vertex of a given quad
    // already carries the same value anyway, same as corner_radius/border_width.
    @location(6) @interpolate(flat) border_edges: u32,
    @location(7) clip_rect: vec4<f32>,
}

@vertex
fn vertex(model: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    out.color = model.color;
    out.clip_position = vec4<f32>(model.position, 1.0);
    out.rect = model.rect;
    out.border_color = model.border_color;
    out.corner_radius = model.corner_radius;
    out.border_width = model.border_width;
    out.background_blur = model.background_blur;
    out.border_edges = model.border_edges;
    out.clip_rect = model.clip_rect;

    return out;
}

// The sharp offscreen scene copy, and BlurRender's precomputed, densely-sampled
// "fully blurred" version of it (see BlurRender's own doc comment for why this
// is a shared precomputed texture rather than a per-fragment sample scatter -
// the latter showed visible ghosting/repeats at large radii instead of smooth
// blur). background_blur below crossfades between the two.
@group(0) @binding(0)
var t_scene: texture_2d<f32>;
@group(0) @binding(1)
var s_scene: sampler;
@group(0) @binding(2)
var t_blurred: texture_2d<f32>;
@group(0) @binding(3)
var s_blurred: sampler;
// xy: screen size in pixels. zw: unused padding, kept for uniform buffer alignment.
@group(0) @binding(4)
var<uniform> u_screen_size: vec4<f32>;

// Matches BlurRender::MAX_BLUR_RADIUS - the background_blur value (in pixels)
// that samples t_blurred outright; values below crossfade toward t_scene.
const MAX_BLUR_RADIUS: f32 = 64.0;

// Signed distance from `p` (relative to the box's own center) to a box of
// `half_size`, corners rounded by `radius` - negative inside, 0 on the edge,
// positive outside. Standard rounded-box SDF (Inigo Quilez).
fn sd_rounded_box(p: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let q = abs(p) - half_size + vec2<f32>(radius, radius);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0, 0.0))) - radius;
}

// Cheap procedural noise (no texture asset, just a hash of the pixel's own
// screen position) - static per-frame is fine here, this isn't meant to
// sparkle/animate, just to break up the perfectly smooth gradients a plain blur
// leaves behind. Standard "hash without sine" style function.
fn hash12(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 += dot(p3, p3.zyx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

// Pushes `color` away from (amount > 1.0) or toward (amount < 1.0) grayscale,
// pivoting on its own perceptual luminance - amount == 1.0 is a no-op.
fn adjust_saturation(color: vec3<f32>, amount: f32) -> vec3<f32> {
    let luminance = dot(color, vec3<f32>(0.2126, 0.7152, 0.0722));
    return mix(vec3<f32>(luminance, luminance, luminance), color, amount);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // Scroll clipping (see UiNode::set_scrollable/node_content_preparation's
    // clip_rect, Rust side) - discard anything outside the tightest scrollable
    // ancestor's own content rect, same [top, left, bottom, right] packing as
    // `rect` below. A no-op for the overwhelming majority of nodes, which carry
    // the huge NO_CLIP sentinel instead of a real rect.
    if (in.clip_position.x < in.clip_rect[1] || in.clip_position.x > in.clip_rect[3]
        || in.clip_position.y < in.clip_rect[0] || in.clip_position.y > in.clip_rect[2]) {
        discard;
    }

    var border_width: f32 = in.border_width;
    var top: f32 = in.rect[0];
    var left: f32 = in.rect[1];
    var bottom: f32 = in.rect[2];
    var right: f32 = in.rect[3];

    let half_size = vec2<f32>((right - left) * 0.5, (bottom - top) * 0.5);
    let center = vec2<f32>(left + half_size.x, top + half_size.y);
    // Radius can't exceed half the box's shorter side - past that a rounded
    // rect is just a stadium/circle, and the SDF above stops making sense.
    let radius = min(in.corner_radius, min(half_size.x, half_size.y));
    let dist = sd_rounded_box(in.clip_position.xy - center, half_size, radius);

    // fwidth(dist) is roughly how much `dist` changes between this fragment and
    // its neighbors - halving it as the smoothstep *half*-width keeps the
    // anti-aliased edge about 1 screen pixel wide *total* regardless of
    // resolution/DPI (smoothstep(-aa, aa, ...) spans 2*aa). Using the full
    // fwidth() as the half-width instead (a common off-by-2x mistake) doubles
    // that to ~2px, which barely shows on a thick border but visibly bloats a
    // thin one - especially on a curve, since a curved/diagonal edge already
    // crosses more partially-covered pixels than a perfectly axis-aligned
    // straight one, so the same over-wide band reads as noticeably softer there.
    let aa = max(fwidth(dist) * 0.5, 0.0001);
    if (dist > aa) {
        discard;
    }

    // background_blur is a pixel radius (0 = off, the default - see
    // Style::resolve_concrete), same units as e.g. Tailwind's backdrop-blur-*
    // scale (sm~4, md~12, lg~16, xl~24, 2xl~40, 3xl~64). Below 0.0 nothing
    // changes from before this existed: fill_color is just the flat in.color.
    // Above 0.0, this matches CSS/Tailwind's own backdrop-filter model: blur
    // whatever's behind, then draw this node's background_color normally on top
    // of THAT instead of the sharp destination - background_color's own alpha is
    // what controls how much of the blur shows through / how tinted it looks
    // (bg-white/30 + backdrop-blur-md, in Tailwind terms), background_blur is
    // purely the radius. The radius itself just crossfades between the sharp and
    // fully-blurred textures (see MAX_BLUR_RADIUS) rather than sampling a truly
    // continuous per-radius blur - not physically exact, but every intermediate
    // value stays as smooth/artifact-free as the endpoints, which is what
    // actually matters for a UI panel.
    //
    // t_scene/t_blurred only ever hold the 3D scene (captured once, before any
    // UI draws that frame - see BlurRender) - never other UI elements. So a
    // blurred node's own backdrop sample can't "see" e.g. a panel drawn earlier
    // in this same UI pass sitting behind it; it can only ever blur the raw 3D
    // scene. Outputting fill_color.a as in.color.a (not a hardcoded 1.0) instead
    // lets the pipeline's normal SrcAlpha/OneMinusSrcAlpha blending do the rest,
    // same as any other translucent node: at low alpha, whatever's genuinely in
    // the destination (a panel drawn earlier, sharp scene, whatever) shows
    // through underneath this node's own blur+tint instead of being replaced
    // outright - not a true "blur of what's actually behind", but close enough
    // to read as layered glass instead of a hole punched through everything
    // beneath it, which a forced-opaque output was doing before.
    var fill_color = in.color;
    var border_color = in.border_color;
    if (in.background_blur > 0.0) {
        let uv = in.clip_position.xy / u_screen_size.xy;
        let sharp = textureSample(t_scene, s_scene, uv);
        let blurred = textureSample(t_blurred, s_blurred, uv);
        let t = saturate(in.background_blur / MAX_BLUR_RADIUS);
        var backdrop = mix(sharp, blurred, t).rgb;

        // Plain blur alone reads as "out of focus" (soft, but still literally the
        // same image) rather than "glass" - real frosted/etched glass scatters
        // light unevenly as it passes through, which does two things a defocused
        // lens doesn't: intensifies whatever color comes through (compare a photo
        // through a frosted shower door to a defocused camera shot of the same
        // scene) and breaks up smooth gradients into faint grain. Both scale with
        // `t` so a barely-blurred node doesn't get the full "heavily frosted"
        // treatment meant for a fully blurred one.
        backdrop = adjust_saturation(backdrop, mix(1.0, 1.35, t));
        let grain = (hash12(in.clip_position.xy) - 0.5) * 0.05 * t;
        backdrop = saturate(backdrop + vec3<f32>(grain, grain, grain));

        // border_color defaults to background_color (see Style::resolve_concrete)
        // specifically so an unset border stays invisible, blending seamlessly
        // with the fill instead of showing as a hairline of transparency - but
        // that trick only works if fill and border are computed the same way.
        // Without this, the border band stayed flat in.border_color while the
        // fill above got the blurred/tinted treatment, so "the same declared
        // color" ended up looking like two different colors - a border ring
        // around every blurred card that was never explicitly set. Blurring the
        // border through the identical backdrop keeps the two visually
        // identical whenever their underlying colors actually match, same as
        // it already worked before background_blur existed.
        // Alpha forced to 1.0 here (not in.color.a/in.border_color.a) - .rgb above
        // already blends this node's own tint with `backdrop` (the blurred/sharp
        // crossfade), which IS the correct final color, matching real CSS
        // backdrop-filter semantics: the blurred backdrop fully replaces whatever
        // was behind this node: nothing further should show through. Outputting
        // the node's own alpha here instead (as every other, non-blurred node
        // correctly does two lines up) would additionally hardware-blend this
        // result against the destination - but the destination the UI pass draws
        // into already holds the raw, UNBLURRED sharp scene (see BlurRender's own
        // doc comment: it blits the sharp copy to the swapchain before this pass
        // runs) rather than `backdrop`. That let a `background_color` alpha low
        // enough to make the blur actually visible also leak that same fraction
        // of the literal sharp scene straight through, completely bypassing
        // `t_blurred` - a bright sky behind a nominally "dark, heavily blurred"
        // panel read as barely-tinted white, no matter how high the blur radius
        // was cranked, since that leak was never routed through the blur at all.
        fill_color = vec4<f32>(mix(backdrop, in.color.rgb, in.color.a), 1.0);
        border_color = vec4<f32>(mix(backdrop, in.border_color.rgb, in.border_color.a), 1.0);
    }

    var border_mix: f32;
    if (in.border_edges == 15u) {
        // All 4 edges (the default - see BorderEdges) - same reasoning as the
        // outer edge below, applied to the fill/border boundary too (around
        // dist == -border_width) - a hard step there was just as jagged on a
        // rounded corner as the outer edge was.
        border_mix = smoothstep(-border_width - aa, -border_width + aa, dist);
    } else {
        // A partial border (e.g. BorderEdges::LEFT, a hover accent bar) can't
        // reuse the rounded-box SDF above - that has no notion of "which edge"
        // a given fragment's border band belongs to, only "distance to the
        // nearest edge of any kind". Instead, each enabled edge gets its own
        // straight distance-from-that-edge check (0 at the edge itself,
        // increasing inward) - the smallest of those (across only the enabled
        // edges) is this fragment's real distance into A border band, same
        // "fade over ~1px" treatment as the rounded case, just not curve-aware.
        // Fine for the sharp-cornered case this exists for; won't follow a
        // rounded corner if used on one (see BorderEdges' own doc comment).
        var edge_dist = 3.4e38;
        if ((in.border_edges & 1u) != 0u) {
            edge_dist = min(edge_dist, in.clip_position.x - left);
        }
        if ((in.border_edges & 2u) != 0u) {
            edge_dist = min(edge_dist, right - in.clip_position.x);
        }
        if ((in.border_edges & 4u) != 0u) {
            edge_dist = min(edge_dist, in.clip_position.y - top);
        }
        if ((in.border_edges & 8u) != 0u) {
            edge_dist = min(edge_dist, bottom - in.clip_position.y);
        }
        border_mix = smoothstep(border_width + aa, border_width - aa, edge_dist);
    }
    var out_color = mix(fill_color, border_color, border_mix);

    // Straight edges mostly land on pixel boundaries and hide a hard cutoff;
    // curved corners cross pixels diagonally and show it as visible stair-
    // stepping - fading alpha smoothly across the SDF's zero crossing (instead
    // of a hard discard right at dist > 0) fixes that for both.
    let coverage = 1.0 - smoothstep(-aa, aa, dist);
    out_color.a = out_color.a * coverage;

    return out_color;
}