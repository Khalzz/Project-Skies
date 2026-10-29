// engine/tiled - a material's texture at a fixed size, repeated across the
// surface however big it is (see `Material::tiled`). Each pixel takes its
// place in the texture from where it is - on its object (scaled like it),
// or in the world - on the plane its surface most faces: a floor or a top
// across X and Z, a side across X or Z, and Y.

struct Params {
    // Meters one copy of the texture covers.
    tile_size: f32,
    // 1: measured in the world - 0: on the object.
    world: f32,
}

fn fragment(f: FragmentIn) -> vec4<f32> {
    let in_world = params.world > 0.5;
    // On the object, from its lowest corner - so a face shows the texture
    // from its edge, a whole copy every `tile_size`.
    let position = select(f.local_position - f.local_bounds_min, f.world_position, in_world);
    let normal = select(f.local_normal, f.normal, in_world);
    let facing = abs(normal);

    // How fast the position changes from one pixel to the next - smooth
    // across a corner, unlike the texture coordinates below (which jump from
    // one face's plane to the next's). It picks the mipmap size: from the
    // coordinates, the GPU would take a corner for something tiny and far.
    let step_x = dpdx(position);
    let step_y = dpdy(position);

    // Across the texture is the way that's right for someone looking at the
    // surface from in front of it - which flips between opposite sides, or
    // the back of a box would show its texture mirrored.
    var uv: vec2<f32>;
    var uv_x: vec2<f32>;
    var uv_y: vec2<f32>;
    if (facing.y >= facing.x && facing.y >= facing.z) {
        let flip = vec2<f32>(1.0, sign(normal.y));
        uv = position.xz * flip;
        uv_x = step_x.xz * flip;
        uv_y = step_y.xz * flip;
    } else if (facing.x >= facing.z) {
        let flip = vec2<f32>(-sign(normal.x), -1.0);
        uv = position.zy * flip;
        uv_x = step_x.zy * flip;
        uv_y = step_y.zy * flip;
    } else {
        let flip = vec2<f32>(sign(normal.z), -1.0);
        uv = position.xy * flip;
        uv_x = step_x.xy * flip;
        uv_y = step_y.xy * flip;
    }
    let scale = 1.0 / max(params.tile_size, 0.0001);
    let color = textureSampleGrad(t_diffuse, s_diffuse, uv * scale, uv_x * scale, uv_y * scale);
    return vec4<f32>(f.lighting * color.rgb, color.a);
}
