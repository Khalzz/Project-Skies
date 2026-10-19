// Moves every live particle one frame - see engine::particles::renderer.
// Particles are CAMERA-RELATIVE (like everything the renderer draws), so
// each frame they're also shifted by however far the camera moved - that's
// what keeps them precise however far from the world origin you fly.
//
// Particle / EmitterParams / Frame must match particles.wgsl and
// ParticleRenderer's GpuParticle / GpuEmitter / GpuFrame.

struct Particle {
    position: vec3<f32>,
    // Seconds alive - negative for one spawned partway through this frame
    // (see ParticleRenderer's spawning), dead once >= lifetime.
    age: f32,
    velocity: vec3<f32>,
    // 0 = an empty slot.
    lifetime: f32,
    rotation: f32,
    spin: f32,
    seed: f32,
    intensity: f32,
    emitter: u32,
    // The velocity it inherited from the emitter at spawn - streaks are drawn
    // along its motion relative to this (see Facing::Velocity).
    carrier_x: f32,
    carrier_y: f32,
    carrier_z: f32,
};

struct EmitterParams {
    // gravity (x g), drag (1/s), wind (0..1), turbulence (m/s²)
    forces: vec4<f32>,
    // brightness, facing (0 camera, 1 velocity, 2 horizontal), stretch (s), soft fade (m)
    look: vec4<f32>,
    // shape (0 soft circle, 1 texture, 2 flipbook), columns, rows, fps
    shape: vec4<f32>,
    // lit, water (0 none, 1 die, 2 bounce, 3 float), restitution, unused
    flags: vec4<f32>,
    // intensity scales alpha, size (0/1), unused x2
    scales: vec4<f32>,
    // size over life, 8 samples
    size: array<vec4<f32>, 2>,
    // color over life, 8 samples
    color: array<vec4<f32>, 8>,
};

struct Frame {
    // xyz: camera right, w: near plane
    camera_right: vec4<f32>,
    // xyz: camera up, w: far plane
    camera_up: vec4<f32>,
    // xyz: camera forward, w: time (s)
    camera_forward: vec4<f32>,
    // xyz: how far the camera moved since last frame, w: this frame's dt (0 = paused)
    camera_delta: vec4<f32>,
    // xyz: wind velocity (m/s), w: sea level (camera-relative y)
    wind: vec4<f32>,
    // xyz: direction toward the sun, w: ambient light
    sun_direction: vec4<f32>,
    // rgb: sunlight color
    sun_color: vec4<f32>,
};

@group(0) @binding(0)
var<storage, read_write> particles: array<Particle>;
@group(0) @binding(1)
var<storage, read> emitters: array<EmitterParams>;
@group(0) @binding(2)
var<uniform> frame: Frame;

const GRAVITY: f32 = 9.81;

@compute @workgroup_size(64)
fn update(@builtin(global_invocation_id) id: vec3<u32>) {
    let index = id.x;
    if (index >= arrayLength(&particles)) {
        return;
    }
    var p = particles[index];
    if (p.lifetime <= 0.0 || p.age >= p.lifetime) {
        return;
    }
    let e = emitters[p.emitter];

    p.position -= frame.camera_delta.xyz;

    // Only the part of the frame it's actually been alive for.
    let frame_dt = frame.camera_delta.w;
    let new_age = p.age + frame_dt;
    let dt = clamp(new_age, 0.0, frame_dt);
    if (dt > 0.0) {
        // Drag pulls it toward the air's own motion - which is the wind,
        // as much as it's carried by it.
        let air = frame.wind.xyz * e.forces.z;
        var acceleration = vec3<f32>(0.0, -GRAVITY * e.forces.x, 0.0) + (air - p.velocity) * e.forces.y;
        if (e.forces.w > 0.0) {
            let t = frame.camera_forward.w * 1.3 + p.seed * 97.0;
            acceleration += e.forces.w * vec3<f32>(
                sin(t * 1.7 + p.seed * 12.9),
                sin(t * 2.3 + p.seed * 7.1),
                sin(t * 1.3 + p.seed * 3.7),
            );
        }
        p.velocity += acceleration * dt;
        p.position += p.velocity * dt;
        p.rotation += p.spin * dt;

        // The sea - a flat plane.
        let sea = frame.wind.w;
        let response = e.flags.y;
        if (p.position.y < sea && response > 0.5) {
            if (response < 1.5) {
                p.age = p.lifetime;
                particles[index] = p;
                return;
            } else if (response < 2.5) {
                p.position.y = sea + (sea - p.position.y);
                p.velocity.y = -p.velocity.y * e.flags.z;
                p.velocity = vec3<f32>(p.velocity.x * 0.8, p.velocity.y, p.velocity.z * 0.8);
            } else {
                p.position.y = sea;
                p.velocity.y = max(p.velocity.y, 0.0);
            }
        }
    }
    p.age = new_age;
    particles[index] = p;
}
