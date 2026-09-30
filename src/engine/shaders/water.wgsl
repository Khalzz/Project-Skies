// Water shader - shares the same camera/mesh-transform/light bind groups
// (1/2/3) every other model-drawing pipeline uses, but group 0 is its own
// thing: not a material's diffuse texture (water never samples one, always
// outputs a computed color instead - see WATER_COLOR/DEEP_COLOR/
// SKY_REFLECTION_COLOR below), it's a snapshot of the depth buffer taken
// right after opaque geometry finishes drawing (see DepthRender::
// foam_depth_copy and render_pass.rs's own render_water_pass) - used for
// shore-intersection foam, see fs_main's own comment on that. This is also
// why water draws through its own manual render_water_pass instead of the
// generic draw_model_instanced_from_list every other model goes through -
// that helper always binds each mesh's own material at group 0, which would
// stomp this.

// Matches every SceneCameras::create_camera call site's znear/zfar today
// (see Projection::new's own callers) - CameraUniform *does* carry near/far
// too, but they're set once at CameraResources::new and never actually
// updated to track the active camera's own current Projection.znear/zfar
// (which can diverge, e.g. cockpit view sets znear=0.01) - a pre-existing
// staleness this shader doesn't try to fix, just avoids by not extending
// CameraUniform's own WGSL declaration further (risks a shader/buffer size
// mismatch - CameraUniform's real GPU buffer isn't padded to WGSL's own
// implied struct size for it) and hardcoding the same default instead.
const NEAR: f32 = 0.1;
// Must match the camera's projection far plane (see camera/handler.rs) or the
// linearize_depth() calls for shore foam read the depth buffer wrong.
const FAR: f32 = 4000000.0;

// Darkest tint, at the wave's own lowest point (height_01 = 0 in fs_main) -
// lightened from an earlier, near-black pass at this (0.0, 0.03, 0.08):
// height_01 = 0 is an ordinary trough in moderate chop, not a dramatic one,
// and near-black there read as an abrupt, out-of-place dark spot. height_01
// is scaled by a TYPICAL crest height (ocean.params.w), not the rare
// in-phase maximum, so ordinary troughs actually reach this color.
const DEEP_COLOR: vec3<f32> = vec3<f32>(0.01, 0.12, 0.23);
// Lightest tint, at the wave's own highest point (height_01 = 1) - classic
// sea blue (blue clearly dominant over green) - an earlier pass at this
// leaned teal (green pushed up relative to blue), which read as too green
// rather than like open sea water.
const WATER_COLOR: vec3<f32> = vec3<f32>(0.02, 0.25, 0.45);
// Light sky-toned BLUE the surface reflects toward at grazing viewing
// angles - see fresnel_amount in fs_main. Real water's own color barely
// matters at a shallow viewing angle (you're mostly seeing reflected sky,
// not the water itself) - a flat, angle-independent color is a big part of
// why water can look flat/fake even when the base tint itself is right;
// this alone is most of what actually reads as "real" here, more than the
// exact WATER_COLOR/DEEP_COLOR values. Deliberately NOT teal-shifted like
// WATER_COLOR - this is meant to read as reflected sky, not water itself.
// Added to the lit water color, never lit itself (see the final
// composition in fs_main) - a reflection is sky light already.
const SKY_REFLECTION_COLOR: vec3<f32> = vec3<f32>(0.42, 0.58, 0.78);
// Base color of the far open sea - the water eases into it between
// OPEN_SEA_BLEND_START and OPEN_SEA_BLEND_END (distance from the camera),
// well out toward the horizon haze, so it reads as part of the sky/sea
// transition rather than a line. A deep blue, a little darker than the wave
// colors' middle.
const OPEN_SEA_COLOR: vec3<f32> = vec3<f32>(0.015, 0.15, 0.28);
// Once every wave is too small to show at a pixel, the fresnel term treats
// the surface as if it never faced the viewer at less than this (cosine of
// the view angle) - a real distant sea is still covered in waves whose
// faces tilt toward you, so it never turns mirror-like at grazing angles the
// way a truly flat plane would (that's what made far water read as
// washed-out sky). Applied in proportion to how much of the waves' slope
// has faded out at that pixel (see `lost_slope` in fs_main), so it eases in
// exactly as the visible wave detail eases out - no fixed distance.
// Caps the sky reflection at roughly (1 - this)^FRESNEL_POWER.
const FLAT_WATER_MIN_FACING: f32 = 0.3;
// See OPEN_SEA_COLOR - distance from the camera, world units.
const OPEN_SEA_BLEND_START: f32 = 4000.0;
const OPEN_SEA_BLEND_END: f32 = 40000.0;
// What the sea turns toward the horizon - an open-ocean blue (see
// SkyboxRender::new_procedural): lighter than the water up close (grazing
// views mirror more of the bright low sky, and there's more haze in front
// of it), but darker than the sky's haze just above the horizon, which is
// what makes the horizon read as a line. The water eases into it over
// SEA_HAZE_START..SEA_HAZE_END (horizontal distance), and the far fog ends
// on it too. Keep in sync with the sky's sea_horizon_color.
const SEA_HORIZON_COLOR: vec3<f32> = vec3<f32>(0.147, 0.296, 0.477);
const SEA_HAZE_START: f32 = 3000.0;
const SEA_HAZE_END: f32 = 80000.0;

