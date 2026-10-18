//! What a particle effect or a trail looks like and how it behaves - pure
//! data, shareable between any number of emitters (see `Emitter`). Every
//! group has sensible defaults, plus builder methods on `ParticleEffect` /
//! `TrailEffect` for the common knobs:
//!
//! ```ignore
//! ParticleEffect::new()
//!     .rate(200.0)
//!     .lifetime(1.0, 2.0)
//!     .cone(20.0)
//!     .speed(5.0, 10.0)
//!     .color_over_life([1.0, 0.6, 0.2, 1.0], [0.3, 0.3, 0.3, 0.0])
//!     .size_over_life(0.5, 3.0)
//!     .additive()
//! ```
//!
//! Colors are LINEAR rgb (the screen is sRGB - 0.5 here shows as ~0.73),
//! alpha 0..1. "Over life" values run 0 = just born .. 1 = about to die.

use nalgebra::Vector3;
use rand::Rng;

/// A random value per particle, anywhere between `min` and `max`.
#[derive(Clone, Copy, Debug)]
pub struct Range {
    pub min: f32,
    pub max: f32,
}

impl Range {
    pub fn new(min: f32, max: f32) -> Self {
        Self { min, max }
    }

    pub fn constant(value: f32) -> Self {
        Self { min: value, max: value }
    }

    pub fn sample(&self, rng: &mut impl Rng) -> f32 {
        if self.max > self.min { rng.gen_range(self.min..self.max) } else { self.min }
    }

    /// The longest it can be - what sizes the particle ring.
    pub fn largest(&self) -> f32 {
        self.min.max(self.max)
    }
}

/// A value over 0..1 (of a particle's life, or a trail's length).
#[derive(Clone, Debug)]
pub enum Curve {
    Constant(f32),
    Linear(f32, f32),
    /// (t, value) keys - linear between them, held flat past the ends.
    Keys(Vec<(f32, f32)>),
}

impl Curve {
    pub fn sample(&self, t: f32) -> f32 {
        match self {
            Curve::Constant(value) => *value,
            Curve::Linear(from, to) => from + (to - from) * t.clamp(0.0, 1.0),
            Curve::Keys(keys) => sample_keys(keys, t, |a, b, f| a + (b - a) * f).unwrap_or(0.0),
        }
    }
}

/// Color (linear rgb) + alpha over 0..1.
#[derive(Clone, Debug)]
pub struct Gradient {
    pub stops: Vec<(f32, [f32; 4])>,
}

impl Gradient {
    /// From (t, rgba) stops, in any order.
    pub fn new(stops: impl Into<Vec<(f32, [f32; 4])>>) -> Self {
        let mut stops: Vec<(f32, [f32; 4])> = stops.into();
        stops.sort_by(|a, b| a.0.total_cmp(&b.0));
        Self { stops }
    }

    pub fn constant(color: [f32; 4]) -> Self {
        Self { stops: vec![(0.0, color)] }
    }

    pub fn linear(from: [f32; 4], to: [f32; 4]) -> Self {
        Self { stops: vec![(0.0, from), (1.0, to)] }
    }

    pub fn sample(&self, t: f32) -> [f32; 4] {
        sample_keys(&self.stops, t, |a, b, f| std::array::from_fn(|i| a[i] + (b[i] - a[i]) * f)).unwrap_or([1.0; 4])
    }
}

fn sample_keys<T: Copy>(keys: &[(f32, T)], t: f32, lerp: impl Fn(T, T, f32) -> T) -> Option<T> {
    let first = keys.first()?;
    if t <= first.0 {
        return Some(first.1);
    }
    for pair in keys.windows(2) {
        let ((t0, a), (t1, b)) = (pair[0], pair[1]);
        if t <= t1 {
            let f = if t1 > t0 { (t - t0) / (t1 - t0) } else { 1.0 };
            return Some(lerp(a, b, f));
        }
    }
    keys.last().map(|key| key.1)
}

