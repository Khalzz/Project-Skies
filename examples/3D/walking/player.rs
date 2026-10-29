use nalgebra::{UnitQuaternion, Vector3};
use pankarta_software::game_nodes::game_object::{Collider, ColliderShape, Physics, RigidBodyData};
use pankarta_software::physics::PhysicsWorld;
use pankarta_software::prelude::*;

const RADIANS_PER_COUNT: f32 = 0.0004;

const PLAYER_RADIUS: f32 = 0.35;
const PLAYER_HALF_HEIGHT: f32 = 0.9;
const EYE_HEIGHT: f32 = 0.7;

const PLAYER_MASS: f32 = 80.0;

struct Camera {
    pitch: f32,
    yaw: f32,
    /// The field of view the scroll wheel asks for (degrees) - the lens eases
    /// toward it. None until the first frame, which takes the lens's own.
    target_fov: Option<f32>,
    /// Seconds the zoom takes to get most of the way to `target_fov`.
    zoom_smoothing: f32
}

struct Movement {
    spawn: Vector3<f32>,
    walk_speed: f32,
    run_speed: f32,
    jump_speed: f32,
    fall_gravity: f32
}

pub struct Player {
    camera: Camera,
    movement: Movement
}

impl Player {
    pub fn new(spawn: Vector3<f32>) -> Player {
        Player {
            camera: Camera {
                pitch: 0.0,
                yaw: 0.0,
                target_fov: None,
                zoom_smoothing: 0.08
            },
            movement: Movement {
                spawn,
                walk_speed: 8.0,
                run_speed: 15.0,
                jump_speed: 7.0,
                fall_gravity: 1.5
            }
        }
    } 
}

impl Behavior for Player {
    fn on_spawn(&mut self, node: &mut Node, scene: &mut SceneAccess, _app: &mut App) {
        node.set_property(Transform3D { position: self.movement.spawn, ..Default::default() });
        node.set_property(Physics {
            rigidbody: RigidBodyData { lock_rotations: true, ccd: true, ..RigidBodyData::dynamic(PLAYER_MASS) },
            colliders: vec![
                Collider::new(ColliderShape::Capsule { half_height: PLAYER_HALF_HEIGHT - PLAYER_RADIUS, radius: PLAYER_RADIUS })
                    .friction(0.0),
            ],
        });

        let camera = Node::new("camera")
            .with_property(Transform3D::default())
            .with_property(CameraLens::new(75.0));

        scene.spawn(camera); 
    }

    fn update(&mut self, node: &mut Node, scene: &mut SceneAccess, app: &mut App) {

        // For enabling the cursor (not gameplay related)
        if input::is_just_pressed(InputSource::key(Key::Escape)) {
            app.window.set_cursor_locked(false);
        } else if input::mouse_left_button_just_pressed() {
            app.window.set_cursor_locked(true);
        }

        if app.window.is_cursor_locked() {
            let look = input::mouse_look() * RADIANS_PER_COUNT;
            self.camera.yaw -= look.x;
            self.camera.pitch = (self.camera.pitch + look.y).clamp(-1.5, 1.5);
        }

        let heading = UnitQuaternion::from_axis_angle(&Vector3::y_axis(), self.camera.yaw);

        let Some(position) = node.get_property::<Transform3D>().map(|transform| transform.position) else { return };
        let Some(camera) = scene.get_node_mut("camera") else { return };
        if let Some(transform) = camera.get_property_mut::<Transform3D>() {
            transform.position = position + Vector3::y() * EYE_HEIGHT;
            transform.rotation = heading * UnitQuaternion::from_axis_angle(&Vector3::x_axis(), self.camera.pitch);
        }

        if let Some(lens) = camera.get_property_mut::<CameraLens>() {
            let target = self.camera.target_fov.get_or_insert(lens.fovy);
            *target = (*target - input::mouse_scroll_y() * 3.0).clamp(15.0, 110.0);
            // Eased the same at any frame rate.
            let ease = 1.0 - (-app.time.unscaled_delta_time / self.camera.zoom_smoothing.max(0.0001)).exp();
            lens.fovy += (*target - lens.fovy) * ease;
        }
    }

    fn fixed_update(&mut self, node: &mut Node, _scene: &mut SceneAccess, app: &mut App) {

        let walk = input::get_vector("left", "right", "forward", "back").normalize();
        let speed = if input::is_pressed("run") { self.movement.run_speed } else { self.movement.walk_speed };

        let grounded = self::on_ground(&app.physics, &node.id);

        let heading = UnitQuaternion::from_axis_angle(&Vector3::y_axis(), self.camera.yaw);
        let forward = heading * Vector3::z();
        let right = forward.cross(&Vector3::y());

        if let Some(body) = app.physics.body_mut(&node.id) {       
            let mut velocity = *body.linvel();

            // Movement logic
            let movement = (forward * -walk.y + right * walk.x) * speed;

            velocity.x = movement.x;
            velocity.z = movement.z;


            // Jump logic
            if input::is_just_pressed("jump") && grounded {
                velocity.y = self.movement.jump_speed;
            }

            let falling = velocity.y < 0.0;
            body.set_gravity_scale(if falling { self.movement.fall_gravity } else { 1.0 }, true);

            body.set_linvel(velocity, true);   // true wakes the body if it was asleep
        }
    }
}

const GROUND_MARGIN: f32 = 0.08;

pub fn on_ground(physics: &PhysicsWorld, id: &str) -> bool {
    let Some(body) = physics.body(id) else { return false };
    let center = *body.translation();
    let reach = PLAYER_HALF_HEIGHT + GROUND_MARGIN;
    let edge = PLAYER_RADIUS * 0.7;
    [Vector3::zeros(), Vector3::x() * edge, -Vector3::x() * edge, Vector3::z() * edge, -Vector3::z() * edge].iter().any(|offset| {
        physics.cast_ray(center + offset, -Vector3::y(), reach, Some(id)).is_some()
    })
}