use nalgebra::{UnitQuaternion, Vector3};
use rapier3d::prelude::{ColliderHandle, ColliderSet, QueryFilter, QueryPipeline, Ray, RigidBody, RigidBodyHandle, RigidBodySet};

// Below these slip speeds (m/s) friction scales down linearly with speed
// instead of snapping to full strength - true Coulomb friction flips sign
// every step around zero speed and jitters. The cost is a tiny creep
// (~1 cm/s) while holding still against thrust on the brakes.
const ROLLING_SLIP_SPEED: f32 = 0.1;
const LATERAL_SLIP_SPEED: f32 = 0.2;

/// How a ground surface grips a tyre - every value is a fraction of the
/// wheel's normal load (its suspension force), the usual Coulomb-friction
/// shape.
#[derive(Clone, Copy, Debug)]
pub struct GroundSurface {
    /// Always on while rolling. On the runway this is deliberately less than
    /// idle thrust can overcome, same as the real jet: it creeps forward at
    /// idle unless the brakes are held.
    pub rolling_resistance: f32,
    /// Extra longitudinal friction at full brake, on top of rolling resistance.
    pub brake_friction: f32,
    /// Sideways grip - what makes the jet follow where its wheels point
    /// instead of sliding, and what nose-wheel steering pushes against.
    pub lateral_friction: f32,
}

impl GroundSurface {
    /// Dry concrete - an aircraft tyre rolls at ~1-2% of its load there.
    pub const RUNWAY: Self = Self { rolling_resistance: 0.02, brake_friction: 0.5, lateral_friction: 0.8 };

    // TODO: more surfaces, once the world has somewhere they apply. Rough
    // starting points:
    // - GRASS: rolling ~0.05-0.1 (soft, drags more), brake ~0.2-0.3,
    //   lateral ~0.4 - slows down on its own sooner, steers/brakes worse.
    // - WET_RUNWAY: rolling as dry, brake ~0.25, lateral ~0.5.
    // - DIRT/GRAVEL, water (ditching) ...

    /// Which surface the ground collider a wheel is touching is made of.
    ///
    /// TODO: every collider counts as runway for now. To make it real, give
    /// each ground node a surface (e.g. on its `Physics` property, carried
    /// onto its colliders through rapier's `Collider::user_data` when the
    /// physics thread builds them) and read it back here.
    pub fn of_collider(_collider: ColliderHandle, _colliders: &ColliderSet) -> Self {
        Self::RUNWAY
    }
}

/// One suspension raycast's result - see `Wheel::update_wheel`.
pub struct SuspensionHit {
    /// Spring + damper force, straight up, before gear-deploy scaling.
    pub force: Vector3<f32>,
    /// Where the ray starts - the strut's attachment point.
    pub origin: Vector3<f32>,
    /// Where the wheel sits - the ground contact, or the ray's full length
    /// when airborne.
    pub wheel_position: Vector3<f32>,
    /// The collider under the wheel, `None` when airborne.
    pub ground: Option<ColliderHandle>,
}

#[derive(Debug, Clone)]
pub struct WheelData {
    /// Where the wheel touches (the ray's hit, or its far end when
    /// airborne), in the rigidbody's frame.
    pub local_position: Vector3<f32>,
    /// Where its suspension is mounted (the ray's origin), in the
    /// rigidbody's frame - where the wheel tucks up to when retracted.
    pub mount: Vector3<f32>,
    /// This wheel's suspension ray hit ground this tick (within
    /// `max_suspension_length`). Read by the landing-gear state machine to
    /// force the gear back down if it's mid-retraction over the runway.
    pub grounded: bool,
}

pub struct Wheel {
    pub mesh_name: String,
    pub offset: Vector3<f32>,  // Local offset of the wheel relative to the plane
    max_suspension_length: f32,
    pub stiffness: f32,
    pub damping: f32,
    /// Turns with the steering input (the nose wheel) - see `steerable`.
    pub steerable: bool,
    /// Has a brake - see `braked`.
    pub braked: bool,
    /// Steering angle applied last step, radians, positive = right.
    pub steer_angle: f32,
    /// Tyre friction applied last step, world space (N) - zero while
    /// airborne. Kept for the F5 overlay.
    pub last_tyre_force: Vector3<f32>,
}

impl Wheel {
    pub fn new(mesh_name: String, offset: Vector3<f32>, max_suspension_length: f32, stiffness: f32, damping: f32) -> Self {
        Self { mesh_name, offset, max_suspension_length, stiffness, damping, steerable: false, braked: false, steer_angle: 0.0, last_tyre_force: Vector3::zeros() }
    }

