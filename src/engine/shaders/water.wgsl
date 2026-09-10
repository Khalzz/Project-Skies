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
// lightened from an earlier, near-black pass at this (0.0, 0.03, 0.08). That
// was tuned back when waves were tall enough for a genuinely deep-looking
// trough to be a rare, dramatic accent; now that AMPLITUDE_* sums to a small
// 3.0 (see that constant's own comment), what height_01 = 0 actually
// represents is just an ordinary shallow dip in small chop, not a dramatic
// trough - the old near-black there read as an abrupt, out-of-place dark
// spot rather than part of the water. See also COLOR_HEIGHT_SCALE below,
// which is what actually lets height_01 reach 0 from realistic wave heights
// in the first place rather than only from a near-impossible in-phase sum of
// all six terms at once.
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
const SKY_REFLECTION_COLOR: vec3<f32> = vec3<f32>(0.55, 0.7, 0.85);

// Six wave terms instead of three, each traveling in its own (non axis-
// aligned) direction with its own frequency/speed/phase - a small number of
// axis-aligned terms sums into an obviously periodic, tiling-looking
// interference pattern once you're far enough up to see the whole thing at
// once; more terms at irregular, non-commensurate directions/frequencies (no
// clean small-integer ratios between them) breaks that symmetry up into
// something that reads as "random" swell instead. amplitude is in world
// units (how tall this term's contribution gets), dir is this term's
// travel direction in the XZ plane (unit length), frequency is in radians
// per world unit (2*PI / frequency is the wavelength in world units), speed
// is in radians per second, phase is a constant radian offset (also there
// purely to decorrelate the terms from each other, same reason dir varies).
//
// Frequencies (wavelengths roughly 16-39 world units) are tuned for
// play::scene::spawn_world's "world" node - the small, dense, camera-
// following plane (see update_water_plane) that's actually meant to be
// looked at up close now, not the old single giant static plane these used
// to be sized for. Also checked against the player's F16 model being 13
// world units long (see AMPLITUDE_*'s own comment) - the shortest term is
// close to that length, the longest a good 3x it, so single crests read
// distinctly smaller than the plane at the short end without every term
// looking the same size. Multiple crests fit inside FALLOFF_END below at
// this wavelength, which is the point - short enough to read as real chop
// from a close/low viewpoint, not one huge gentle swell barely curving
// across the whole visible surface. `water.wgsl`'s constants are global/
// compiled in, shared by every water-shaded surface in the game, including
// "world_far" (forced calm via its own Y-scale regardless of this) and
// main_menu::scene::spawn_world's own pond.
//
// This still sits against a real geometric floor, not just a taste call:
// "world" is 1000 subdivisions over 2000 world units (see spawn_world),
// i.e. ~2-unit mesh cells - a wavelength much under ~8-10 (only a handful
// of cells) is shorter than the mesh can actually resolve, which reads as
// broken/faceted geometry rather than "sharper" waves, independent of
// anything below about motion aliasing. An earlier pass at 1000 world units
// (~1-unit cells) forced these six terms into a much narrower cluster
// (14-20) just to all clear that tighter floor - which itself read as
// repetitive, since near-equal wavelengths beat together into a fairly
// regular secondary pattern. Growing "world" back out to 2000 relaxed the
// floor enough to widen this spread again (16-39, a ~2.4x range vs. the
// previous ~1.4x) without any term coming close to it - DIR_*/PHASE_* were
// also spread more evenly around the circle/across radian offsets for the
// same reason. If the repeating look ever comes back, this spread (and
// DIR_*/PHASE_*) is the lever to reach for again, not wavelength alone.
//
// A pass shortening these to ~45-235 once looked "static"/frozen instead of
// choppy once actually flown over. NOT a SPEED_* problem (t*SPEED_* alone is
// far too slow on its own to explain that - even the fastest term here only
// completes a cycle every several seconds) - it's spatial aliasing: at
// flight speed, a short enough wavelength means the camera crosses a full
// crest in a small fraction of a second, so the phase sampled at true_xz
// jumps by more than a cycle between rendered frames and reads as frozen/
// flickering rather than flowing. This range's shortest wavelength (~16) is
// well below that failure point, but AMPLITUDE_* is tiny now (well under 1
// total, see its own comment) - the same aliasing may still technically
// happen, but on a wave amplitude this small it should read as faint
// shimmer rather than the dramatic frozen-blob look it caused before. The
// true fix (subsampling/anti-aliasing the wave phase itself, or capping how
// much true_xz can move per frame) still hasn't been needed - revisit if
// this is still visible.
//
// SPEED_* used to be slowed specifically to mask a since-fixed bug (camera-
// relative rendering, see GameObject::to_raw / `true_xz` below), then pushed
// back up further than actually wanted once that was fixed - now split the
// difference between those two passes.
//
// AMPLITUDE_* is scaled against a concrete reference: the player's F16 model
// is 13 world units long (see play::scene::spawn_world's "player" node).
// Halved three times, then cut a further 5% - the huge-static-ocean-era
// total summed to 40 (read as oversized rounded blobs up close, "goo" was
// the right word for it), then 3.0, then 1.5, then 0.75 (each still too
// much) - now sums to ~0.713, i.e. under a 1.5-unit crest-to-trough swing,
// proportionate small chop rather than dominating the plane's own
// silhouette. MAX_WAVE_HEIGHT below is computed from these, not hand-set,
// so it tracks automatically whenever this changes again.
//
// Amplitude is in world units (how tall a term's contribution gets) - see
// the conversation this came out of for why these specific six values.
const DIR_1: vec2<f32> = vec2<f32>(0.985, 0.174);
const AMPLITUDE_1: f32 = 0.242;
const FREQUENCY_1: f32 = 0.3855;
const SPEED_1: f32 = 0.11;
const PHASE_1: f32 = 0.0;