// The water mesh is one set of nested square rings around the camera (see
// resources::build_water_rings_mesh / main.rs's OCEAN_TIERS): coarser the
// further out, and every vertex knows its ring's cell size and outer
// half-extent (tex_coords) and the next ring's cell size (normal.x). A ring
// only MOVES the waves it's fine enough to draw - a wave needs a few cells
// per wavelength, or vertices would skip across its crests. The mesh only
// SHAPES the surface, though: how the water looks (normal, color,
// whitecaps) is computed per pixel from every wave everywhere (see
// surface_waves), so nothing changes where the moving rings end.
//
// A wave is fully moved by the mesh once it spans GEOMETRY_FULL_CELLS
// cells, and left entirely to the per-pixel normal below GEOMETRY_MIN_CELLS.
const GEOMETRY_MIN_CELLS: f32 = 3.0;
const GEOMETRY_FULL_CELLS: f32 = 5.0;
// Every other vertex slides onto the next ring's coarser grid as its TRUE
// (circular) distance from the camera goes from this fraction of the ring's
// half-extent to the full half-extent - so quality changes in circles around
// the camera, not in the rings' square outlines. The rings' square edges
// (and their corners, out past the half-extent) are therefore always fully
// morphed, which is what makes them meet the next ring exactly. Must stay
// above 1/sqrt(2) (~0.707): a ring's inner corners sit at that fraction of
// its half-extent, and must not morph at all, to meet the ring inside it.
const RING_MORPH_START: f32 = 0.72;
// Over the last part of each ring (this fraction of its half-extent out to
// the edge), the waves it moves fade down to what the NEXT ring can carry -
// for a next ring twice as coarse that's what the morph above already does,
// but where the next ring jumps to much bigger (flat) cells this is what
// flattens the water right at the edge, so the rings still meet exactly.
const NEXT_RING_FADE_START: f32 = 0.9;



// The waves themselves come from `ocean` (group 0, binding 2) - generated on
// the CPU from a wind-sea spectrum by engine::rendering::enviroment::ocean
// (see that module's doc comment), which is also what gameplay asks for the
// surface height, so both always agree. Two groups, in that order in
// `ocean.waves`:
// - geometry waves (ocean.params.x of them) - Gerstner waves displacing the
//   mesh in vs_main: height plus a sideways push toward each crest, which
//   sharpens crests and flattens troughs;
// - detail waves (ocean.params.y of them) - too short for the mesh, so they
//   only bend the per-pixel normal in fs_main (ripples, glints), each fading
//   out once it would be smaller than a few pixels on screen.
// Wavelengths are all unrelated to each other, so the pattern never repeats.
//
// Each wave's phase at the CAMERA arrives precomputed (in f64, on the CPU);
// the shader only adds k * (distance from the camera along the wave), using
// the camera-relative positions it already works in. That keeps every
// number small, so waves stay crisp arbitrarily far from the world origin.

// Must match engine::rendering::enviroment::ocean::MAX_WAVES.
const MAX_WAVES: u32 = 80u;
// Must match engine::rendering::enviroment::ocean::MAX_WAKE_POINTS.
const MAX_WAKE_POINTS: u32 = 32u;

// A plane flying fast and low stirs up the water (see water_wake, and
// WaterRenderData's wake for when/how strongly) - shaped by which way it's
// flying, like a boat's wake, not a round drop:
// - under it, a small patch flattened and whitened by the blast,
//   WAKE_PATCH_RADIUS across its width (+ WAKE_PATCH_GROWTH per unit of
//   height - higher spreads wider), WAKE_PATCH_STRETCH times as long along
//   the flight path, sitting a little behind the plane;
// - behind it, a V of ripples spreading out to both sides at
//   WAKE_V_SLOPE (sideways per unit back - 0.36 is ~20 degrees), fading
//   out WAKE_V_LENGTH back;
// - and a foam streak along its trail, WAKE_TRAIL_WIDTH wide when fresh
//   and WAKE_TRAIL_SPREAD wider per second of age, fading out.
// The *_FOAM / WAKE_FLATTEN / WAKE_RIPPLE_SLOPE values are its strength.
const WAKE_PATCH_RADIUS: f32 = 5.0;
const WAKE_PATCH_GROWTH: f32 = 0.25;
const WAKE_PATCH_STRETCH: f32 = 2.0;
const WAKE_PATCH_FOAM: f32 = 0.75;
const WAKE_FLATTEN: f32 = 0.6;
const WAKE_V_SLOPE: f32 = 0.36;
const WAKE_V_LENGTH: f32 = 80.0;
const WAKE_RIPPLE_WAVELENGTH: f32 = 4.0;
const WAKE_RIPPLE_SPEED: f32 = 6.0;
const WAKE_RIPPLE_SLOPE: f32 = 0.2;
const WAKE_TRAIL_WIDTH: f32 = 5.0;
const WAKE_TRAIL_SPREAD: f32 = 6.0;
const WAKE_TRAIL_FOAM: f32 = 0.8;
// The trail's foam is broken up by noise pinned to the water: dense when
// fresh, thinning to patches as it ages - WAKE_FOAM_COVERAGE_FRESH/_OLD are
// how much of it survives (0..1) at those two ends. The noise repeats every
// WAKE_NOISE_PERIOD meters (ocean.rs's WAKE_NOISE_PERIOD, keep in sync).
const WAKE_FOAM_COVERAGE_FRESH: f32 = 0.85;
const WAKE_FOAM_COVERAGE_OLD: f32 = 0.35;
const WAKE_NOISE_PERIOD: f32 = 4096.0;
const WAKE_FOAM_COLOR: vec3<f32> = vec3<f32>(0.85, 0.9, 0.95);
// A detail wave fully shows once its wavelength spans this many pixels, and
// fades to nothing by half that - shorter than that it would only shimmer.
const DETAIL_FULL_PIXELS: f32 = 8.0;
// Whitecap tint - brighter/whiter than FOAM_COLOR's shore foam.
const WHITECAP_COLOR: vec3<f32> = vec3<f32>(0.85, 0.9, 0.95);
// Whitecaps show where a crest is pinched by more than this fraction of the
// maximum (see crest in vs_main), fully by WHITECAP_FULL.
const WHITECAP_START: f32 = 0.45;
const WHITECAP_FULL: f32 = 0.85;
// How tightly the fresnel blend (see fs_main) concentrates toward true
// grazing angles - higher means only very shallow viewing angles pick up
// SKY_REFLECTION_COLOR, most of the surface stays true water color; lower
// spreads the reflective look further back toward straight-down views.
const FRESNEL_POWER: f32 = 4.0;
// How much of the fresnel blend actually reaches SKY_REFLECTION_COLOR even
// at a full 90-degree grazing angle - 1.0 would let grazing views go fully
// sky-colored, this caps it short of that so the surface never completely
// stops reading as water.
const FRESNEL_STRENGTH: f32 = 0.6;

