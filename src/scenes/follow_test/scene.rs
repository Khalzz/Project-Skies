use nalgebra::{UnitQuaternion, Vector3};

use pankarta_software::prelude::*;

use crate::camera::camera::{looking_along, Camera, CameraConfig, Fov, Speed};
use crate::prefabs::ocean::{self, Ocean};

/// Testbed for the upcoming "follow point" camera system. For now it just
/// stands up the world (island + water + sky) with the MQ-9 drone parked in
/// the air and the free-fly `Camera` behavior
/// (`crate::camera::camera::Camera` with `free: true`) so the scene can be
/// flown around by hand while the follow system is built on top.
///
/// Everything here is static - no physics.
pub struct GameLogic {
    /// The sea.
    ocean: Ocean,
}

/// Where the sun is, seen from the world's origin - 40 degrees above the
/// horizon. Only its direction matters.
pub const SUN_DIRECTION: Vector3<f32> = Vector3::new(445_200.0, 642_800.0, 623_300.0);

/// The sun: a directional light shining from `SUN_DIRECTION`.
pub fn sun_node(color: [f32; 3]) -> Node {
    Node::new("sun")
        .with_property(Transform3D::facing(-SUN_DIRECTION))
        .with_property(Light::directional(color))
}

impl GameLogic {
    pub fn new(scene: &mut Scene, app: &mut App, environment: Environment) -> Self {
        app.renderer.set_environment(environment);
        Self::spawn_world(scene, app);
        Self { ocean: Ocean::default() }
    }

    fn spawn_world(scene: &mut Scene, app: &mut App) {
        let at = |position: Vector3<f32>, scale: f32| Transform3D { position, rotation: UnitQuaternion::identity(), scale: Vector3::new(scale, scale, scale) };
        let nodes = [
            sun_node([0.7, 0.7, 0.8]),
            // The island at the origin (same model/scale/height as play's
            // "runway" island).
            Node::new("island").with_property(at(Vector3::new(0.0, 100.0, 0.0), 60.0)).with_property(Model::file("models/Runway/Runway.glb").double_sided(DoubleSidedMeshes::All)),
            // Distant landmass for the horizon (matches play's "ground").
            Node::new("ground").with_property(at(Vector3::new(0.0, 0.0, 50_000.0), 30_000.0)).with_property(Model::file("models/ground/ground.glb")),
            // The sea, recentred under the camera every frame (Ocean::update) so
            // a fast free-fly can't outrun it.
            Node::new("world").with_property(at(Vector3::zeros(), 1.0)).with_property(ocean::surface()),
            // The MQ-9 drone - the node the follow camera will track.
            Node::new("mq9").with_property(at(Vector3::new(0.0, 180.0, 0.0), 11.0)).with_property(Model::file("models/MQ-9/mq-9.glb")),
            // Camera - the reusable `Camera` behavior with `free: true`.
            // Starts as free-fly (mouse-look + WASD / Space / Left Ctrl, Left
            // Shift to sprint, scroll to zoom); "follow_cam_toggle" (no key
            // bound) toggles it to orbit-a-target mode centred on the MQ-9.
            // The pivot is fed to the behavior every frame by `update`.
            Camera::camera_node(CameraConfig {
                camera_name: "free_camera".to_owned(),
                transform: Transform3D {
                    position: Vector3::new(0.0, 210.0, 140.0),
                    rotation: looking_along(Vector3::new(0.0, -12.0f32.to_radians().tan(), -1.0)),
                    ..Default::default()
                },
                speed: Speed { base_speed: 60.0, sprint_multiplier: 5.0 },
                fov: Fov { max: 75.0, min: 30.0, speed: 2.0 },
                look_sensitivity_scale: 1.0,
                max_pitch_deg: 89.0,
                free: true,
                orbit_toggle_action: Some("follow_cam_toggle".to_owned()),
            }),
        ];
        for node in nodes {
            if let Err(error) = scene.spawn_node(app, node) {
                eprintln!("{error}");
            }
        }
    }
}

impl SceneBehaviour for GameLogic {
    fn update(&mut self, scene: &mut Scene, _app: &mut App, _ctx: &mut FrameContext) {
        // Feed the camera the MQ-9's position so its orbit mode has a pivot
        // to swing around - before the camera node's own behavior runs (the
        // scene's update runs first).
        if let Some(position) = scene.get_node("mq9").and_then(|node| node.transform_3d()).map(|transform| transform.position) {
            if let Some(camera) = scene.get_node_mut("free_camera").and_then(|node| node.get_behavior_mut::<Camera>()) {
                camera.set_orbit_target(position);
            }
        }
    }

    fn after_cameras(&mut self, scene: &mut Scene, app: &mut App) {
        // The sea's waves, around where the camera is drawn from (and the
        // sea kept centred under it).
        self.ocean.update(scene, "world", app.time.delta_time);
    }
}