// ── ParticleEffect ─────────────────────────────────────────────────────────

/// Independent particles - sparks, spray, smoke, explosions.
#[derive(Clone, Debug)]
pub struct ParticleEffect {
    pub spawn: Spawn,
    pub emission_shape: EmissionShape,
    pub motion: Motion,
    pub forces: Forces,
    pub look: ParticleLook,
    pub collision: Collision,
    pub intensity_scales: IntensityScales,
    /// Hard cap on how many can be alive from one emitter - the size of its
    /// ring of particles (the oldest is replaced when it's full).
    pub max_particles: u32,
}

/// When and how many.
#[derive(Clone, Debug)]
pub struct Spawn {
    /// Continuous stream, particles per second.
    pub rate: f32,
    /// One-off bursts - fired when the emitter is enabled (and again each
    /// time it's re-enabled), repeating every `every` seconds if set.
    pub bursts: Vec<Burst>,
    /// Particles per meter the emitter travels - even spacing at any speed
    /// (smoke behind a fast plane).
    pub per_meter: f32,
    /// How long each particle lives (s).
    pub lifetime: Range,
}

#[derive(Clone, Copy, Debug)]
pub struct Burst {
    pub count: u32,
    pub every: Option<f32>,
}

/// Where around the emitter they appear, in the emitter's own frame
/// (+Z = its forward).
#[derive(Clone, Copy, Debug)]
pub enum EmissionShape {
    Point,
    /// Anywhere inside.
    Sphere { radius: f32 },
    /// Flat, facing the emitter's forward.
    Disk { radius: f32 },
    Box { half_extents: Vector3<f32> },
    /// Along the emitter's X, centered - e.g. a whole wing's trailing edge.
    Line { length: f32 },
}

/// Their initial movement.
#[derive(Clone, Copy, Debug)]
pub struct Motion {
    /// Direction, in the emitter's frame (default: its forward, +Z).
    pub direction: Vector3<f32>,
    /// Random spread around it (degrees; 0 = a laser, 180 = every way).
    pub cone_angle: f32,
    /// Launch speed (m/s).
    pub speed: Range,
    /// How much of the parent's own velocity they keep (0 = left behind in
    /// the air, 1 = moving with it at the moment of spawn).
    pub inherit_velocity: f32,
    /// Spin around their own view axis (deg/s).
    pub spin: Range,
    /// `direction` is in the WORLD's frame instead of the emitter's - e.g.
    /// smoke that always rises, however the thing it comes off is tumbling.
    pub world_direction: bool,
}

/// What acts on them while alive.
#[derive(Clone, Copy, Debug)]
pub struct Forces {
    /// Multiples of real gravity (1 = falls like a rock, -0.1 = hot smoke rising).
    pub gravity: f32,
    /// Air drag - how fast they lose their speed relative to the air (1/s).
    pub drag: f32,
    /// How much the world's wind carries them (0..1) - through `drag`: the
    /// air they slow down against moves with the wind.
    pub wind: f32,
    /// Random wander, for smoke that churns (m/s²).
    pub turbulence: f32,
}