const DIR_2: vec2<f32> = vec2<f32>(0.259, 0.966);
const AMPLITUDE_2: f32 = 0.166;
const FREQUENCY_2: f32 = 0.3190;
const SPEED_2: f32 = -0.08;
const PHASE_2: f32 = 2.1;

const DIR_3: vec2<f32> = vec2<f32>(-0.643, 0.766);
const AMPLITUDE_3: f32 = 0.119;
const FREQUENCY_3: f32 = 0.2663;
const SPEED_3: f32 = 0.14;
const PHASE_3: f32 = 4.4;

const DIR_4: vec2<f32> = vec2<f32>(-0.966, -0.259);
const AMPLITUDE_4: f32 = 0.076;
const FREQUENCY_4: f32 = 0.2252;
const SPEED_4: f32 = -0.10;
const PHASE_4: f32 = 1.3;

const DIR_5: vec2<f32> = vec2<f32>(-0.342, -0.940);
const AMPLITUDE_5: f32 = 0.048;
const FREQUENCY_5: f32 = 0.1939;
const SPEED_5: f32 = 0.17;
const PHASE_5: f32 = 5.2;

const DIR_6: vec2<f32> = vec2<f32>(0.766, -0.643);
const AMPLITUDE_6: f32 = 0.062;
const FREQUENCY_6: f32 = 0.1624;
const SPEED_6: f32 = -0.06;
const PHASE_6: f32 = 3.6;