// Foam tint for shore/object intersections - see shore_foam in fs_main.
const FOAM_COLOR: vec3<f32> = vec3<f32>(0.55, 0.75, 0.95);
// How deep the water can be (vertically, world units) over solid ground and
// still foam - 0 (water right at the ground: the shoreline, an object's
// waterline) is full foam, fading to none at this depth. Measured straight
// down, not along the view ray, so the band keeps the same width in the
// world from any distance or viewing angle. Larger = a wider band.
const SHORE_FOAM_DEPTH: f32 = 1.5;
// Shore / object-collision foam (and whitecaps) fade out with the same
// distance blend as the wave colors (wave_visibility in fs_main) - they're
// near-surface details, so they shouldn't fringe every far coastline.

struct CameraUniform {
    view_proj: mat4x4<f32>,
    view_pos: vec4<f32>,
};

struct Transform {
    model_matrix: mat4x4<f32>,
};

struct Light {
    position: vec3<f32>,
    color: vec3<f32>,
    time: f32,
    camera_position: vec3<f32>,
}

// Depth snapshot (see this file's own top-of-file comment) - a plain
// Float{filterable: false} texture, not WGSL's dedicated texture_depth_2d/
// Depth sample type, mirroring depth_map.wgsl's own already-proven-working
// pattern for sampling this exact Depth32Float format. No sampler filtering
// happens (NonFiltering, textureSample with a nearest-only sampler) - depth
// values shouldn't be blended between texels anyway.
@group(0) @binding(0)
var t_scene_depth: texture_2d<f32>;
@group(0) @binding(1)
var s_scene_depth: sampler;

// See engine::rendering::enviroment::ocean::OceanUniform - each wave is two
// vec4s: (dir.x, dir.z, k, amplitude), (steepness, phase at camera,
// wavelength, unused).
struct Ocean {
    // geometry wave count, detail wave count, base height (added to every
    // height so troughs stay above Y=0), color height scale (a typical crest)
    params: vec4<f32>,
    // choppiness, whitecap strength, detail strength, wind direction (radians)
    shading: vec4<f32>,
    waves: array<vec4<f32>, 160>,
    // The low-flying plane's wake - see engine::rendering::enviroment::
    // ocean::OceanUniform. All zero when there's none.
    // plane: camera-relative x, z, height above the water, intensity 0..1
    wake_head: vec4<f32>,
    // circle around the whole wake (camera-relative x, z, radius), trail max age
    wake_bounds: vec4<f32>,
    // trail point count
    wake_info: vec4<f32>,
    // trail, newest first: camera-relative x, z, age, intensity
    wake_points: array<vec4<f32>, 32>,
    // camera world x, z wrapped to WAKE_NOISE_PERIOD, unused x2
    wake_anchor: vec4<f32>,
};

@group(0) @binding(2)
var<uniform> ocean: Ocean;

@group(1) @binding(0)
var<uniform> camera: CameraUniform;

@group(2) @binding(0)
var<uniform> transform: Transform;

@group(3) @binding(0)
var<uniform> light: Light;

