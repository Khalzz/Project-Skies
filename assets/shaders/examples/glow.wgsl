// An example Surface shader with settings (glow.ron): the whole model drawn
// in the "transparent" pass, added onto what's behind it, both sides,
// pulsing - and the edges brightest. Try it on a node with
// `Material::shader("examples/glow")`, or in another color:
// `Material::shader("examples/glow").param("color", [1.0, 0.3, 0.1, 1.0])`.

// Its own values - `params` in the hooks. Defaults in glow.ron; a material
// can set any of them (by field name).
struct Params {
    color: vec4<f32>,
    pulse_speed: f32,
    rim_power: f32,
};

fn fragment(f: FragmentIn) -> vec4<f32> {
    // Brightest where the surface turns away from the eye - its outline.
    let to_eye = normalize(-f.position);
    let rim = pow(1.0 - abs(dot(f.normal, to_eye)), params.rim_power);
    let pulse = 0.6 + 0.4 * sin(engine.time * params.pulse_speed);
    let glow = params.color.rgb * (0.15 + rim) * pulse;
    return vec4<f32>(glow, params.color.a);
}
