use nalgebra::{UnitQuaternion, Vector3};

use crate::app::App;
use crate::engine::game_nodes::game_object::Lighting;
use crate::engine::rendering::camera::camera::yaw_pitch_rotation;
use crate::engine::rendering::enviroment::environment::Environment;
use crate::engine::scene_manager::node::Node;
use crate::engine::scene_manager::properties::{Model, Transform3D};
use crate::engine::scene_manager::scene::{FrameContext, Scene, SceneBehaviour};
use crate::game::camera::camera::{Camera, CameraConfig, Fov, Speed};
use crate::resources;
use crate::transform::Transform;

/// Testbed for the upcoming "follow point" camera system. For now it just
/// stands up the world (island + water + sky) with the MQ-9 drone parked in
/// the air and the existing free-fly `Camera` behavior
/// (`game::camera::camera::Camera` with `free: true`) so the scene can be
/// flown around by hand while the follow system is built on top.
///
/// Everything here is static - no physics thread (see the `SceneBehaviour`
/// impl's default `fixed_update`).
pub struct GameLogic;

impl GameLogic {
    pub fn new(scene: &mut Scene, app: &mut App, environment: Environment) -> Self {
        resources::apply_environment(scene, app, environment);

        // Free-fly camera wants raw mouse deltas for mouse-look.
        app.window_manager.context.mouse().set_relative_mouse_mode(true);

        Self::spawn_world(scene, app);

        Self
    }