// Converts a raw reversed-Z depth-buffer value (0=far, 1=near - see
// render_pass.rs's own "Reversed-Z" comment) into a linear view-space
// distance in world units - same formula depth_map.wgsl already uses (and
// visually confirms) for this engine's specific projection setup, just
// reused here instead of only for that debug overlay.
fn linearize_depth(raw: f32, near: f32, far: f32) -> f32 {
    let depth = 1.0 - raw;
    let z_ndc = depth * 2.0 - 1.0;
    return (2.0 * near * far) / (far + near - z_ndc * (far - near));
}

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

    @location(9) normal_matrix_0: vec3<f32>,
    @location(10) normal_matrix_1: vec3<f32>,
    @location(11) normal_matrix_2: vec3<f32>,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    // This point's camera-relative XZ before the waves displace it - fs_main
    // evaluates every wave's shading here, per pixel (see surface_waves).
    @location(1) rest_position: vec2<f32>,
    @location(2) world_position: vec3<f32>,
    @location(3) view_depth: f32,
};

// How much of a wave of `wavelength` a mesh of `cell`-sized cells moves -
// see GEOMETRY_MIN_CELLS/GEOMETRY_FULL_CELLS. Only shapes the surface; its
// look comes from surface_waves, per pixel.
fn geometry_share(wavelength: f32, cell: f32) -> f32 {
    return smoothstep(GEOMETRY_MIN_CELLS * cell, GEOMETRY_FULL_CELLS * cell, wavelength);
}

// The geometry waves at camera-relative point `p` (XZ), scaled by `amp`,
// each by its geometry_share for this `cell` size: x/z = sideways push,
// y = height; plus the surface normal and the crest pinch (see
// VertexOutput::crest). The
// standard Gerstner sum - see GPU Gems ch.1 - with each wave's phase taken
// from the camera (see this file's wave-field comment).
struct GerstnerSample {
    offset: vec3<f32>,
    normal: vec3<f32>,
    crest: f32,
};

fn gerstner(p: vec2<f32>, amp: f32, cell: f32) -> GerstnerSample {
    var offset = vec3<f32>(0.0);
    var slope = vec2<f32>(0.0);
    var crest = 0.0;
    let count = min(u32(ocean.params.x), MAX_WAVES);
    for (var i = 0u; i < count; i++) {
        let wave = ocean.waves[i * 2u];
        let extra = ocean.waves[i * 2u + 1u];
        let share = geometry_share(extra.z, cell);
        if (share <= 0.0) {
            continue;
        }
        let dir = wave.xy;
        let k = wave.z;
        let a = wave.w * amp * share;
        let steepness = extra.x;
        let theta = k * dot(dir, p) + extra.y;
        let s = sin(theta);
        let c = cos(theta);
        offset += vec3<f32>(steepness * a * dir.x * c, a * s, steepness * a * dir.y * c);
        slope += dir * (k * a * c);
        crest += steepness * k * a * s;
    }
    var out: GerstnerSample;
    out.offset = offset;
    out.normal = normalize(vec3<f32>(-slope.x, 1.0 - crest, -slope.y));
    out.crest = crest;
    return out;
}

// The geometry waves' look at camera-relative rest point `p`, per pixel -
// computed the same way everywhere, whether or not a ring is also moving
// them, so the water looks identical inside and outside the moving rings
// (the mesh only shapes the surface). Each wave fades out once it's too
// short for the pixel it's in, instead of shimmering.
struct SurfaceSample {
    // d height/dx, d height/dz
    slope: vec2<f32>,
    // Height, centered on 0 - drives the deep/light color ramp.
    height: f32,
    // How pinched this point is by the Gerstner crests, 0 (flat/trough) up
    // to ~choppiness at the sharpest peaks - whitecaps, and the normal.
    crest: f32,
    // The slope variance that faded out at this pixel - see detail_waves
    // for the total; their ratio drives FLAT_WATER_MIN_FACING.
    lost_variance: f32,
};

fn surface_waves(p: vec2<f32>, pixel_size: f32) -> SurfaceSample {
    var out: SurfaceSample;
    out.slope = vec2<f32>(0.0);
    out.height = 0.0;
    out.crest = 0.0;
    out.lost_variance = 0.0;
    let count = min(u32(ocean.params.x), MAX_WAVES);
    for (var i = 0u; i < count; i++) {
        let wave = ocean.waves[i * 2u];
        let extra = ocean.waves[i * 2u + 1u];
        let fade = smoothstep(DETAIL_FULL_PIXELS * 0.5, DETAIL_FULL_PIXELS, extra.z / pixel_size);
        let slope_amplitude = wave.z * wave.w;
        out.lost_variance += 0.5 * slope_amplitude * slope_amplitude * (1.0 - fade);
        if (fade <= 0.0) {
            continue;
        }
        let theta = wave.z * dot(wave.xy, p) + extra.y;
        let s = sin(theta);
        out.slope += wave.xy * (slope_amplitude * cos(theta) * fade);
        out.height += wave.w * s * fade;
        out.crest += extra.x * slope_amplitude * s * fade;
    }
    return out;
}

// Random 0..1 per integer lattice point, repeating every `period` points.
fn wake_hash(cell: vec2<i32>, period: i32) -> f32 {
    let wrapped = bitcast<vec2<u32>>(((cell % period) + period) % period);
    var h = (wrapped.x * 1597334677u) ^ (wrapped.y * 3812015801u);
    h = h * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    h = (h >> 22u) ^ h;
    return f32(h) * (1.0 / 4294967296.0);
}

