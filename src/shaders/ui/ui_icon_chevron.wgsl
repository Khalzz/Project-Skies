// engine/icon_chevron - a downward chevron (a "v" stroke) in the box's own
// background color (so its disabled look can fade it), drawn over the whole
// box (see widgets::dropdown). A distance field in a -1..1 square, so it
// stays sharp at any size.

fn sd_segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

fn fragment(f: UiIn) -> vec4<f32> {
    let p = vec2<f32>(f.uv.x - 0.5, 0.5 - f.uv.y) * 2.0;
    let aa = max(length(vec2<f32>(dpdx(p.x), dpdy(p.y))), 0.0001);

    let tip = vec2<f32>(0.0, -0.3);
    let stroke = min(
        sd_segment(p, vec2<f32>(-0.7, 0.35), tip),
        sd_segment(p, vec2<f32>(0.7, 0.35), tip),
    ) - 0.13;

    let shape = 1.0 - smoothstep(-aa, aa, stroke);
    return vec4<f32>(f.color.rgb, f.color.a * shape);
}
