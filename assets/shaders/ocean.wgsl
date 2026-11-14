// The ocean - the game's Raw shader for the sea (settings in ocean.ron; see
// engine::rendering::shader_library for what a Raw shader gets). Groups
// 1/2/3 are the camera, the mesh's transform and the light, as for every
// model. Group 0 is the material's: its params - the wave field, `Ocean`
// below, filled every frame by prefabs::ocean - at binding 2, and
// (it `reads: [Depth]`) a snapshot of the depth buffer taken right after the
// solid world finished drawing (the frame plan's CopyDepth) at 16/17 - used
// for shore-intersection foam, see fs_main's own comment on that. Bindings
// 0/1 are the mesh's own texture, which water never samples - it always
// outputs a computed color instead (see WATER_COLOR/DEEP_COLOR/
// SKY_REFLECTION_COLOR below).

// The camera's near/far planes and lens come from CameraUniform (the
// engine fills them from the active camera every frame) - a perspective or
// an orthographic lens (CameraLens::orthographic: a top-down map, say), and
// see linearize_depth / view_dir / view_depth for what changes with it.

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
// prefabs::ocean::mesh): coarser the
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
// the CPU from a wind-sea spectrum by prefabs::ocean
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

// Must match prefabs::ocean::waves::MAX_WAVES.
const MAX_WAVES: u32 = 80u;
// The noise pinned to the sea (the depth tint's patches, the ripples'
// gusts) repeats every NOISE_PERIOD meters - prefabs::ocean::shader's
// NOISE_PERIOD, keep in sync.
const NOISE_PERIOD: f32 = 4096.0;
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
    near: f32,
    far: f32,
    // 1: an orthographic lens, 0: perspective.
    orthographic: f32,
    _padding: f32,
    // Which way the camera looks (xyz, unit length).
    forward: vec4<f32>,
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
@group(0) @binding(16)
var t_scene_depth: texture_2d<f32>;
@group(0) @binding(17)
var s_scene_depth: sampler;

// The ripple map (ocean.ron's `ripples` - see prefabs::ocean::ripples): the
// sea's shortest waves baked into one tiling texture of slopes - red along
// the map's u, green along v, 0.5 flat (ripple_info.z turns it back into a
// slope). Read by ripple_layer when the detail waves come from it
// (tuning.w), instead of summing them one by one.
@group(0) @binding(3)
var ripples: texture_2d<f32>;
@group(0) @binding(4)
var ripples_sampler: sampler;
// The ripples on the map, shortest and longest, in map tiles -
// prefabs::ocean::ripples' SHORTEST and LONGEST over its TILE (keep in sync).
const RIPPLE_SHORTEST: f32 = 0.015625;
const RIPPLE_LONGEST: f32 = 0.25;
// The ripple map isn't laid down tile after tile (that repeats plainly
// from a few hundred meters up) but stochastically: over a triangle grid,
// each grid point showing the map from a random place, every point on the
// sea blending the three around it (see ripple_layer). RIPPLE_HEX_SCALE
// grid points per tile, the random places repeating every
// RIPPLE_HEX_PERIOD grid points - prefabs::ocean::ripples' HEX_SCALE and
// HEX_PERIOD (keep in sync).
const RIPPLE_HEX_SCALE: f32 = 1.5;
const RIPPLE_HEX_PERIOD: i32 = 256;
// The ripples' strength drifts across the sea - rougher and calmer patches,
// like gusts on real water - between these, over blobs about
// RIPPLE_GUST_SIZE meters across (and a finer layer a quarter that). It also
// keeps any leftover pattern from lining up.
const RIPPLE_GUST_MIN: f32 = 0.6;
const RIPPLE_GUST_MAX: f32 = 1.4;
const RIPPLE_GUST_SIZE: f32 = 512.0;