    fn spawn_world(scene: &mut Scene, app: &mut App) {
        // Sun / key light - same far-away-mesh trick main_menu and play use
        // (lighting is carried on a renderable instance's metadata, so it
        // needs a node with a model; parked 1,000km up so it's never seen).
        scene
            .spawn_node(
                app,
                Node::new("sun")
                    .add_property(Transform3D {
                        position: Vector3::new(1000.0, 1_000_000.0, 1000.0),
                        rotation: UnitQuaternion::identity(),
                        scale: Vector3::new(1.0, 1.0, 1.0),
                    })
                    .add_property(Model { model_ref: "F16".to_owned() }),
            )
            .expect("follow_test should only spawn 'sun' once");
        if let Some(sun) = scene.content.renderizable_instances.get_mut("sun") {
            sun.instance.metadata.lighting =
                Some(Lighting { intensity: 1.0, color: Vector3::new(0.7, 0.7, 0.8) });
        }

        // The island at the origin (same model/scale/height as play's
        // "runway" island).
        scene
            .spawn_node(
                app,
                Node::new("island")
                    .add_property(Transform3D {
                        position: Vector3::new(0.0, 100.0, 0.0),
                        rotation: UnitQuaternion::identity(),
                        scale: Vector3::new(60.0, 60.0, 60.0),
                    })
                    .add_property(Model { model_ref: "Runway".to_owned() }),
            )
            .expect("follow_test should only spawn 'island' once");

        // Distant landmass for the horizon (matches play's "ground").
        scene
            .spawn_node(
                app,
                Node::new("ground")
                    .add_property(Transform3D {
                        position: Vector3::new(0.0, 0.0, 50_000.0),
                        rotation: UnitQuaternion::identity(),
                        scale: Vector3::new(30_000.0, 30_000.0, 30_000.0),
                    })
                    .add_property(Model { model_ref: "Ground".to_owned() }),
            )
            .expect("follow_test should only spawn 'ground' once");

        // Water - near detailed plane + the huge forced-calm far plane, same
        // pair play::scene::spawn_world sets up. Both are recentred under the
        // camera every frame in `update` so a fast free-fly can't outrun
        // them.
        scene
            .spawn_node(
                app,
                Node::new("world")
                    .add_property(Transform3D {
                        position: Vector3::new(0.0, 0.0, 0.0),
                        rotation: UnitQuaternion::identity(),
                        scale: Vector3::new(2_000.0, 1.0, 2_000.0),
                    })
                    .add_property(Model { model_ref: "WaterPlane".to_owned() }),
            )
            .expect("follow_test should only spawn 'world' once");
        scene
            .spawn_node(
                app,
                Node::new("world_far")
                    .add_property(Transform3D {
                        position: Vector3::new(0.0, -4.0, 0.0),
                        rotation: UnitQuaternion::identity(),
                        scale: Vector3::new(3_000_000.0, 0.03, 3_000_000.0),
                    })
                    .add_property(Model { model_ref: "WaterPlaneFar".to_owned() }),
            )
            .expect("follow_test should only spawn 'world_far' once");

        // The MQ-9 drone - the node the follow camera will track. Static for
        // now, scaled 11x on every axis as requested.
        scene
            .spawn_node(
                app,
                Node::new("mq9")
                    .add_property(Transform3D {
                        position: Vector3::new(0.0, 180.0, 0.0),
                        rotation: UnitQuaternion::identity(),
                        scale: Vector3::new(11.0, 11.0, 11.0),
                    })
                    .add_property(Model { model_ref: "MQ9".to_owned() }),
            )
            .expect("follow_test should only spawn 'mq9' once");

        // Camera - the reusable `Camera` behavior with `free: true`. Starts
        // as free-fly (mouse-look + WASD / Space / Left Ctrl, Left Shift to
        // sprint, scroll to zoom); F1 toggles it to orbit-a-target mode
        // centred on the MQ-9 (mouse orbits, scroll reels the boom in/out) -
        // same idea as play's own free camera. The pivot is fed to the
        // behavior every frame by `update` below.
        scene
            .spawn_node(
                app,
                Node::new("free_camera").add_behavior(Camera::new(CameraConfig {
                    camera_name: "free_camera".to_owned(),
                    transform: Transform::new(
                        Vector3::new(0.0, 210.0, 140.0),
                        yaw_pitch_rotation(-90.0, -12.0),
                        Vector3::new(1.0, 1.0, 1.0),
                    ),
                    speed: Speed { base_speed: 60.0, sprint_multiplier: 5.0 },
                    fov: Fov { max: 75.0, min: 30.0, speed: 2.0 },
                    look_sensitivity_scale: 1.0,
                    max_pitch_deg: 89.0,
                    free: true,
                    orbit_toggle_action: Some("follow_cam_toggle".to_owned()),
                })),
            )
            .expect("follow_test should only spawn 'free_camera' once");
    }

    fn update(&mut self, scene: &mut Scene) {
        // Feed the camera the MQ-9's position so its orbit mode (F1) has a
        // pivot to swing around - has to run before the "free_camera" node's
        // own Behavior tick (SceneBehaviour::update runs first, see
        // App::run), same ordering play relies on for its follow cam.
        if let Some(mq9) = scene.content.renderizable_instances.get("mq9") {
            let position = mq9.instance.transform.position;
            if let Some(camera) = scene
                .content
                .nodes
                .get_mut("free_camera")
                .and_then(|node| node.get_behavior_mut::<Camera>())
            {
                camera.set_orbit_target(position);
            }
        }

        // Keep both water planes centred on the camera's XZ - the wave field
        // is analytic (position in, height out), so sliding the mesh causes
        // no shimmer, and it means the water never runs out from under a
        // fast free-fly. Same approach as play::scene::update_water_plane.
        let camera_xz = scene.cameras.active().camera.position().coords;
        for name in ["world", "world_far"] {
            if let Some(water) = scene.content.renderizable_instances.get_mut(name) {
                water.instance.transform.position.x = camera_xz.x;
                water.instance.transform.position.z = camera_xz.z;
            }
        }
    }
}

impl SceneBehaviour for GameLogic {
    fn update(&mut self, scene: &mut Scene, _app: &mut App, _ctx: &mut FrameContext) {
        self.update(scene);
    }
}