// The tallest a crest can ever get (all six terms peaking at once) - used
// both as a compile-time constant and to normalize height into -1..1 for
// fs_main's own depth shading (troughs bottom out at -MAX_WAVE_HEIGHT,
// symmetric with crests).
const MAX_WAVE_HEIGHT: f32 = AMPLITUDE_1 + AMPLITUDE_2 + AMPLITUDE_3 + AMPLITUDE_4 + AMPLITUDE_5 + AMPLITUDE_6;
// fs_main's own height_01 normalizes by this instead of MAX_WAVE_HEIGHT
// directly - a sum of six independent-phase sine terms essentially never
// actually reaches anywhere near MAX_WAVE_HEIGHT in practice (that needs all
// six to peak in phase at once), so normalizing color by the true
// theoretical max left height_01 clustered tightly around 0.5 almost
// everywhere - DEEP_COLOR/WATER_COLOR's own extremes basically never showed,
// which read as flat, low-variation coloring (and, combined with a small
// number of fixed travel directions, like an obviously repeating texture
// rather than natural-looking chop). Half of MAX_WAVE_HEIGHT is close enough
// to the sum's *typical* excursion that ordinary wave heights - not just
// rare in-phase peaks - now drive real color variation. Geometry (the
// world_position.y offset in vs_main) still uses the true MAX_WAVE_HEIGHT -
// that one has to cover the actual theoretical extreme to guarantee
// non-negative height, this is purely a fs_main coloring concern. Pulled
// back up slightly from an earlier 0.5 to 0.6 - 0.5 fixed the flat/
// repeating look but was asked to variate "a little less" once seen in
// motion; 0.6 keeps most of that fix while easing off the sensitivity a bit.
const COLOR_HEIGHT_SCALE: f32 = MAX_WAVE_HEIGHT * 0.6;
// Camera-distance (world units, matches dist_from_camera in vs_main) band
// over which wave amplitude fades from full (at/inside FALLOFF_START) to
// zero (at/beyond FALLOFF_END) - what actually gives the "detailed near,
// calm far" look, on top of whatever mesh density happens to be nearby. See
// play::scene::spawn_world's "world"/"world_far" node pair - "world" is a
// small, dense plane recentered under the camera every frame for real
// near-camera resolution, "world_far" is a much bigger, near-flat one (its
// own per-instance Y-scale read back via y_scale in vs_main, not this
// falloff) that covers the same huge area the old single plane did, so
// there's always a water surface out past FALLOFF_END for fog/shore-foam to
// read against - this constant alone does not create that calm-far surface,
// it just makes sure "world"'s own waves don't cut off with a visible edge.
// Pulled in twice, now pushed back out once (1200/2800 -> 400/900 -> this)
// to match "world" itself (see spawn_world) shrinking twice then growing
// back to 2000 (half-size 1000) - FALLOFF_END has to stay comfortably
// inside that half-size or the fade-to-flat zone doesn't finish before the
// mesh's own edge, and the seam this whole falloff mechanism exists to hide
// shows up again. See also RADIUS below, just past FALLOFF_END - "world" is
// already flat by FALLOFF_END, so the hard circular cutoff at RADIUS lands
// on water that already looks the same as "world_far" underneath it.
const FALLOFF_START: f32 = 360.0;
const FALLOFF_END: f32 = 800.0;
// Hard circular cutoff for "world" specifically (see y_scale's own gating in
// fs_main - "world_far" is never discarded, it's the permanent backdrop) -
// past this, "world"'s fragments are discarded outright rather than just
// faded, so the near-detail patch reads as a clean circle instead of a
// square with its diagonal corners poking out past FALLOFF_END (a square
// mesh's corners reach ~1.41x further than its edges at the same "radius").
// Sits just past FALLOFF_END, inside "world"'s own half-size (1000), so the
// cutoff always lands on already-flat, already-matching water via the
// smooth amplitude falloff above - never a visible edge, since by RADIUS
// there's nothing left for the hard discard to visibly remove. A first-pass
// value, not derived from anything - revisit if the circle ends up in the
// wrong place.
const RADIUS: f32 = 900.0;
// Same idea as FALLOFF_START/END above but for camera altitude instead of
// distance - fades "world"'s own amplitude out as the camera climbs, same
// smoothstep shape, so it goes flat/matching before HEIGHT_FALLOFF_END is
// ever reached (see fs_main's discard, gated on this same END value). A
// single hard HEIGHT_CUTOFF discard with no fade leading into it (an
// earlier pass at this) popped "world" in/out abruptly on approach/climb -
// exactly the mistake FALLOFF_START/END+RADIUS above were designed to
// avoid, just missed here the first time.
const HEIGHT_FALLOFF_START: f32 = 200.0;
const HEIGHT_FALLOFF_END: f32 = 500.0;
// How tightly the fresnel blend (see fs_main) concentrates toward true
// grazing angles - higher means only very shallow viewing angles pick up
// SKY_REFLECTION_COLOR, most of the surface stays true water color; lower
// spreads the reflective look further back toward straight-down views.
const FRESNEL_POWER: f32 = 4.0;
// How much of the fresnel blend actually reaches SKY_REFLECTION_COLOR even
// at a full 90-degree grazing angle - 1.0 would let grazing views go fully
// sky-colored, this caps it short of that so the surface never completely
// stops reading as water.
const FRESNEL_STRENGTH: f32 = 0.75;