/// How they appear.
#[derive(Clone, Debug)]
pub struct ParticleLook {
    pub shape: ParticleShape,
    pub facing: Facing,
    /// Size over life (meters across).
    pub size: Curve,
    /// Color (linear) + alpha over life.
    pub color: Gradient,
    /// Multiplies the color - >1 glows (fire, sparks, afterburner).
    pub brightness: f32,
    pub blend: Blend,
    /// Lit by the sun (smoke, spray) or self-lit (fire, sparks).
    pub lit: bool,
    /// Fade out this close (m) to geometry/water behind them instead of a
    /// hard cut line where they intersect. 0 = off.
    pub soft_fade_distance: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ParticleShape {
    /// No texture - a round, soft-edged dot.
    SoftCircle,
    /// A sprite, path relative to assets/ (e.g. "sprites/particles/smoke.png").
    Texture(String),
    /// An animated sprite sheet: `columns` x `rows` frames, left to right
    /// then top to bottom, played at `fps` from each particle's birth.
    Flipbook { path: String, columns: u32, rows: u32, fps: f32 },
    /// Low-poly: a hard-edged polygon with `sides` sides, split into flat
    /// facets - an outer ring and an inner one - each shaded flat when
    /// `lit`, like a low-poly ball. 5-8 sides reads best.
    Faceted { sides: u32 },
}

#[derive(Clone, Copy, Debug)]
pub enum Facing {
    /// Plain billboard.
    Camera,
    /// Streaked along its motion (sparks, spray): `stretch` seconds of
    /// travel added to its length. The motion RELATIVE to what threw it -
    /// minus the emitter's velocity when it spawned, whether or not it kept
    /// any of it - so spray off a fast plane streaks the way it moves as
    /// seen from the plane (for a still emitter that's just its motion).
    Velocity { stretch: f32 },
    /// Lying flat on the horizontal (foam rings, splash marks on water).
    Horizontal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Blend {
    /// Adds light - glows, never darkens, no sorting needed.
    Additive,
    /// Covers what's behind - smoke, spray.
    Alpha,
}

#[derive(Clone, Copy, Debug)]
pub struct Collision {
    /// Hitting the sea (a flat plane at sea level).
    pub water: WaterResponse,
}

#[derive(Clone, Copy, Debug)]
pub enum WaterResponse {
    None,
    Die,
    Bounce { restitution: f32 },
    /// Stops at the surface and drifts along it.
    Float,
}

/// What the emitter's `intensity` (0..1) scales.
#[derive(Clone, Copy, Debug)]
pub struct IntensityScales {
    /// Fewer particles.
    pub rate: bool,
    /// Fainter.
    pub alpha: bool,
    /// Smaller.
    pub size: bool,
    /// Slower launch.
    pub speed: bool,
}

impl Default for Spawn {
    fn default() -> Self {
        Self { rate: 50.0, bursts: Vec::new(), per_meter: 0.0, lifetime: Range::new(1.0, 2.0) }
    }
}

impl Default for Motion {
    fn default() -> Self {
        Self { direction: Vector3::z(), cone_angle: 15.0, speed: Range::new(1.0, 2.0), inherit_velocity: 0.0, spin: Range::constant(0.0), world_direction: false }
    }
}

impl Default for Forces {
    fn default() -> Self {
        Self { gravity: 0.0, drag: 0.5, wind: 1.0, turbulence: 0.0 }
    }
}

impl Default for ParticleLook {
    fn default() -> Self {
        Self {
            shape: ParticleShape::SoftCircle,
            facing: Facing::Camera,
            size: Curve::Constant(1.0),
            color: Gradient::linear([1.0, 1.0, 1.0, 1.0], [1.0, 1.0, 1.0, 0.0]),
            brightness: 1.0,
            blend: Blend::Alpha,
            lit: false,
            soft_fade_distance: 1.0,
        }
    }
}

impl Default for Collision {
    fn default() -> Self {
        Self { water: WaterResponse::None }
    }
}

impl Default for IntensityScales {
    fn default() -> Self {
        Self { rate: true, alpha: false, size: false, speed: false }
    }
}

impl Default for ParticleEffect {
    fn default() -> Self {
        Self {
            spawn: Spawn::default(),
            emission_shape: EmissionShape::Point,
            motion: Motion::default(),
            forces: Forces::default(),
            look: ParticleLook::default(),
            collision: Collision::default(),
            intensity_scales: IntensityScales::default(),
            max_particles: 1000,
        }
    }
}

impl ParticleEffect {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn rate(mut self, per_second: f32) -> Self {
        self.spawn.rate = per_second;
        self
    }

    pub fn per_meter(mut self, per_meter: f32) -> Self {
        self.spawn.per_meter = per_meter;
        self
    }