// Smooth value noise 0..1 with blobs about `cell` meters across, at noise
// position `q` - repeats every WAKE_NOISE_PERIOD, so it lines up across the
// wrap in wake_anchor.
fn wake_value_noise(q: vec2<f32>, cell: f32) -> f32 {
    let period = i32(WAKE_NOISE_PERIOD / cell);
    let x = q / cell;
    let i = vec2<i32>(floor(x));
    let f = fract(x);
    let s = f * f * (3.0 - 2.0 * f);
    let a = wake_hash(i, period);
    let b = wake_hash(i + vec2<i32>(1, 0), period);
    let c = wake_hash(i + vec2<i32>(0, 1), period);
    let d = wake_hash(i + vec2<i32>(1, 1), period);
    return mix(mix(a, b, s.x), mix(c, d, s.x), s.y);
}

// Churned-foam pattern at noise position `q`: big clumps with finer
// breakup inside them. Roughly 0..1, around 0.5 on average.
fn wake_foam_noise(q: vec2<f32>) -> f32 {
    return wake_value_noise(q, 16.0) * 0.5 + wake_value_noise(q, 4.0) * 0.3 + wake_value_noise(q, 1.0) * 0.2;
}

// The low-flying plane's wake at camera-relative point `p` - see the
// WAKE_* constants. `foam` 0..1 whitens the water, `flatten` 0..1 calms its
// waves, `ripple_slope` adds the V of ripples spreading out behind it.
struct WakeSample {
    foam: f32,
    flatten: f32,
    ripple_slope: vec2<f32>,
};

fn water_wake(p: vec2<f32>) -> WakeSample {
    var out: WakeSample;
    out.foam = 0.0;
    out.flatten = 0.0;
    out.ripple_slope = vec2<f32>(0.0);
    let bounds = ocean.wake_bounds;
    if (bounds.z <= 0.0 || distance(p, bounds.xy) > bounds.z) {
        return out;
    }

    // Under and just behind the plane, in its own flight frame: `behind`
    // is how far back along the flight path, `across` how far to the side.
    let head = ocean.wake_head;
    let intensity = head.w;
    if (intensity > 0.0) {
        let forward = ocean.wake_info.yz;
        let side = vec2<f32>(-forward.y, forward.x);
        let to_point = p - head.xy;
        let behind = -dot(to_point, forward);
        let across = dot(to_point, side);

        // The blast: a small patch, stretched along the path and centered a
        // little behind the plane - flattened and whitened.
        let radius = WAKE_PATCH_RADIUS + max(head.z, 0.0) * WAKE_PATCH_GROWTH;
        let ellipse = length(vec2<f32>(across / radius, (behind - radius * 0.5) / (radius * WAKE_PATCH_STRETCH)));
        let blast = intensity * (1.0 - smoothstep(0.35, 1.0, ellipse));
        out.foam = blast * WAKE_PATCH_FOAM;
        out.flatten = blast;

        // The V: two arms of ripples spreading out behind it, each running
        // parallel to its arm and moving outward. The ripples are one height
        // field, sin(off_arm), and the slope is its true gradient - so the
        // crests the lighting sees line up with the arms on both sides.
        // `side_distance` is a rounded abs(across): a sharp abs (and the
        // sign() that goes with it) would flip the slope instantly on the
        // flight path, leaving a visible seam down the middle of the V.
        if (behind > 0.0) {
            let arm_width = 2.0 + behind * 0.15;
            let rounding = arm_width * 0.5;
            let side_distance = sqrt(across * across + rounding * rounding);
            let off_arm = side_distance - behind * WAKE_V_SLOPE;
            let on_arm = exp(-(off_arm * off_arm) / (arm_width * arm_width));
            // Fades in just behind the plane (no hard edge under it) and out
            // WAKE_V_LENGTH back.
            let fade = intensity * smoothstep(0.0, 4.0, behind) * (1.0 - smoothstep(0.0, WAKE_V_LENGTH, behind));
            let k = 6.2831853 / WAKE_RIPPLE_WAVELENGTH;
            let phase = (off_arm - light.time * WAKE_RIPPLE_SPEED) * k;
            // d(off_arm)/dp: outward across the path, plus backward-slope
            // along it (behind = -dot(to_point, forward)).
            let off_arm_gradient = side * (across / side_distance) + forward * WAKE_V_SLOPE;
            out.ripple_slope = off_arm_gradient * (cos(phase) * WAKE_RIPPLE_SLOPE * on_arm * fade);
        }
    }

    // Behind it: a foam streak along the trail, widening and fading with age.
    let max_age = bounds.w;
    let count = min(u32(ocean.wake_info.x), MAX_WAKE_POINTS);
    var trail = 0.0;
    // How old the foam is where `trail` came from, 0 (fresh) .. 1 (max age).
    var trail_age = 0.0;
    var previous = vec4<f32>(head.x, head.y, 0.0, intensity);
    for (var i = 0u; i < count; i++) {
        let point = ocean.wake_points[i];
        // Closest spot on the segment previous -> point.
        let segment = point.xy - previous.xy;
        let along = clamp(dot(p - previous.xy, segment) / max(dot(segment, segment), 0.0001), 0.0, 1.0);
        let distance_to_trail = length(p - (previous.xy + segment * along));
        let age = mix(previous.z, point.z, along);
        let strength = mix(previous.w, point.w, along) * (1.0 - smoothstep(0.0, max_age, age));
        let width = WAKE_TRAIL_WIDTH + age * WAKE_TRAIL_SPREAD;
        let here = strength * (1.0 - smoothstep(width * 0.3, width, distance_to_trail));
        if (here > trail) {
            trail = here;
            trail_age = clamp(age / max(max_age, 0.001), 0.0, 1.0);
        }
        previous = point;
    }
    // Broken up by noise pinned to the water (not the camera - the trail
    // stays put as the plane flies on), so it reads as churned foam, not a
    // painted stripe: nearly solid when fresh, clumps and holes as it ages.
    if (trail > 0.0) {
        let noise = wake_foam_noise(ocean.wake_anchor.xy + p);
        let coverage = mix(WAKE_FOAM_COVERAGE_FRESH, WAKE_FOAM_COVERAGE_OLD, trail_age);
        // Noise above (1 - coverage) is foam, with a soft edge.
        let threshold = 1.0 - coverage;
        let breakup = smoothstep(threshold - 0.12, threshold + 0.12, noise);
        out.foam = max(out.foam, trail * WAKE_TRAIL_FOAM * breakup);
    }
    return out;
}