// Foam tint for shore/object intersections - see shore_foam in fs_main.
const FOAM_COLOR: vec3<f32> = vec3<f32>(0.55, 0.75, 0.95);
// How many world units of linear depth separation still counts as "water
// intersecting solid geometry" - 0 separation (water sitting exactly on/in
// something solid) is always full foam, this is how far that falls off to
// nothing. Larger = a wider foam band around every shore/object.
const SHORE_FOAM_RANGE: f32 = 25.0;
// Shore / object-collision foam is tied to the SAME visibility envelope as
// the waves themselves rather than its own distance constants: FALLOFF_START/
// END over camera XZ-distance, HEIGHT_FALLOFF_START/END over camera altitude,
// and the "world"-only y_scale gate (see foam_wave_visibility in fs_main).
// Foam is a near-surface detail - it should appear exactly where the detailed
// "world" water patch is and fade out on the same schedule, not linger on the
// flat "world_far" backdrop (which has no waves to collide against anyway) or
// fringe every coastline when viewed from flight altitude.

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
    @location(1) world_normal: vec3<f32>,
    @location(2) world_position: vec3<f32>,
    @location(3) view_depth: f32,
    // This vertex's own wave_height() result - interpolated across each
    // triangle and read back in fs_main to blend between DEEP_COLOR and
    // WATER_COLOR (see height_01). Cheaper than recomputing the wave
    // function from world_position in the fragment shader, and exactly
    // matches what the vertex shader actually displaced this point by.
    @location(4) wave_height: f32,
    // This instance's own y_scale (see vs_main) - constant across every
    // vertex of one instance, so interpolation just carries it through
    // unchanged. Lets fs_main's RADIUS/HEIGHT_FALLOFF_END discard tell
    // "world" (y_scale near 1.0) apart from "world_far" (y_scale near 0.03)
    // without a new bind group - see RADIUS's own comment for why only
    // "world" gets discarded.
    @location(5) y_scale: f32,
};

// Sum of six sine waves at different (non axis-aligned) directions/
// frequencies/speeds/phases (see the DIR_*/AMPLITUDE_*/FREQUENCY_*/SPEED_*/
// PHASE_* constants above) so the surface doesn't look like one obviously-
// repeating ripple. Driven by WORLD-space x/z (post instance+mesh transform,
// see vs_main) rather than the mesh's own local -0.5..0.5 space, so
// wavelength reads in world units regardless of whatever scale a node
// applies to size the plane. Each term is A*sin(dot(p, dir)*F + t*S + phase)
// - a plane wave traveling along `dir` - rather than the axis-aligned
// x*F/z*F this used to be, so `dir` (any 2D direction, not just X or Z) is
// what actually breaks the old grid-aligned symmetry.
fn wave_height(x: f32, z: f32, t: f32) -> f32 {
    let p = vec2<f32>(x, z);
    var h = 0.0;
    h += AMPLITUDE_1 * sin(dot(p, DIR_1) * FREQUENCY_1 + t * SPEED_1 + PHASE_1);
    h += AMPLITUDE_2 * sin(dot(p, DIR_2) * FREQUENCY_2 + t * SPEED_2 + PHASE_2);
    h += AMPLITUDE_3 * sin(dot(p, DIR_3) * FREQUENCY_3 + t * SPEED_3 + PHASE_3);
    h += AMPLITUDE_4 * sin(dot(p, DIR_4) * FREQUENCY_4 + t * SPEED_4 + PHASE_4);
    h += AMPLITUDE_5 * sin(dot(p, DIR_5) * FREQUENCY_5 + t * SPEED_5 + PHASE_5);
    h += AMPLITUDE_6 * sin(dot(p, DIR_6) * FREQUENCY_6 + t * SPEED_6 + PHASE_6);
    return h;
}

// Analytic partial derivatives of wave_height, for a lit normal without a
// second (finite-difference) texture/geometry sample. Each term's own
// d/dx = A*F*dir.x*cos(...), d/dz = A*F*dir.z*cos(...) - same cos(...)
// argument as that term's own sin(...) in wave_height above, just scaled by
// F and this axis's share of `dir`.
fn wave_height_dx(x: f32, z: f32, t: f32) -> f32 {
    let p = vec2<f32>(x, z);
    var d = 0.0;
    d += AMPLITUDE_1 * FREQUENCY_1 * DIR_1.x * cos(dot(p, DIR_1) * FREQUENCY_1 + t * SPEED_1 + PHASE_1);
    d += AMPLITUDE_2 * FREQUENCY_2 * DIR_2.x * cos(dot(p, DIR_2) * FREQUENCY_2 + t * SPEED_2 + PHASE_2);
    d += AMPLITUDE_3 * FREQUENCY_3 * DIR_3.x * cos(dot(p, DIR_3) * FREQUENCY_3 + t * SPEED_3 + PHASE_3);
    d += AMPLITUDE_4 * FREQUENCY_4 * DIR_4.x * cos(dot(p, DIR_4) * FREQUENCY_4 + t * SPEED_4 + PHASE_4);
    d += AMPLITUDE_5 * FREQUENCY_5 * DIR_5.x * cos(dot(p, DIR_5) * FREQUENCY_5 + t * SPEED_5 + PHASE_5);
    d += AMPLITUDE_6 * FREQUENCY_6 * DIR_6.x * cos(dot(p, DIR_6) * FREQUENCY_6 + t * SPEED_6 + PHASE_6);
    return d;
}