    pub fn burst(mut self, count: u32, every: Option<f32>) -> Self {
        self.spawn.bursts.push(Burst { count, every });
        self
    }

    pub fn lifetime(mut self, min: f32, max: f32) -> Self {
        self.spawn.lifetime = Range::new(min, max);
        self
    }

    pub fn shape(mut self, shape: EmissionShape) -> Self {
        self.emission_shape = shape;
        self
    }

    pub fn direction(mut self, direction: Vector3<f32>) -> Self {
        self.motion.direction = direction;
        self
    }

    /// Launch along `direction` in the world's frame (see `Motion::world_direction`).
    pub fn world_direction(mut self, direction: Vector3<f32>) -> Self {
        self.motion.direction = direction;
        self.motion.world_direction = true;
        self
    }

    pub fn cone(mut self, degrees: f32) -> Self {
        self.motion.cone_angle = degrees;
        self
    }

    pub fn speed(mut self, min: f32, max: f32) -> Self {
        self.motion.speed = Range::new(min, max);
        self
    }

    pub fn inherit_velocity(mut self, amount: f32) -> Self {
        self.motion.inherit_velocity = amount;
        self
    }

    pub fn spin(mut self, min_deg_s: f32, max_deg_s: f32) -> Self {
        self.motion.spin = Range::new(min_deg_s, max_deg_s);
        self
    }

    pub fn forces(mut self, forces: Forces) -> Self {
        self.forces = forces;
        self
    }

    pub fn gravity(mut self, gravity: f32) -> Self {
        self.forces.gravity = gravity;
        self
    }

    pub fn drag(mut self, drag: f32) -> Self {
        self.forces.drag = drag;
        self
    }

    pub fn texture(mut self, path: impl Into<String>) -> Self {
        self.look.shape = ParticleShape::Texture(path.into());
        self
    }

    pub fn flipbook(mut self, path: impl Into<String>, columns: u32, rows: u32, fps: f32) -> Self {
        self.look.shape = ParticleShape::Flipbook { path: path.into(), columns, rows, fps };
        self
    }

    /// Low-poly polygons (see `ParticleShape::Faceted`).
    pub fn faceted(mut self, sides: u32) -> Self {
        self.look.shape = ParticleShape::Faceted { sides };
        self
    }

    pub fn facing(mut self, facing: Facing) -> Self {
        self.look.facing = facing;
        self
    }

    pub fn size_over_life(mut self, start: f32, end: f32) -> Self {
        self.look.size = Curve::Linear(start, end);
        self
    }

    pub fn color_over_life(mut self, start: [f32; 4], end: [f32; 4]) -> Self {
        self.look.color = Gradient::linear(start, end);
        self
    }

    pub fn brightness(mut self, brightness: f32) -> Self {
        self.look.brightness = brightness;
        self
    }

    pub fn additive(mut self) -> Self {
        self.look.blend = Blend::Additive;
        self
    }

    pub fn lit(mut self) -> Self {
        self.look.lit = true;
        self
    }

    pub fn water(mut self, response: WaterResponse) -> Self {
        self.collision.water = response;
        self
    }