// The detail waves at camera-relative point `p`, scaled by `strength` - each
// faded by how many screen pixels its wavelength spans here (`pixel_size` =
// world units per pixel), so ones too short to show cleanly drop out
// instead of shimmering. Returns xy = slope, z = the slope variance that
// faded out, w = the total slope variance of ALL waves (geometry + detail) -
// see surface_waves.
fn detail_waves(p: vec2<f32>, pixel_size: f32, strength: f32) -> vec4<f32> {
    var result = vec4<f32>(0.0);
    let geometry_count = min(u32(ocean.params.x), MAX_WAVES);
    for (var i = 0u; i < geometry_count; i++) {
        let slope_amplitude = ocean.waves[i * 2u].z * ocean.waves[i * 2u].w;
        result.w += 0.5 * slope_amplitude * slope_amplitude;
    }
    let last = min(geometry_count + u32(ocean.params.y), MAX_WAVES);
    for (var i = geometry_count; i < last; i++) {
        let wave = ocean.waves[i * 2u];
        let extra = ocean.waves[i * 2u + 1u];
        let slope_amplitude = wave.z * wave.w * strength;
        let variance = 0.5 * slope_amplitude * slope_amplitude;
        let fade = smoothstep(DETAIL_FULL_PIXELS * 0.5, DETAIL_FULL_PIXELS, extra.z / pixel_size);
        result.w += variance;
        result.z += variance * (1.0 - fade);
        if (fade <= 0.0) {
            continue;
        }
        let theta = wave.z * dot(wave.xy, p) + extra.y;
        result.x += wave.x * slope_amplitude * cos(theta) * fade;
        result.y += wave.y * slope_amplitude * cos(theta) * fade;
    }
    return result;
}

