
// the elements brought here by the render buffer
struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) rect: vec4<f32>,
    @location(3) border_color: vec4<f32>,
    @location(4) corner_radius: f32,
    @location(5) border_width: f32,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) rect: vec4<f32>,
    @location(2) border_color: vec4<f32>,
    @location(3) corner_radius: f32,
    @location(4) border_width: f32,
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

    return out;
}

// Signed distance from `p` (relative to the box's own center) to a box of
// `half_size`, corners rounded by `radius` - negative inside, 0 on the edge,
// positive outside. Standard rounded-box SDF (Inigo Quilez).
fn sd_rounded_box(p: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let q = abs(p) - half_size + vec2<f32>(radius, radius);
    return min(max(q.x, q.y), 0.0) + length(max(q, vec2<f32>(0.0, 0.0))) - radius;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
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

    // Same reasoning as the outer edge below, applied to the fill/border boundary
    // too (around dist == -border_width) - a hard step there was just as jagged on
    // a rounded corner as the outer edge was.
    let border_mix = smoothstep(-border_width - aa, -border_width + aa, dist);
    var out_color = mix(in.color, in.border_color, border_mix);

    // Straight edges mostly land on pixel boundaries and hide a hard cutoff;
    // curved corners cross pixels diagonally and show it as visible stair-
    // stepping - fading alpha smoothly across the SDF's zero crossing (instead
    // of a hard discard right at dist > 0) fixes that for both.
    let coverage = 1.0 - smoothstep(-aa, aa, dist);
    out_color.a = out_color.a * coverage;

    return out_color;
}