    pub fn max_particles(mut self, max: u32) -> Self {
        self.max_particles = max;
        self
    }
}

// ── TrailEffect ────────────────────────────────────────────────────────────

/// A continuous ribbon left behind the emitter - wingtip vortices,
/// contrails, smoke trails. "Over life" here is each piece's age: 0 = fresh
/// at the emitter, 1 = the oldest end.
#[derive(Clone, Debug)]
pub struct TrailEffect {
    /// How long a piece of trail lasts (s).
    pub lifetime: f32,
    /// A new point every this many meters traveled (smaller = smoother curves).
    pub segment_spacing: f32,
    /// Width over life (m).
    pub width: Curve,
    /// Color (linear) + alpha over life.
    pub color: Gradient,
    pub brightness: f32,
    pub blend: Blend,
    pub lit: bool,
    /// Optional texture along it (path relative to assets/) - U runs along
    /// its life, V across it.
    pub texture: Option<String>,
    /// Scrolls the texture along the trail (U per second) - for wispy,
    /// moving vortices.
    pub uv_scroll: f32,
    /// How much the world's wind drifts the older parts of it (0..1).
    pub wind: f32,
    /// Fade out this close (m) to geometry/water behind it. 0 = off.
    pub soft_fade_distance: f32,
    /// What the emitter's `intensity` scales - `alpha` and `size` (width)
    /// apply; each point keeps the intensity it was laid down at.
    pub intensity_scales: IntensityScales,
    pub facing: TrailFacing,
    /// Each point moves at this velocity once laid (m/s, in the emitter's
    /// frame as it was when laid) - e.g. spray curtains spreading out from
    /// under a plane. Zero = it stays where it was laid (plus wind).
    pub point_velocity: Vector3<f32>,
}

/// How a trail's ribbon is turned.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TrailFacing {
    /// Turned to face the camera, `width` across - a wisp.
    Camera,
    /// Standing straight up from its points, `width` tall - a curtain
    /// (spray off the water), fading toward a jagged, streaky top edge.
    Upright,
    /// A low-poly 3D ridge along its points: a triangular cross-section
    /// `width` tall (base 0.7x that across), its peak jittered in height and
    /// lean point to point so the ridge line zig-zags - shaded flat, face by
    /// face, like the terrain.
    Ridge,
}

impl Default for TrailEffect {
    fn default() -> Self {
        Self {
            lifetime: 2.0,
            segment_spacing: 1.0,
            width: Curve::Linear(0.5, 2.0),
            color: Gradient::linear([1.0, 1.0, 1.0, 0.6], [1.0, 1.0, 1.0, 0.0]),
            brightness: 1.0,
            blend: Blend::Alpha,
            lit: true,
            texture: None,
            uv_scroll: 0.0,
            wind: 1.0,
            soft_fade_distance: 1.0,
            intensity_scales: IntensityScales { rate: false, alpha: true, size: false, speed: false },
            facing: TrailFacing::Camera,
            point_velocity: Vector3::zeros(),
        }
    }
}

impl TrailEffect {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn lifetime(mut self, seconds: f32) -> Self {
        self.lifetime = seconds;
        self
    }

    pub fn segment_spacing(mut self, meters: f32) -> Self {
        self.segment_spacing = meters;
        self
    }

    pub fn width_over_life(mut self, start: f32, end: f32) -> Self {
        self.width = Curve::Linear(start, end);
        self
    }

    pub fn color_over_life(mut self, start: [f32; 4], end: [f32; 4]) -> Self {
        self.color = Gradient::linear(start, end);
        self
    }

    pub fn brightness(mut self, brightness: f32) -> Self {
        self.brightness = brightness;
        self
    }

    pub fn additive(mut self) -> Self {
        self.blend = Blend::Additive;
        self
    }

    pub fn unlit(mut self) -> Self {
        self.lit = false;
        self
    }

    pub fn texture(mut self, path: impl Into<String>, uv_scroll: f32) -> Self {
        self.texture = Some(path.into());
        self.uv_scroll = uv_scroll;
        self
    }

    /// Stands up as a curtain, `width` tall (see `TrailFacing::Upright`).
    pub fn upright(mut self) -> Self {
        self.facing = TrailFacing::Upright;
        self
    }

    /// A low-poly 3D ridge, `width` tall (see `TrailFacing::Ridge`).
    pub fn ridge(mut self) -> Self {
        self.facing = TrailFacing::Ridge;
        self
    }

    pub fn point_velocity(mut self, velocity: Vector3<f32>) -> Self {
        self.point_velocity = velocity;
        self
    }

    pub fn width_curve(mut self, width: Curve) -> Self {
        self.width = width;
        self
    }
}