fn wave_height_dz(x: f32, z: f32, t: f32) -> f32 {
    let p = vec2<f32>(x, z);
    var d = 0.0;
    d += AMPLITUDE_1 * FREQUENCY_1 * DIR_1.y * cos(dot(p, DIR_1) * FREQUENCY_1 + t * SPEED_1 + PHASE_1);
    d += AMPLITUDE_2 * FREQUENCY_2 * DIR_2.y * cos(dot(p, DIR_2) * FREQUENCY_2 + t * SPEED_2 + PHASE_2);
    d += AMPLITUDE_3 * FREQUENCY_3 * DIR_3.y * cos(dot(p, DIR_3) * FREQUENCY_3 + t * SPEED_3 + PHASE_3);
    d += AMPLITUDE_4 * FREQUENCY_4 * DIR_4.y * cos(dot(p, DIR_4) * FREQUENCY_4 + t * SPEED_4 + PHASE_4);
    d += AMPLITUDE_5 * FREQUENCY_5 * DIR_5.y * cos(dot(p, DIR_5) * FREQUENCY_5 + t * SPEED_5 + PHASE_5);
    d += AMPLITUDE_6 * FREQUENCY_6 * DIR_6.y * cos(dot(p, DIR_6) * FREQUENCY_6 + t * SPEED_6 + PHASE_6);
    return d;
}

