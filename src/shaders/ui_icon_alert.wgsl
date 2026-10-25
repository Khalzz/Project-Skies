// engine/icon_alert - a yellow warning triangle with a dark "!", drawn over
// the whole box (see widgets::toast). Shapes are distance fields in a -1..1
// square, y up, so it stays sharp at any size.

fn sd_triangle_up(p: vec2<f32>, r: f32) -> f32 {
    let k = sqrt(3.0);
    var q = vec2<f32>(abs(p.x) - r, p.y + r / k);
    if (q.x + k * q.y > 0.0) {
        q = vec2<f32>(q.x - k * q.y, -k * q.x - q.y) / 2.0;
    }
    q.x -= clamp(q.x, -2.0 * r, 0.0);
    return -length(q) * sign(q.y);
}

fn sd_box(p: vec2<f32>, half_size: vec2<f32>) -> f32 {
    let d = abs(p) - half_size;
    return length(max(d, vec2<f32>(0.0))) + min(max(d.x, d.y), 0.0);
}

fn fragment(f: UiIn) -> vec4<f32> {
    let p = vec2<f32>(f.uv.x - 0.5, 0.5 - f.uv.y) * 2.0;
    let aa = max(length(vec2<f32>(dpdx(p.x), dpdy(p.y))), 0.0001);

    // Rounded corners: a slightly smaller triangle, grown back by the radius.
    let triangle = sd_triangle_up(p + vec2<f32>(0.0, 0.22), 0.8) - 0.1;
    let bar = sd_box(p - vec2<f32>(0.0, 0.12), vec2<f32>(0.075, 0.25)) - 0.03;
    let point = length(p - vec2<f32>(0.0, -0.36)) - 0.1;
    let mark = min(bar, point);

    let shape = 1.0 - smoothstep(-aa, aa, triangle);
    let in_mark = 1.0 - smoothstep(-aa, aa, mark);
    let yellow = vec3<f32>(1.0, 0.78, 0.16);
    let dark = vec3<f32>(0.1, 0.08, 0.02);
    return vec4<f32>(mix(yellow, dark, in_mark), shape);
}
