// The template every Surface shader is built from (see engine::rendering::
// shader_library): the engine's inputs, the vertex and fragment work, and
// two hooks the shader fills in - pasted where the HOOKS line is. A hook it
// leaves out gets the default: `vertex` changes nothing, `fragment` returns
// the lit color. "engine/standard" is this with both defaults.
//
// Inputs (the model pipelines' bind groups - there are only four):
//   0 - the mesh's texture   1 - the camera
//   2 - the mesh's transform 3 - the light (it carries the time too)

struct CameraUniform {
    view_proj: mat4x4<f32>,
    view_pos: vec4<f32>,
    near: f32,
    far: f32,
};

struct Transform {
    model_matrix: mat4x4<f32>,
    // The lowest corner of the mesh's geometry, in its model (see
    // MeshUniform).
    bounds_min: vec4<f32>,
};

struct Light {
    position: vec3<f32>,
    color: vec3<f32>,
    time: f32,
    camera_position: vec3<f32>,
    ambient: vec3<f32>,
}

@group(0) @binding(0)
var t_diffuse: texture_2d<f32>;
@group(0) @binding(1)
var s_diffuse: sampler;

@group(1) @binding(0)
var<uniform> camera: CameraUniform;

@group(2) @binding(0)
var<uniform> transform: Transform;

@group(3) @binding(0)
var<uniform> light: Light;

// What every hook can read, as `engine`.
struct Engine {
    // Seconds, counting up (back to 0 every 10000 s).
    time: f32,
    // The camera's true position in the world - every position a hook gets
    // is relative to it (add it back for the true world position).
    camera_position: vec3<f32>,
    // Which way the sun is, and its light's color.
    sun_direction: vec3<f32>,
    sun_color: vec3<f32>,
    // The light from the whole sky, falling on every surface whichever
    // way it faces.
    ambient_color: vec3<f32>,
    // The camera's near and far planes - what `linear_depth` uses.
    near: f32,
    far: f32,
}
var<private> engine: Engine;

fn engine_values() -> Engine {
    return Engine(light.time, light.camera_position, normalize(light.position), light.color, light.ambient, camera.near, camera.far);
}

// How far from the camera (world units) a raw depth is - one read from the
// depth so far (`reads: [Depth]`), or a point's own (its clip-space z / w).
// Depths here are reversed: 1 at the near plane, 0 at the far one.
fn linear_depth(raw: f32) -> f32 {
    let z = 1.0 - 2.0 * raw;
    return 2.0 * camera.near * camera.far / (camera.far + camera.near - z * (camera.far - camera.near));
}

// The `vertex` hook's vertex: where it is (relative to the camera), its
// normal and its texture coordinates.
struct VertexIn {
    position: vec3<f32>,
    normal: vec3<f32>,
    uv: vec2<f32>,
}

// The `fragment` hook's pixel.
struct FragmentIn {
    // Where it is: relative to the camera, and in the world.
    position: vec3<f32>,
    world_position: vec3<f32>,
    // The surface's normal (unit length), facing the side that's seen.
    normal: vec3<f32>,
    uv: vec2<f32>,
    // The mesh's own texture here, and the color engine/standard draws -
    // lit by the sun, fogged far away.
    base_color: vec4<f32>,
    lit_color: vec4<f32>,
    front_facing: bool,
    // Where it is on its object, and the way its surface faces there - in
    // the object's own space, scaled like it: a point 1 m up a cube scaled
    // to 4 m tall is at y = 1 - turning or moving the object changes
    // neither.
    local_position: vec3<f32>,
    local_normal: vec3<f32>,
    // The lowest corner of the mesh's geometry, in the same space as
    // `local_position` - `local_position - local_bounds_min` measures from it
    // (0 at that corner, the mesh's size at the opposite one).
    local_bounds_min: vec3<f32>,
    // The light falling here (the sun and the ambient light) - times a
    // color, that color lit the way `lit_color` is (before far fog).
    lighting: vec3<f32>,
}

//@HOOKS

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) tex_coords: vec2<f32>,
    @location(2) normal: vec3<f32>,
}

