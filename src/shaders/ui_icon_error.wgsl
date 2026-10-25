// engine/icon_error - a red circle with a white "x", drawn over the whole
// box (see widgets::toast). Shapes are distance fields in a -1..1 square,
// so it stays sharp at any size.

fn sd_segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

fn fragment(f: UiIn) -> vec4<f32> {
    let p = (f.uv - vec2<f32>(0.5)) * 2.0;
    let aa = max(length(vec2<f32>(dpdx(p.x), dpdy(p.y))), 0.0001);

    let circle = length(p) - 0.92;
    let arm = 0.36;
    let x_mark = min(
        sd_segment(p, vec2<f32>(-arm, -arm), vec2<f32>(arm, arm)),
        sd_segment(p, vec2<f32>(-arm, arm), vec2<f32>(arm, -arm)),
    ) - 0.1;

    let shape = 1.0 - smoothstep(-aa, aa, circle);
    let in_cross = 1.0 - smoothstep(-aa, aa, x_mark);
    let red = vec3<f32>(0.9, 0.2, 0.2);
    return vec4<f32>(mix(red, vec3<f32>(1.0), in_cross), shape);
}
