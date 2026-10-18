// An example Surface shader with no .ron - so the defaults: drawn like
// engine/standard (solid meshes in "opaque", see-through ones in
// "transparent"), only its color changed. Try it on a node with
// `Material::shader("examples/rust")`.
//
// It's only a `fragment` hook - the engine's template gives it the pixel
// (`f`, see standard_template.wgsl's FragmentIn) and `engine.*`.

fn fragment(f: FragmentIn) -> vec4<f32> {
    // The normal lit look, browned and darkened - an old, rusty finish -
    // with blotches that stay put on the surface (true world position).
    let blotch = 0.85 + 0.15 * sin(f.world_position.x * 3.1) * sin(f.world_position.z * 2.7);
    return vec4<f32>(f.lit_color.rgb * vec3<f32>(0.8, 0.55, 0.4) * blotch, f.lit_color.a);
}