@vertex
fn vs_main(model: VertexInput, instance: InstanceInput) -> VertexOutput {
    let model_matrix = mat4x4<f32>(
        instance.model_matrix_0,
        instance.model_matrix_1,
        instance.model_matrix_2,
        instance.model_matrix_3,
    );

    // Ring morph (see RING_MORPH_START): as this vertex nears its ring's
    // outer circle, odd grid vertices slide onto the next ring's coarser
    // grid (every other vertex), so the ring's edge matches the next ring's
    // exactly. `cell`/`outer` are this ring's cell size and half-extent, and
    // `next_cell` the next ring's (see resources::build_water_rings_mesh) -
    // a whole multiple of `cell`; positions are exact multiples of `cell`, so
    // `snap` is exactly how far each vertex is past the next ring's grid.
    let cell = model.tex_coords.x;
    let outer = model.tex_coords.y;
    let next_cell = model.normal.x;
    let local = model.position.xz;
    let ring_position = length(local) / outer;
    let morph = smoothstep(RING_MORPH_START, 1.0, ring_position);
    let snap = fract(local / next_cell) * next_cell;
    let morphed = local - snap * morph;
    // The cell size this vertex effectively samples at - grows smoothly to
    // the next ring's by the ring's edge (see NEXT_RING_FADE_START), so the
    // waves each ring moves (see geometry_share) also match there.
    let next_ring_fade = smoothstep(NEXT_RING_FADE_START, 1.0, ring_position);
    let effective_cell = mix(cell * (1.0 + morph), next_cell, next_ring_fade);

    var world_position: vec4<f32> = model_matrix * transform.model_matrix * vec4<f32>(morphed.x, model.position.y, morphed.y, 1.0);

    // world_position.xz here is camera-relative (see GameObject::to_raw) -
    // exactly what the wave field wants (see its comment at the top of this
    // file: each wave's phase at the camera comes precomputed), and what
    // camera.view_proj / view_depth below need for GPU float precision too.
    // This instance's own Transform3D.scale.y, recovered from its model
    // matrix's Y-basis column - scales wave height, 1.0 normally.
    let y_scale = length(instance.model_matrix_1.xyz);
    let wave = gerstner(world_position.xz, y_scale, effective_cell);
    let rest_position = world_position.xz;

    // Lifted by the base height (ocean.params.z, ~3 standard deviations of
    // the wave height) so troughs stay above Y=0 - the same offset
    // OceanWaves::height_at adds, so gameplay heights line up.
    world_position.x += wave.offset.x;
    world_position.z += wave.offset.z;
    world_position.y += wave.offset.y + ocean.params.z;

    var out: VertexOutput;
    out.rest_position = rest_position;
    out.world_position = world_position.xyz;
    out.clip_position = camera.view_proj * world_position;
    out.view_depth = length(camera.view_pos.xyz - out.world_position);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // How much sea one screen pixel covers here (the longer side of its
    // footprint, so nothing shimmers along the view direction) - taken first,
    // while every pixel is still on the same path: screen-space derivatives
    // aren't allowed inside branches that differ per pixel.
    let footprint_x = length(dpdx(in.world_position.xz));
    let footprint_y = length(dpdy(in.world_position.xz));
    let pixel_size = max(max(footprint_x, footprint_y), 0.0001);

    // Lower than before, and diffuse no longer gets flattened against a
    // single ambient-dominated base - the point is for the normal-driven
    // diffuse term (which already varies per-pixel with wave slope) to read
    // as real shading contrast (bright wave faces toward the light, darker
    // troughs/shadowed faces) instead of a near-uniform wash.
    let ambient_strength = 0.35;
    let ambient_color = light.color * ambient_strength;

    // The sun is one fixed direction, infinitely far away (App::run places
    // light.position 1e9 units along the "sun" node's direction, camera-
    // relative) - so light_dir is the same everywhere on the water, and the
    // sun glint lines up with the sky's sun disc wherever the camera is.
    // So this plain dot-product diffuse term only varies as much
    // as the surface normal actually tilts - the Gerstner crests and the
    // per-pixel detail waves (below) are what give it real contrast; an
    // earlier pass contrast-stretching this curve (smoothstep, steep pow())
    // was removed - it can't create variation that isn't in the normal.
    // Near water vs far open sea - see OPEN_SEA_COLOR.
    let wave_visibility = 1.0 - smoothstep(OPEN_SEA_BLEND_START, OPEN_SEA_BLEND_END, in.view_depth);

    // Per-pixel normal, from every wave at this point's rest position - the
    // geometry waves (surface_waves, with their Gerstner crest pinch) and the
    // detail waves too short for any mesh (detail_waves) - each faded out
    // once it's too small for the pixel. The same everywhere, so nothing
    // changes where the moving rings end.
    let surface = surface_waves(in.rest_position, pixel_size);
    let detail = detail_waves(in.rest_position, pixel_size, ocean.shading.z);
    // The plane's wake, if any: calms the waves under it and adds its ripples.
    let wake = water_wake(in.rest_position);
    let calm = 1.0 - wake.flatten * WAKE_FLATTEN;
    let slope = (surface.slope + detail.xy) * calm + wake.ripple_slope;
    let normal = normalize(vec3<f32>(-slope.x, 1.0 - surface.crest * calm, -slope.y));
    // How much of the waves' total slope has faded out at this pixel, 0..1 -
    // see FLAT_WATER_MIN_FACING.
    let lost_slope = clamp((surface.lost_variance + detail.z) / max(detail.w, 1e-8), 0.0, 1.0);

    let light_dir = normalize(light.position - in.world_position);
    let diffuse_strength = max(dot(normal, light_dir), 0.0);
    let diffuse_color = light.color * diffuse_strength;

    // Sun-glint - one of the strongest "this is actually water" cues (a
    // real reflective liquid surface, not a painted texture), so this is
    // deliberately punchier than a typical specular highlight: a lower
    // exponent (broader, easier-to-catch glint) combined with a stronger
    // multiplier below (specular_color * 1.4, not the usual subtle * 0.2-0.5)
    // rather than the tight speck a higher exponent alone would give.
    let view_dir = normalize(camera.view_pos.xyz - in.world_position);
    let reflect_dir = reflect(-light_dir, normal);
    let specular_strength = pow(max(dot(view_dir, reflect_dir), 0.0), 48.0);
    let specular_color = specular_strength * light.color;

    // A direct, always-visible blend across the wave's ENTIRE current height
    // range - DEEP_COLOR at the very lowest point (height_01 = 0) up to
    // WATER_COLOR at the very highest (height_01 = 1), darker at low spots
    // and lighter at high ones everywhere, not just a rare accent gated to
    // extreme troughs (an earlier pass at this only blended toward
    // DEEP_COLOR near the deepest few percent of the range, via a pow()
    // curve - correct in theory, but it read as barely-there). No
    // height-based whitening here - whitecaps come from how pinched a crest
    // is (surface.crest, below), shore foam from the depth snapshot.
    // Per pixel too (see surface_waves), so the color ramp is the same inside
    // and outside the moving rings.
    let height_01 = clamp(surface.height / ocean.params.w, -1.0, 1.0) * 0.5 + 0.5;
    // Wave colors where waves are rendered, easing into the open-sea color
    // as they fade out - see OPEN_SEA_COLOR.
    let base_color = mix(OPEN_SEA_COLOR, mix(DEEP_COLOR, WATER_COLOR, height_01), wave_visibility);

    // Fresnel - blends toward SKY_REFLECTION_COLOR the more edge-on the
    // surface is being viewed (view_dir already computed above, for
    // specular) - see that constant's own doc comment for why this, more
    // than the exact base tint, is what makes an opaque water shader read
    // as real rather than a flat-colored plane. 1.0 - dot(...) is 0 looking
    // straight down the normal, 1 at a true grazing angle.
    // As wave detail fades out at this pixel, the surface stops counting as
    // facing away by more than FLAT_WATER_MIN_FACING allows - see it.
    let facing = max(dot(view_dir, normal), lost_slope * FLAT_WATER_MIN_FACING);
    let fresnel_amount = pow(1.0 - clamp(facing, 0.0, 1.0), FRESNEL_POWER) * FRESNEL_STRENGTH;

    // Shore-intersection foam - samples t_scene_depth (this same screen
    // position's depth, snapshotted right after opaque geometry finished
    // drawing - see this file's own top comment) and compares it against
    // this fragment's own depth, both linearized into comparable world-unit
    // distances (see linearize_depth above) - close to 0 separation means
    // the water surface is right at/inside something solid (a shoreline, a
    // rock, a hull), which is exactly where real foam actually forms.
    // textureLoad (an exact texel fetch, not textureSample) since
    // in.clip_position.xy is already the precise framebuffer pixel this
    // fragment is at - no interpolation wanted for a depth lookup anyway.
    let scene_depth_raw = textureLoad(t_scene_depth, vec2<i32>(in.clip_position.xy), 0).x;
    let scene_z = linearize_depth(scene_depth_raw, NEAR, FAR);
    let water_z = linearize_depth(in.clip_position.z, NEAR, FAR);
    // How far behind the water surface the ground is along the view ray,
    // turned into water depth straight down: the ray drops view_dir.y per
    // unit it travels. (Along the ray alone, a steep close-up view reaches
    // the ground sooner than a grazing far one over the same depth - which
    // made the foam band grow as the camera approached.)
    let shore_distance = max(scene_z - water_z, 0.0) * max(view_dir.y, 0.02);
    // Near water only - see wave_visibility.
    let foam_wave_visibility = wave_visibility;
    let shore_foam = (1.0 - smoothstep(0.0, SHORE_FOAM_DEPTH, shore_distance)) * foam_wave_visibility;
    let shore_tinted = mix(base_color, FOAM_COLOR, shore_foam);

    // Whitecaps on the sharpest crests - how pinched this point is relative
    // to the most any crest can be (the choppiness), over the same
    // visibility envelope as everything else wave-related.
    let crest_01 = surface.crest / max(ocean.shading.x, 0.0001);
    let whitecap = smoothstep(WHITECAP_START, WHITECAP_FULL, crest_01) * ocean.shading.y * wave_visibility;
    let capped = mix(mix(shore_tinted, WHITECAP_COLOR, whitecap), WAKE_FOAM_COLOR, wake.foam);

    // Final composition: light the water (and its foam) first, THEN blend in
    // the sky reflection - unlit, since reflected sky is already light. (It
    // used to be blended in before lighting, which multiplied the pale sky
    // color by ~1.35 and clipped it toward grey-white at every diagonal
    // view.) Foam isn't a mirror, so it takes no reflection; the sun glint
    // goes on top of everything.
    let lit_water = (ambient_color + diffuse_color) * capped;
    let reflection = fresnel_amount * (1.0 - max(max(shore_foam, whitecap), wake.foam));
    let result = mix(lit_water, SKY_REFLECTION_COLOR, reflection) + specular_color * 1.4;

    // Long-distance atmospheric haze - fades the water toward the sea's own
    // horizon blue (SEA_HORIZON_COLOR), which sits just below the sky's
    // paler haze band - a soft but clear horizon line, like the photos.
    //
    // The ocean mesh reaches ~2,100 km and re-centres on the camera every
    // frame (GameLogic::update_water_plane), so there's room for a genuinely
    // gradual fade: fully blue for tens of km, then hazing over hundreds
    // more, reaching fog_color well inside the mesh edge so it blends into
    // the sky with no visible boundary.
    //
    // Driven by HORIZONTAL distance (in.world_position.xz is camera-relative,
    // so its length is distance-from-camera on the ground plane), not
    // in.view_depth - otherwise climbing makes the depth to the water
    // directly below large enough to fog the ocean out from under the plane.
    //
    // fog_start / fog_end are kept matched to depth.wgsl's model fog. Its
    // color isn't: the sea fogs to its own horizon blue, while objects (seen
    // against the sky) fog to the sky's haze - see depth.wgsl.
    let horizon_dist = length(in.world_position.xz);
    // Aerial perspective: the sea lightens toward its horizon blue with
    // distance (see SEA_HORIZON_COLOR)...
    let hazed = mix(result, SEA_HORIZON_COLOR, smoothstep(SEA_HAZE_START, SEA_HAZE_END, horizon_dist));
    // ...and the far fog settles fully onto it.
    let fog_start = 80000.0;
    let fog_end = 1400000.0;
    let fog_factor = smoothstep(fog_start, fog_end, horizon_dist);
    let fogged_color = mix(hazed, SEA_HORIZON_COLOR, fog_factor);

    return vec4<f32>(fogged_color, 1.0);
}