    /// Builder: this wheel turns with the steering input.
    pub fn steerable(mut self) -> Self {
        self.steerable = true;
        self
    }

    /// Builder: this wheel has a brake.
    pub fn braked(mut self) -> Self {
        self.braked = true;
        self
    }

    /// World-space direction this wheel rolls in, flattened onto the ground
    /// (world up is taken as the ground normal, same as the suspension
    /// force) - the body's forward axis (+Z local), turned by `steer_angle`.
    /// Right is -X local (this is a right-handed frame, "Left wing" sits at
    /// +X), so a positive angle swings the heading toward -X.
    pub fn heading(&self, rotation: &UnitQuaternion<f32>) -> Option<Vector3<f32>> {
        let local = Vector3::new(-self.steer_angle.sin(), 0.0, self.steer_angle.cos());
        let world = rotation * local;
        Vector3::new(world.x, 0.0, world.z).try_normalize(1e-4)
    }

    /// Friction between the tyre and `surface` at `contact`, in world space:
    /// rolling resistance + `brake` (0..1, only if this wheel is `braked`)
    /// along the heading, and sideways grip across it - both scaled by
    /// `load`, the wheel's current normal force (N).
    pub fn tyre_force(&self, rigidbody: &RigidBody, contact: Vector3<f32>, load: f32, brake: f32, surface: &GroundSurface) -> Vector3<f32> {
        let Some(forward) = self.heading(rigidbody.rotation()) else { return Vector3::zeros() };
        let side = Vector3::y().cross(&forward);

        let velocity = rigidbody.velocity_at_point(&contact.into());
        let rolling_speed = velocity.dot(&forward);
        let sideways_speed = velocity.dot(&side);

        let brake = if self.braked { brake.clamp(0.0, 1.0) } else { 0.0 };
        let rolling_friction = (surface.rolling_resistance + brake * surface.brake_friction) * load;
        let longitudinal = -(rolling_speed / ROLLING_SLIP_SPEED).clamp(-1.0, 1.0) * rolling_friction;
        let lateral = -(sideways_speed / LATERAL_SLIP_SPEED).clamp(-1.0, 1.0) * surface.lateral_friction * load;

        forward * longitudinal + side * lateral
    }

    /// Casts this wheel's suspension ray. `ground` is `None` (and the force
    /// zero) when the ray hit nothing within `max_suspension_length`.
    pub fn update_wheel(&mut self, body: RigidBodyHandle, collider_set: &ColliderSet, rigidbody_set: &RigidBodySet, query_pipeline: &QueryPipeline) -> Option<SuspensionHit> {
        if let Some(rigidbody) = rigidbody_set.get(body) {
            // Origin of the raycast
            let rotation = rigidbody.rotation();
            let suspension_origin = rigidbody.translation() + (rotation * self.offset);
    
            // Direction of the ray (downward in local space)
            let local_ray_direction = -Vector3::y_axis();
            
            // Transforming local ray direction to world space
            let ray_direction = rotation * local_ray_direction;
            
            let max_wheel_position = suspension_origin + (ray_direction.into_inner() * self.max_suspension_length);
            
            // Raycast from the wheel downward to detect the ground
            let ray = Ray::new(suspension_origin.into(), ray_direction.into_inner());
            
            // Exclude all colliders belonging to this physics object
            let filter = QueryFilter::default().exclude_rigid_body(body);

            // Perform raycast
            if let Some((ground, time_of_impact)) = query_pipeline.cast_ray(
                rigidbody_set,
                collider_set,
                &ray,
                self.max_suspension_length,
                true,          // solid: treat colliders as solid
                filter
            ) {
                // Calculate compression based on hit distance
                let compression = 1.0 - (time_of_impact / self.max_suspension_length);
    
                // Calculate spring force (Hooke's law) and damping force
                let spring_force = compression * self.stiffness;
                let damping_force = rigidbody.linvel().y * self.damping;
    
                // Apply total force in the upward direction at the wheel position
                let suspension_force = Vector3::new(0.0, spring_force - damping_force, 0.0);
                let wheel_position = ray.point_at(time_of_impact);

                return Some(SuspensionHit { force: suspension_force, origin: suspension_origin, wheel_position: wheel_position.coords, ground: Some(ground) });
            } else {
                return Some(SuspensionHit { force: Vector3::zeros(), origin: suspension_origin, wheel_position: max_wheel_position, ground: None });
            };
        }

        None
    }
}