struct InstanceInput {
    @location(5) model_matrix_0: vec4<f32>,
    @location(6) model_matrix_1: vec4<f32>,
    @location(7) model_matrix_2: vec4<f32>,
    @location(8) model_matrix_3: vec4<f32>,

    // Lighting rotation
    @location(9) normal_matrix_0: vec3<f32>,
    @location(10) normal_matrix_1: vec3<f32>,
    @location(11) normal_matrix_2: vec3<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
    @location(1) world_normal: vec3<f32>,
    @location(2) world_position: vec3<f32>,
    @location(3) view_depth: f32, // Used for fog calculation
    @location(4) local_position: vec3<f32>,
    @location(5) local_normal: vec3<f32>,
    @location(6) local_bounds_min: vec3<f32>,
};

@vertex
fn vs_main(model: VertexInput, instance: InstanceInput) -> VertexOutput {
    engine = engine_values();
    let model_matrix = mat4x4<f32>(
        instance.model_matrix_0,
        instance.model_matrix_1,
        instance.model_matrix_2,
        instance.model_matrix_3,
    );

    let normal_matrix = mat3x3<f32>(
        instance.normal_matrix_0,
        instance.normal_matrix_1,
        instance.normal_matrix_2,
    );

    let world_position = model_matrix * transform.model_matrix * vec4<f32>(model.position, 1.0);
    let v = vertex(VertexIn(world_position.xyz, normal_matrix * model.normal, model.tex_coords));

    var out: VertexOutput;
    out.tex_coords = v.uv;
    out.world_normal = v.normal;
    out.world_position = v.position;
    out.clip_position = camera.view_proj * vec4<f32>(v.position, 1.0);

    // Calculate view-space depth for fog, using the camera's view position
    out.view_depth = length(camera.view_pos.xyz - out.world_position);

    // Back into the object's own (scaled) space: the instance's matrix is
    // place x turn x size, and `normal_matrix` its turn alone - so undoing
    // the place and the turn leaves the size in.
    let unturn = transpose(normal_matrix);
    out.local_position = unturn * (v.position - model_matrix[3].xyz);
    out.local_normal = unturn * v.normal;
    // The corner sized like the object: its size is the length of each of
    // its matrix's first three columns.
    let size = vec3<f32>(length(model_matrix[0].xyz), length(model_matrix[1].xyz), length(model_matrix[2].xyz));
    out.local_bounds_min = size * transform.bounds_min.xyz;

    return out;
}

@fragment
fn fs_main(in: VertexOutput, @builtin(front_facing) front_facing: bool) -> @location(0) vec4<f32> {
    engine = engine_values();
    let object_color: vec4<f32> = textureSample(t_diffuse, s_diffuse, in.tex_coords);

    // For double-sided meshes (backface culling off) the back face is
    // rasterized with the same interpolated normal as the front - flip it so
    // it's lit as the surface it actually is. Single-sided meshes only ever
    // hit the front face, so this is a no-op for them.
    var normal = normalize(in.world_normal);
    if (!front_facing) {
        normal = -normal;
    }

    let light_dir = normalize(light.position - in.world_position);

    let diffuse_strength = max(dot(normal, light_dir), 0.0);
    let diffuse_color = light.color * diffuse_strength;

    let lighting = light.ambient + diffuse_color;
    let result = lighting * object_color.xyz;

    // Long-distance atmospheric haze - same start/end as water.wgsl's fog,
    // fading toward the sky's horizon haze (SkyboxRender::new_procedural's
    // haze_color), since distant objects are seen against the sky.
    let fog_start = 80000.0;
    let fog_end = 1400000.0;
    let fog_factor = smoothstep(fog_start, fog_end, in.view_depth);
    let fogged_color = mix(result, vec3<f32>(0.448, 0.604, 0.711), fog_factor);
    let lit = vec4<f32>(fogged_color, object_color.a);

    return fragment(FragmentIn(in.world_position, in.world_position + engine.camera_position, normal, in.tex_coords, object_color, lit, front_facing, in.local_position, normalize(in.local_normal), in.local_bounds_min, lighting));
}