@vertex
fn vs_main(model: VertexInput, instance: InstanceInput) -> VertexOutput {
    let model_matrix = mat4x4<f32>(
        instance.model_matrix_0,
        instance.model_matrix_1,
        instance.model_matrix_2,
        instance.model_matrix_3,
    );

    var world_position: vec4<f32> = model_matrix * transform.model_matrix * vec4<f32>(model.position, 1.0);

    let t = light.time;
    // world_position.xz here is camera-relative (see GameObject::to_raw) -
    // adding the camera's own true world position back recovers true world
    // coordinates for the wave phase, so a given point in the world always
    // waves the same way regardless of where the camera happens to be. Do
    // NOT feed this into camera.view_proj / view_depth below - those still
    // need the camera-relative value for GPU float precision, same as ever.
    let true_xz = world_position.xz + light.camera_position.xz;
    // Distance from the camera, in the XZ plane - still camera-relative
    // world_position on purpose, it's already exactly "distance from
    // camera" for any instance (that's what to_raw's translation encodes).
    let dist_from_camera = length(world_position.xz);
    // Recovers this instance's own Transform3D.scale.y from its model
    // matrix's Y-basis column (works since rotation here is orthonormal) -
    // "world_far" (see play::scene::spawn_world) sets this far below 1.0 to
    // force its own waves calm/flat regardless of distance, "world" leaves
    // it at 1.0 so only FALLOFF_START/END governs it.
    let y_scale = length(instance.model_matrix_1.xyz);
    let amplitude_falloff = 1.0 - smoothstep(FALLOFF_START, FALLOFF_END, dist_from_camera);
    // Same shape as amplitude_falloff, over camera altitude instead of
    // distance - see HEIGHT_FALLOFF_START/END's own comment for why this
    // exists (fixes "world" popping in/out on approach instead of fading).
    let camera_height = light.camera_position.y;
    let height_falloff = 1.0 - smoothstep(HEIGHT_FALLOFF_START, HEIGHT_FALLOFF_END, camera_height);
    let total_amp = amplitude_falloff * y_scale * height_falloff;

    // Shifted up by MAX_WAVE_HEIGHT (wave_height's own theoretical minimum
    // is exactly -MAX_WAVE_HEIGHT, all six terms troughing at once) rather
    // than clamped - a clamp flattens every point that would've gone
    // negative into a dead, untilted patch at 0, which reads as the surface
    // visibly breaking character wherever it happens; an offset keeps the
    // exact same continuous, organic shape as before, just repositioned so
    // its lowest point now sits at 0 instead of below it. Derivatives are
    // untouched by this - a constant offset doesn't change slope anywhere.
    let raw_height = wave_height(true_xz.x, true_xz.y, t) * total_amp;
    let height = raw_height + MAX_WAVE_HEIGHT;
    world_position.y += height;

    let dx = wave_height_dx(true_xz.x, true_xz.y, t) * total_amp;
    let dz = wave_height_dz(true_xz.x, true_xz.y, t) * total_amp;
    // Height-field normal, computed directly in world space - assumes the
    // water plane itself isn't rotated (a reasonable assumption for a flat
    // sheet of water; skips normal_matrix entirely, unlike depth.wgsl).
    let world_normal = normalize(vec3<f32>(-dx, 1.0, -dz));

    var out: VertexOutput;
    out.world_normal = world_normal;
    out.world_position = world_position.xyz;
    out.clip_position = camera.view_proj * world_position;
    out.view_depth = length(camera.view_pos.xyz - out.world_position);
    // raw_height (the -MAX_WAVE_HEIGHT..+MAX_WAVE_HEIGHT centered value,
    // already scaled by total_amp above), not the shifted `height` actually
    // written to world_position.y above - fs_main's own height_01
    // normalization expects a value centered on 0, not the offset one, and
    // needs the falloff/y_scale already applied or flattened-out water would
    // still show full-contrast color.
    out.wave_height = raw_height;
    out.y_scale = y_scale;
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // See RADIUS/HEIGHT_FALLOFF_END's own comments - only "world" (y_scale
    // near 1.0) is ever discarded here; "world_far" (y_scale near 0.03)
    // always renders, it's what stays visible underneath once "world" cuts
    // off. Both thresholds sit past where vs_main's own amplitude_falloff/
    // height_falloff have already smoothly flattened "world" to match
    // "world_far", so the discard itself is never visible - it's just
    // removing water that already looks identical to what's underneath.
    // in.world_position.xz is still camera-relative (the wave-height offset
    // only touched .y in vs_main), so its length is exactly distance from
    // camera - same value vs_main's own dist_from_camera computes.
    if (in.y_scale > 0.5) {
        let dist_from_camera = length(in.world_position.xz);
        let camera_height = light.camera_position.y;
        if (dist_from_camera > RADIUS || camera_height > HEIGHT_FALLOFF_END) {
            discard;
        }
    }

    // Lower than before, and diffuse no longer gets flattened against a
    // single ambient-dominated base - the point is for the normal-driven
    // diffuse term (which already varies per-pixel with wave slope) to read
    // as real shading contrast (bright wave faces toward the light, darker
    // troughs/shadowed faces) instead of a near-uniform wash.
    let ambient_strength = 0.35;
    let ambient_color = light.color * ambient_strength;

    // The "sun" node sits at an absurd altitude (Y=1,000,000 - see
    // play::scene::spawn_world/main_menu::scene::spawn_world) specifically
    // so it reads as a fixed, effectively-at-infinity direction rather than
    // a nearby point light - which means light_dir is very close to
    // straight up (0,1,0) EVERYWHERE on the water, regardless of where you
    // are on it. At this wave field's current scale (AMPLITUDE_*/
    // FREQUENCY_* above - a max slope on the order of ~0.01 radians), the
    // surface normal barely leaves vertical anywhere, so this plain
    // dot-product diffuse term stays close to its own max basically
    // everywhere - a calm, evenly-lit sea, which is the physically honest
    // result for waves this gentle. (An earlier pass here tried
    // contrast-stretching this - smoothstep, then a steep pow() exponent -
    // to manufacture visible per-wave shading anyway; neither actually
    // creates variation that isn't in the input, so both were removed
    // rather than tuned further. If real per-pixel wave shading is wanted
    // again, it needs steeper actual geometry - shorter wavelengths at
    // comparable amplitude - not a steeper light response curve. Visual
    // interest at this scale comes from the fresnel reflection below and
    // the depth-color gradient further down instead.)
    let light_dir = normalize(light.position - in.world_position);
    let diffuse_strength = max(dot(in.world_normal, light_dir), 0.0);
    let diffuse_color = light.color * diffuse_strength;

    // Sun-glint - one of the strongest "this is actually water" cues (a
    // real reflective liquid surface, not a painted texture), so this is
    // deliberately punchier than a typical specular highlight: a lower
    // exponent (broader, easier-to-catch glint) combined with a stronger
    // multiplier below (specular_color * 1.4, not the usual subtle * 0.2-0.5)
    // rather than the tight speck a higher exponent alone would give.
    let view_dir = normalize(camera.view_pos.xyz - in.world_position);
    let reflect_dir = reflect(-light_dir, in.world_normal);
    let specular_strength = pow(max(dot(view_dir, reflect_dir), 0.0), 48.0);
    let specular_color = specular_strength * light.color;

    // A direct, always-visible blend across the wave's ENTIRE current height
    // range - DEEP_COLOR at the very lowest point (height_01 = 0) up to
    // WATER_COLOR at the very highest (height_01 = 1), darker at low spots
    // and lighter at high ones everywhere, not just a rare accent gated to
    // extreme troughs (an earlier pass at this only blended toward
    // DEEP_COLOR near the deepest few percent of the range, via a pow()
    // curve - correct in theory, but with such a small total height range
    // now (see AMPLITUDE_* above) it read as barely-there). No crest-based
    // whitening here (an earlier pass at this used height alone as a foam
    // proxy) - real foam belongs at actual shoreline/object intersections,
    // which needs the water to see world geometry it doesn't have access to
    // yet (see the conversation this came out of for the options there).
    let height_01 = clamp(in.wave_height / COLOR_HEIGHT_SCALE, -1.0, 1.0) * 0.5 + 0.5;
    let base_color = mix(DEEP_COLOR, WATER_COLOR, height_01);

    // Fresnel - blends toward SKY_REFLECTION_COLOR the more edge-on the
    // surface is being viewed (view_dir already computed above, for
    // specular) - see that constant's own doc comment for why this, more
    // than the exact base tint, is what makes an opaque water shader read
    // as real rather than a flat-colored plane. 1.0 - dot(...) is 0 looking
    // straight down the normal, 1 at a true grazing angle.
    let fresnel_amount = pow(1.0 - clamp(dot(view_dir, in.world_normal), 0.0, 1.0), FRESNEL_POWER) * FRESNEL_STRENGTH;
    let fresnel_tinted = mix(base_color, SKY_REFLECTION_COLOR, fresnel_amount);

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
    let shore_distance = abs(scene_z - water_z);
    // Gate the foam to the exact same envelope the waves use: same camera
    // XZ-distance fade (FALLOFF_START/END), same camera-altitude fade
    // (HEIGHT_FALLOFF_START/END), and zeroed on the flat "world_far" backdrop
    // via the same y_scale > 0.5 "world"-only test the discard at the top of
    // fs_main uses. in.world_position.xz is still camera-relative, so its
    // length is distance from camera - identical to vs_main's dist_from_camera.
    let foam_dist_from_camera = length(in.world_position.xz);
    let foam_camera_height = light.camera_position.y;
    let foam_wave_visibility =
        (1.0 - smoothstep(FALLOFF_START, FALLOFF_END, foam_dist_from_camera))
        * (1.0 - smoothstep(HEIGHT_FALLOFF_START, HEIGHT_FALLOFF_END, foam_camera_height))
        * step(0.5, in.y_scale);
    let shore_foam = (1.0 - smoothstep(0.0, SHORE_FOAM_RANGE, shore_distance)) * foam_wave_visibility;
    let shore_tinted = mix(fresnel_tinted, FOAM_COLOR, shore_foam);

    let result = (ambient_color + diffuse_color) * shore_tinted + specular_color * 1.4;

    // Long-distance atmospheric haze - fades the water toward a pale hazy
    // blue so the water/sky seam softens into a band instead of a hard line.
    //
    // "world_far" is ~1.5M half-extent and re-centres on the camera every
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
    // fog_color / fog_start / fog_end are kept matched to depth.wgsl's model
    // fog (same three values) so a distant object and the sea under it haze
    // to the same tone. Retune one -> retune the other.
    let fog_color = vec3<f32>(0.72, 0.80, 0.88);
    let fog_start = 80000.0;
    let fog_end = 1400000.0;
    let horizon_dist = length(in.world_position.xz);
    let fog_factor = smoothstep(fog_start, fog_end, horizon_dist);
    let fogged_color = mix(result, fog_color, fog_factor);

    return vec4<f32>(fogged_color, 1.0);
}
