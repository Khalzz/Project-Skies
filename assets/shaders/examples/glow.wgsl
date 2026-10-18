// An example Surface shader with settings (glow.ron): the whole model drawn
// in the "transparent" pass, added onto what's behind it, both sides,
// pulsing - and the edges brightest. Try it on a node with
// `Material::shader("examples/glow")`.

fn fragment(f: FragmentIn) -> vec4<f32> {
    // Brightest where the surface turns away from the eye - its outline.
    let to_eye = normalize(-f.position);
    let rim = pow(1.0 - abs(dot(f.normal, to_eye)), 2.5);
    let pulse = 0.6 + 0.4 * sin(engine.time * 3.0);
    let glow = vec3<f32>(0.2, 0.8, 1.0) * (0.15 + rim) * pulse;
    return vec4<f32>(glow, 1.0);
}