// See prefabs::ocean (OceanWaves::params) - each wave is two
// vec4s: (dir.x, dir.z, k, amplitude), (steepness, phase at camera,
// wavelength, unused).
struct Ocean {
    // geometry wave count, detail wave count, base height (added to every
    // height so troughs stay above Y=0), color height scale (a typical crest)
    params: vec4<f32>,
    // choppiness, whitecap strength, detail strength, the total slope
    // variance of every wave (detail ones scaled by the detail strength) -
    // see surface_waves
    shading: vec4<f32>,
    waves: array<vec4<f32>, 160>,
    // camera world x, z wrapped to NOISE_PERIOD, unused x2
    noise_anchor: vec4<f32>,
    // Per-pixel cost knobs (see OceanSettings::pixel_*): how many pixels a
    // wave must span to fully show (DETAIL_FULL_PIXELS when 0), at most how
    // many waves a pixel sums, unused, whether the detail waves come from
    // the ripple map (1) or are summed (0).
    tuning: vec4<f32>,
    // The ripple map's two layers (see ripple_layer): where the camera is
    // on each (u, v, wrapped to one tile), and which way its u points (the
    // axis's x, z).
    ripple_a: vec4<f32>,
    ripple_b: vec4<f32>,
    // 1 / each layer's tile size (m), what turns the map's 0..1 back into
    // a slope (times a layer's weight and the detail strength), the total
    // slope variance with the map standing in for the detail waves.
    ripple_info: vec4<f32>,
    // Where a fully averaged mipmap of the map sits off 0.5 (u, v - see
    // RippleMap::mip_bias), one layer's slope variance at full detail,
    // unused.
    ripple_bias: vec4<f32>,
    // Where the camera is on each layer's stochastic grid (see
    // RIPPLE_HEX_SCALE): layer a's x, y, layer b's - wrapped to
    // RIPPLE_HEX_PERIOD.
    ripple_cells: vec4<f32>,
    // Detail by the camera's altitude (see OceanSettings::altitude_detail):
    // the shortest wave still drawn from up there (m) - shorter ones are
    // gone, ones twice as long fully there - unused x3.
    detail: vec4<f32>,
    // The far layer of the ripple map (OceanSettings::far_variation, its
    // strength in detail.w): like ripple_a, and its grid (x, y), 1 / its
    // tile size, unused.
    ripple_far: vec4<f32>,
    ripple_far_cells: vec4<f32>,
    // Where the far look takes over (from, to - m from the camera; the near
    // ripples fade out and the far layer in between), the depth tint's
    // strength (0..1 - see depth_tint), unused.
    look: vec4<f32>,
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
//
// Written as near*far / (near + raw*(far - near)) - the same thing as
// 2nf / (f + n - z_ndc*(f - n)) with z_ndc = 1 - 2*raw, but without that
// form's (f + n) - (f - n): with f = 4e6 and n = 0.1 those two round to the
// same f32, so a raw 0 (the far plane - the sky) divided by exactly zero. On Metal (fast
// math) that came out as nonsense, and the shore foam covered the sea.
// An orthographic lens spreads depth evenly from near to far instead.
fn linearize_depth(raw: f32, near: f32, far: f32) -> f32 {
    if (camera.orthographic > 0.5) {
        return near + (1.0 - raw) * (far - near);
    }
    return (near * far) / (near + raw * (far - near));
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
};

// How much of a wave of `wavelength` a mesh of `cell`-sized cells moves -
// see GEOMETRY_MIN_CELLS/GEOMETRY_FULL_CELLS. Only shapes the surface; its
// look comes from surface_waves, per pixel.
fn geometry_share(wavelength: f32, cell: f32) -> f32 {
    return smoothstep(GEOMETRY_MIN_CELLS * cell, GEOMETRY_FULL_CELLS * cell, wavelength);
}

// The geometry waves' displacement at camera-relative point `p` (XZ),
// scaled by `amp`, each by its geometry_share for this `cell` size: x/z =
// sideways push, y = height. The standard Gerstner sum - see GPU Gems ch.1 -
// with each wave's phase taken from the camera (see this file's wave-field
// comment). The normal and crest pinch aren't needed here - fs_main works
// them out per pixel (see surface_waves).
//
// Walks the waves longest first: they arrive sorted shortest to longest
// (see prefabs::ocean::waves' OceanWaves::new), so once one is too short for this
// cell size every one after it is too - and a coarse ring stops right away.
fn gerstner(p: vec2<f32>, amp: f32, cell: f32) -> vec3<f32> {
    var offset = vec3<f32>(0.0);
    let count = min(u32(ocean.params.x), MAX_WAVES);
    for (var j = count; j > 0u; j--) {
        let i = j - 1u;
        let wave = ocean.waves[i * 2u];
        let extra = ocean.waves[i * 2u + 1u];
        let share = geometry_share(extra.z, cell);
        if (share <= 0.0) {
            break;
        }
        let dir = wave.xy;
        let a = wave.w * amp * share;
        let steepness = extra.x;
        let theta = wave.z * dot(dir, p) + extra.y;
        let c = cos(theta);
        offset += vec3<f32>(steepness * a * dir.x * c, a * sin(theta), steepness * a * dir.y * c);
    }
    return offset;
}

// Every wave's look at camera-relative rest point `p`, per pixel - computed
// the same way everywhere, whether or not a ring is also moving them, so the
// water looks identical inside and outside the moving rings (the mesh only
// shapes the surface). Two groups (see this file's wave-field comment): the
// geometry waves give the slope, height and crest pinch; the detail waves,
// too short for any mesh, only add slope (ripples, glints - scaled by the
// detail strength, ocean.shading.z). Each wave fades out once it's too short
// for the pixel it's in (DETAIL_FULL_PIXELS), instead of shimmering.
struct SurfaceSample {
    // d height/dx, d height/dz - every wave's, geometry and detail.
    slope: vec2<f32>,
    // Height, centered on 0 - drives the deep/light color ramp.
    height: f32,
    // How pinched this point is by the Gerstner crests, 0 (flat/trough) up
    // to ~choppiness at the sharpest peaks - whitecaps, and the normal.
    crest: f32,
    // The slope variance of the waves it summed (faded ones partly) - what
    // fs_main compares with the total for `lost_slope`.
    shown_variance: f32,
};

fn surface_waves(p: vec2<f32>, pixel_size: f32) -> SurfaceSample {
    var out: SurfaceSample;
    out.slope = vec2<f32>(0.0);
    out.height = 0.0;
    out.crest = 0.0;
    // The slope variance of the waves still showing - what faded out is the
    // rest of the total (ocean.shading.w, summed once on the CPU).
    var shown_variance = 0.0;

    // A wave fully shows once its wavelength spans `full_pixels` pixels and
    // is gone by half that - and, from high up, once it's twice the
    // altitude's shortest wave (ocean.detail.x), gone at it: smoothstep(0.5,
    // 1, wavelength * per_wavelength). Worked out once here, so each wave's
    // test is a multiply, not a divide.
    let full_pixels = select(DETAIL_FULL_PIXELS, ocean.tuning.x, ocean.tuning.x > 0.0);
    let per_wavelength = 1.0 / max(pixel_size * full_pixels, 2.0 * ocean.detail.x);
    // At most this many waves per pixel (the longest - the rest count as
    // faded out, so the fresnel still treats them as there: lost_slope).
    let budget = u32(max(ocean.tuning.y, 0.0));
    var summed = 0u;

    // Each group arrives sorted shortest to longest (see gerstner), so both
    // loops walk them longest first and stop at the first one too short for
    // this pixel - every one after it is shorter still. Far water, where
    // pixels cover meters, stops after a few waves or none.
    let geometry_count = min(u32(ocean.params.x), MAX_WAVES);
    for (var j = geometry_count; j > 0u; j--) {
        if (summed >= budget) {
            break;
        }
        let i = j - 1u;
        let extra = ocean.waves[i * 2u + 1u];
        let fade = smoothstep(0.5, 1.0, extra.z * per_wavelength);
        if (fade <= 0.0) {
            break;
        }
        summed += 1u;
        let wave = ocean.waves[i * 2u];
        let slope_amplitude = wave.z * wave.w;
        shown_variance += 0.5 * slope_amplitude * slope_amplitude * fade;
        let theta = wave.z * dot(wave.xy, p) + extra.y;
        let s = sin(theta);
        out.slope += wave.xy * (slope_amplitude * cos(theta) * fade);
        out.height += wave.w * s * fade;
        out.crest += extra.x * slope_amplitude * s * fade;
    }

    // Every detail wave is shorter than every geometry wave, so this only
    // gets past its first one where all of those showed. None when the
    // ripple map stands in for them (see ripple_layer).
    let strength = ocean.shading.z;
    let detail_count = select(u32(ocean.params.y), 0u, ocean.tuning.w > 0.5);
    let last = min(geometry_count + detail_count, MAX_WAVES);
    for (var j = last; j > geometry_count; j--) {
        if (summed >= budget) {
            break;
        }
        let i = j - 1u;
        let extra = ocean.waves[i * 2u + 1u];
        let fade = smoothstep(0.5, 1.0, extra.z * per_wavelength);
        if (fade <= 0.0) {
            break;
        }
        summed += 1u;
        let wave = ocean.waves[i * 2u];
        let slope_amplitude = wave.z * wave.w * strength;
        shown_variance += 0.5 * slope_amplitude * slope_amplitude * fade;
        let theta = wave.z * dot(wave.xy, p) + extra.y;
        out.slope += wave.xy * (slope_amplitude * cos(theta) * fade);
    }

    out.shown_variance = shown_variance;
    return out;
}

// A random place on the ripple map (0..1 tiles) for stochastic grid point
// `point` (whole numbers).
fn ripple_offset(point: vec2<f32>) -> vec2<f32> {
    let id = vec2<i32>(point);
    return vec2<f32>(sea_hash(id, RIPPLE_HEX_PERIOD), sea_hash(id + vec2<i32>(101, 57), RIPPLE_HEX_PERIOD));
}

// One layer of the ripple map at camera-relative rest point `p`: its slope
// (world XZ) and how much of its slope variance still shows at this
// pixel's size (0..1). `layer` is ocean.ripple_a or _b, `cells` where the
// camera is on its stochastic grid, `inv_tile` 1 / its tile size. Only from
// uniform control flow (it takes derivatives).
fn ripple_layer(p: vec2<f32>, layer: vec4<f32>, cells: vec2<f32>, inv_tile: f32, pixel_size: f32, stochastic: bool) -> vec3<f32> {
    let axis = layer.zw;
    let side = vec2<f32>(-axis.y, axis.x);
    // In the layer's own frame, in tiles.
    let local = vec2<f32>(dot(p, axis), dot(p, side)) * inv_tile;
    let uv = layer.xy + local;
    // The mip level from the smooth coordinates - the random jumps between
    // grid points would otherwise pick a tiny mip along every seam.
    let uv_dx = dpdx(uv);
    let uv_dy = dpdy(uv);

    // The mipmaps average away the ripples shorter than ~2 pixels, and every
    // octave of ripple carries about the same slope - so what's left is the
    // share of the band's octaves still longer than that.
    let shortest = RIPPLE_SHORTEST / inv_tile;
    let longest = RIPPLE_LONGEST / inv_tile;
    let octaves = log(longest / shortest);
    let kept = clamp(log(longest / max(shortest, 2.0 * pixel_size)) / octaves, 0.0, 1.0);
    // And from high up, only what's longer than the altitude's shortest
    // wave (ocean.detail.x) - the map can't drop its short ripples alone,
    // so the whole layer is turned down to what that leaves.
    let kept_here = clamp(log(longest / max(max(shortest, 2.0 * pixel_size), ocean.detail.x)) / octaves, 0.0, 1.0);
    let altitude_fade = sqrt(kept_here / max(kept, 1e-4));
    // Flat is 0.5 up close, and a little under it where the mipmaps (made
    // in sRGB) take over.
    let flat = 0.5 + ocean.ripple_bias.xy * (1.0 - kept);

    if (!stochastic) {
        // Plainly tiled: one read.
        let texel = textureSampleGrad(ripples, ripples_sampler, uv, uv_dx, uv_dy).rg - flat;
        let slope = texel * ocean.ripple_info.z * altitude_fade;
        return vec3<f32>(axis * slope.x + side * slope.y, kept_here);
    }

    // Stochastic tiling (Heitz & Neyret's triangle grid): which grid
    // triangle this point is in, and how close to each of its corners.
    let scaled = local * RIPPLE_HEX_SCALE;
    let skewed = cells + vec2<f32>(scaled.x - 0.57735027 * scaled.y, 1.15470054 * scaled.y);
    let base = floor(skewed);
    let f = fract(skewed);
    let z = 1.0 - f.x - f.y;
    var weights: vec3<f32>;
    var corner_a: vec2<f32>;
    var corner_b: vec2<f32>;
    var corner_c: vec2<f32>;
    if (z > 0.0) {
        weights = vec3<f32>(z, f.y, f.x);
        corner_a = base;
        corner_b = base + vec2<f32>(0.0, 1.0);
        corner_c = base + vec2<f32>(1.0, 0.0);
    } else {
        weights = vec3<f32>(-z, 1.0 - f.y, 1.0 - f.x);
        corner_a = base + vec2<f32>(1.0, 1.0);
        corner_b = base + vec2<f32>(1.0, 0.0);
        corner_c = base + vec2<f32>(0.0, 1.0);
    }
    // Sharpened a little, so each spot is mostly one corner's piece of the
    // map (less ghosting), still blending smoothly between them.
    weights = weights * weights * weights;
    weights = weights / (weights.x + weights.y + weights.z);

    let a = textureSampleGrad(ripples, ripples_sampler, uv + ripple_offset(corner_a), uv_dx, uv_dy).rg - flat;
    let b = textureSampleGrad(ripples, ripples_sampler, uv + ripple_offset(corner_b), uv_dx, uv_dy).rg - flat;
    let c = textureSampleGrad(ripples, ripples_sampler, uv + ripple_offset(corner_c), uv_dx, uv_dy).rg - flat;
    // Blending unrelated slopes flattens them where they mix - divided back
    // up by how much (the blend's length), so the sea is as rough between
    // grid points as on them.
    let blended = (a * weights.x + b * weights.y + c * weights.z) / max(length(weights), 1e-4);
    let slope = blended * ocean.ripple_info.z * altitude_fade;
    return vec3<f32>(axis * slope.x + side * slope.y, kept_here);
}

// The depth so far at framebuffer pixel `pixel` (the frame plan's
// CopyDepth) - the only place this shader reads it. Raw reversed-Z: 0 is the
// far plane.
fn scene_depth(pixel: vec2<i32>) -> f32 {
    return textureLoad(t_scene_depth, pixel, 0).x;
}

// Broad patches of deeper and shallower-looking water at camera-relative
// rest point `p`, -1..1 - noise pinned to the sea (on its wrap) at ~512, 128 and 32 m - broad, soft patches - the finer ones
// fading out once they're too fine for the pixel (`pixel_size`, m) so they
// never shimmer. Shades the water's color like the swell's height does
// (fs_main's height_01).
fn depth_tint(p: vec2<f32>, pixel_size: f32) -> f32 {
    let q = ocean.noise_anchor.xy + p;
    var tint = (sea_value_noise(q, 512.0) - 0.5) * 0.6;
    tint += (sea_value_noise(q, 128.0) - 0.5) * 0.3 * (1.0 - smoothstep(16.0, 64.0, pixel_size));
    tint += (sea_value_noise(q, 32.0) - 0.5) * 0.1 * (1.0 - smoothstep(4.0, 16.0, pixel_size));
    return tint * 2.0;
}

// Random 0..1 per integer lattice point, repeating every `period` points.
fn sea_hash(cell: vec2<i32>, period: i32) -> f32 {
    let wrapped = bitcast<vec2<u32>>(((cell % period) + period) % period);
    var h = (wrapped.x * 1597334677u) ^ (wrapped.y * 3812015801u);
    h = h * 747796405u + 2891336453u;
    h = ((h >> ((h >> 28u) + 4u)) ^ h) * 277803737u;
    h = (h >> 22u) ^ h;
    return f32(h) * (1.0 / 4294967296.0);
}

// Smooth value noise 0..1 with blobs about `cell` meters across, at noise
// position `q` - repeats every NOISE_PERIOD, so it lines up across the
// wrap in noise_anchor.
fn sea_value_noise(q: vec2<f32>, cell: f32) -> f32 {
    let period = i32(NOISE_PERIOD / cell);
    let x = q / cell;
    let i = vec2<i32>(floor(x));
    let f = fract(x);
    let s = f * f * (3.0 - 2.0 * f);
    let a = sea_hash(i, period);
    let b = sea_hash(i + vec2<i32>(1, 0), period);
    let c = sea_hash(i + vec2<i32>(0, 1), period);
    let d = sea_hash(i + vec2<i32>(1, 1), period);
    return mix(mix(a, b, s.x), mix(c, d, s.x), s.y);
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
    // `next_cell` the next ring's (see prefabs::ocean::mesh) -
    // a whole multiple of `cell`; positions are exact multiples of `cell`, so
    // `snap` is exactly how far each vertex is past the next ring's grid.
    let cell = model.tex_coords.x;
    let outer = model.tex_coords.y;
    let next_cell = model.normal.x;
    let local = model.position.xz;
    let ring_position = length(local) / outer;
    // The next-grid point it slides onto (the one below it on both axes).
    let anchor = floor(local / next_cell) * next_cell;
    let snap = local - anchor;
    // How far it slides is worked out from that point, not from the vertex
    // itself - so every vertex sliding onto the same point slides by the
    // same fraction, and neighbors never pass each other: no triangle folds
    // over, however much coarser the next ring is. (A ring's inner corners
    // must stay unmorphed even so: its half-extent must be at least ~2x the
    // next ring's cell - see prefabs::ocean::mesh's RINGS.)
    let morph = smoothstep(RING_MORPH_START, 1.0, length(anchor) / outer);
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
    let offset = gerstner(world_position.xz, y_scale, effective_cell);
    let rest_position = world_position.xz;

    // Lifted by the base height (ocean.params.z, ~3 standard deviations of
    // the wave height) so troughs stay above Y=0 - the same offset
    // OceanWaves::height_at adds, so gameplay heights line up.
    world_position.x += offset.x;
    world_position.z += offset.z;
    world_position.y += offset.y + ocean.params.z;

    var out: VertexOutput;
    out.rest_position = rest_position;
    out.world_position = world_position.xyz;
    out.clip_position = camera.view_proj * world_position;
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
    // Distance to the eye, per pixel: the far rings' cells are kilometers
    // wide, too big to interpolate a distance across.
    // With an orthographic lens (a chart, looking down) there's no "far
    // away": every point is drawn as if close up - its detail limited only by
    // how much sea a pixel covers - and nothing hazes.
    var view_depth = length(camera.view_pos.xyz - in.world_position);
    if (camera.orthographic > 0.5) {
        view_depth = 0.0;
    }
    // The ripples from the ripple map, when it stands in for the detail
    // waves - its two layers (see prefabs::ocean::ripples), sampled here while
    // control flow is still uniform (a texture read needs its derivatives).
    let textured_ripples = ocean.tuning.w > 0.5;
    var ripple_slope = vec2<f32>(0.0);
    var ripple_shown = 0.0;
    if (textured_ripples) {
        // How it's read (OceanSettings::ripple_layers / ripple_stochastic).
        let stochastic = ocean.detail.z > 0.5;
        let first = ripple_layer(in.rest_position, ocean.ripple_a, ocean.ripple_cells.xy, ocean.ripple_info.x, pixel_size, stochastic);
        var layers = first.xy;
        var shown = first.z;
        if (ocean.detail.y > 1.5) {
            let second = ripple_layer(in.rest_position, ocean.ripple_b, ocean.ripple_cells.zw, ocean.ripple_info.y, pixel_size, stochastic);
            layers += second.xy;
            shown += second.z;
        } else {
            // One layer carries both one's share (each is scaled for two).
            layers *= 1.41421356;
            shown *= 2.0;
        }
        // Gusts: the ripples' strength drifting across the sea (pinned to
        // it - the same noise as the depth tint's, on its wrap).
        let gust_at = ocean.noise_anchor.xy + in.rest_position;
        let gust_noise = sea_value_noise(gust_at, RIPPLE_GUST_SIZE) * 0.65 + sea_value_noise(gust_at, RIPPLE_GUST_SIZE * 0.25) * 0.35;
        let gust = mix(RIPPLE_GUST_MIN, RIPPLE_GUST_MAX, gust_noise);
        // Near to far (ocean.look): the near ripples fade out with distance
        // and the far layer takes over - sooner than the mipmaps alone would
        // have them go. Faded ripples count as lost slope (wider glint).
        let far_blend = smoothstep(ocean.look.x, ocean.look.y, view_depth);
        let near = 1.0 - far_blend;
        ripple_slope = layers * gust * near;
        ripple_shown = ocean.ripple_bias.z * shown * near * near;
        // The far sea: the map 64x larger (30-500 m patterns) - so the open
        // sea isn't one smooth plane.
        let far_strength = ocean.detail.w * max(far_blend, 1.0 - first.z);
        if (ocean.detail.w > 0.0) {
            let far = ripple_layer(in.rest_position, ocean.ripple_far, ocean.ripple_far_cells.xy, ocean.ripple_far_cells.z, pixel_size, stochastic);
            ripple_slope += far.xy * far_strength;
        }
    }

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
    let wave_visibility = 1.0 - smoothstep(OPEN_SEA_BLEND_START, OPEN_SEA_BLEND_END, view_depth);

    // Per-pixel normal, from every wave at this point's rest position - the
    // geometry waves (with their Gerstner crest pinch) and the detail waves
    // too short for any mesh - each faded out once it's too small for the
    // pixel (see surface_waves). The same everywhere, so nothing changes
    // where the moving rings end.
    let surface = surface_waves(in.rest_position, pixel_size);
    let slope = surface.slope + ripple_slope;
    let normal = normalize(vec3<f32>(-slope.x, 1.0 - surface.crest, -slope.y));
    // How much of the waves' total slope has faded out at this pixel, 0..1 -
    // see FLAT_WATER_MIN_FACING. The total is summed once on the CPU, with
    // the detail waves or the ripple map's in their place.
    let total_variance = select(ocean.shading.w, ocean.ripple_info.w, textured_ripples);
    let lost_slope = clamp((total_variance - surface.shown_variance - ripple_shown) / max(total_variance, 1e-8), 0.0, 1.0);

    let light_dir = normalize(light.position - in.world_position);
    let diffuse_strength = max(dot(normal, light_dir), 0.0);
    let diffuse_color = light.color * diffuse_strength;

    // Sun-glint - one of the strongest "this is actually water" cues (a
    // real reflective liquid surface, not a painted texture), so this is
    // deliberately punchier than a typical specular highlight: a lower
    // exponent (broader, easier-to-catch glint) combined with a stronger
    // multiplier below (specular_color * 1.4, not the usual subtle * 0.2-0.5)
    // rather than the tight speck a higher exponent alone would give.
    // Toward the camera - with an orthographic lens, the same for every
    // pixel: straight back along the way it looks.
    var view_dir = normalize(camera.view_pos.xyz - in.world_position);
    if (camera.orthographic > 0.5) {
        view_dir = -camera.forward.xyz;
    }
    let reflect_dir = reflect(-light_dir, normal);
    // Where the waves are too small to see (far off, high up), their slopes
    // are still there, tilting the tiny facets every which way: the sun's
    // reflection spreads into a wider, dimmer glitter instead of one
    // mirror-sharp, full-strength glint. So the exponent widens with the
    // slope variance that faded out (a reflection turns twice as far as its
    // facet - 4x the variance) and the glint dims to keep its total light
    // (normalized Phong: brightness ~ exponent + 1).
    let unseen_variance = total_variance * lost_slope;
    let glint_exponent = 1.0 / (1.0 / 48.0 + 4.0 * unseen_variance);
    let glint_brightness = (glint_exponent + 1.0) / 49.0;
    let specular_strength = pow(max(dot(view_dir, reflect_dir), 0.0), glint_exponent) * glint_brightness;
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
    let height_01 = clamp(surface.height / ocean.params.w + depth_tint(in.rest_position, pixel_size) * ocean.look.z, -1.0, 1.0) * 0.5 + 0.5;
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
    let scene_depth_raw = scene_depth(vec2<i32>(in.clip_position.xy));
    let scene_z = linearize_depth(scene_depth_raw, camera.near, camera.far);
    let water_z = linearize_depth(in.clip_position.z, camera.near, camera.far);
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
    let capped = mix(shore_tinted, WHITECAP_COLOR, whitecap);

    // Final composition: light the water (and its foam) first, THEN blend in
    // the sky reflection - unlit, since reflected sky is already light. (It
    // used to be blended in before lighting, which multiplied the pale sky
    // color by ~1.35 and clipped it toward grey-white at every diagonal
    // view.) Foam isn't a mirror, so it takes no reflection; the sun glint
    // goes on top of everything.
    let lit_water = (ambient_color + diffuse_color) * capped;
    let reflection = fresnel_amount * (1.0 - max(shore_foam, whitecap));
    let result = mix(lit_water, SKY_REFLECTION_COLOR, reflection) + specular_color * 1.4;

    // Long-distance atmospheric haze - fades the water toward the sea's own
    // horizon blue (SEA_HORIZON_COLOR), which sits just below the sky's
    // paler haze band - a soft but clear horizon line, like the photos.
    //
    // The ocean mesh reaches ~2,100 km and re-centres on the camera every
    // frame (Ocean::update), so there's room for a genuinely
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
    // With an orthographic lens (a chart looking down) there's no horizon to
    // haze toward.
    let horizon_dist = select(length(in.world_position.xz), 0.0, camera.orthographic > 0.5);
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
