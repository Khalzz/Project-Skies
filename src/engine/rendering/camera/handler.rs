use std::collections::HashMap;
use nalgebra::{Matrix4, Point3, Quaternion, Vector3};
use sdl2::rect::Point;
use wgpu::{util::DeviceExt, BindGroup, BindGroupLayout, BindGroupLayoutDescriptor, Buffer, Device, Queue};
use crate::engine::rendering::instance_management::InstanceData;
use crate::engine::utils::lerps::{lerp, lerp_point3, smoothstep};
use crate::transform::Transform;

use super::camera::Camera;
use super::projection::Projection;
use super::uniform::{CameraUniform, NearFarUniform};

// Stands in for "no real camera exists" - `SceneCameras::new` selects this
// instead of a scene starting with no active camera at all. There's no other
// engine-level default camera name any more - "main" (play::scene.rs's own
// choice), "main_menu", "sandbox_free" etc. are all just scene-chosen names
// now, nothing special about any of them at this level. A real, if
// meaningless, `CameraInstance` still sits in `cameras` under this name so
// `active()`'s `.expect()` never has anything to panic on - only
// `has_active_camera()` (and the render loop, via that) treats this specially.
const NO_CAMERA_SENTINEL: &str = "__no_camera";
// A dedicated hidden camera swapped in via select_camera - keeps a
// transition's blended values out of both the source and destination
// camera's own stored settings, so nothing reading those directly ever sees
// a mid-blend value.
const TRANSITION_CAMERA_NAME: &str = "__camera_transition";

// State for an in-progress transition_to(...) - see its doc comment.
struct CameraTransition {
    from: Camera,
    from_fovy: f32,
    to: String,
    elapsed: f32,
    duration: f32,
}

/// What a camera's `look_at` (see `CameraInstance::look_at`) aims at -
/// resolved fresh every frame by `SceneCameras::update`, not just once when
/// it's set, so a `Node` target keeps being tracked as it moves instead of
/// freezing at wherever it was the instant `look_at` was configured.
pub enum LookAtTarget {
    Point(Point3<f32>),
    // A renderizable instance's id (see `Scene::spawn_node`'s `Model`
    // registration, or a `data.ron`-loaded object) - resolved via its live
    // `transform.position` each frame, the same lookup
    // `animation_tracks::CameraTrack::Shot`'s `follow_at`/`look_at` already
    // do for the same reason.
    Node(String),
}

/// A single named camera's own settings - position/orientation (`Camera`) and lens
/// (`Projection`). Cheap to create since it owns no GPU resources of its own; only
/// the active one (see `SceneCameras`) actually drives what gets rendered.
pub struct CameraInstance {
    pub camera: Camera,
    pub projection: Projection,
    // None means "whatever on_spawn/update sets position/yaw/pitch to
    // directly is the final word" - the common case. Some(target) means this
    // camera's orientation is owned by `SceneCameras::update` instead: it
    // overwrites yaw/pitch every frame to keep facing `target`, after
    // whatever else (a Behavior, camera_control, ...) already ran that frame.
    pub look_at: Option<LookAtTarget>,
}

/// The persistent, app-level half of the camera system - the one shared set
/// of GPU resources (uniform/buffer/bind group) that actually feeds the
/// renderer each frame, created once and reused across every scene switch.
/// Lives on `App` as `app.camera_resources`. Doesn't hold any camera data
/// itself (no cameras, no "active" concept) - that's scene-scoped (see
/// `SceneCameras`, on `Scene::cameras`); this just uploads whatever
/// `CameraInstance` it's handed (`update_buffer`) and answers screen-space
/// queries against whatever it last uploaded (`world_to_screen`).
pub struct CameraResources {
    // Remembered so SceneCameras::new/create_camera never need width/height
    // passed in explicitly - every camera's aspect ratio is always just
    // "whatever the screen currently is". Kept current by `resize`.
    width: u32,
    height: u32,
    pub uniform: CameraUniform,
    pub buffer: Buffer,
    pub bind_group_layout: BindGroupLayout,
    pub bind_group: BindGroup,
}

impl CameraResources {
    pub fn new(device: &Device, config: &wgpu::SurfaceConfiguration) -> Self {
        let near_far_uniform = NearFarUniform {
            near: 0.1,
            // Pushed way out so the huge "world_far" backdrop water plane (see
            // play::scene::spawn_world) isn't clipped - reversed-Z keeps
            // near/mid depth precision fine regardless of how far this goes.
            // Kept in sync with the Projection::new call below and water.wgsl's
            // own `const FAR`.
            far: 4_000_000.0,
        };

        let uniform = CameraUniform::new(near_far_uniform);

        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Camera Buffer"),
            contents: bytemuck::cast_slice(&[uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("camera_bind_group_layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera_bind_group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });

        CameraResources {
            width: config.width,
            height: config.height,
            uniform,
            buffer,
            bind_group_layout,
            bind_group,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    // Only remembers the new size for future SceneCameras::new/create_camera
    // calls - resizing every already-registered camera's own aspect is
    // scene-scoped now (see SceneCameras::resize), since the cameras
    // themselves live there, not here.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }

    // Recomputes the view_proj uniform from `active` and uploads it - called
    // once per frame from App, replacing what used to be two lines inlined
    // at every call site.
    pub fn update_buffer(&mut self, queue: &Queue, active: &CameraInstance) {
        self.uniform.update_view_proj(&active.camera, &active.projection);
        queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&[self.uniform]));
    }

    pub fn world_to_screen(&self, active: &CameraInstance, pos_world: Point3<f32>, screen_width: u32, screen_height: u32) -> Option<Point> {
        // view_proj now assumes the camera sits at the origin, so every
        // position fed into it must first be made camera-relative.
        let camera_to_point = pos_world - active.camera.position();
        let forward = active.camera.calc_forward_direction();

        if camera_to_point.dot(&forward) < 0.0 {
            return None;
        }

        let view_proj = Matrix4::from(self.uniform.view_proj);
        let pos_homogeneous = view_proj * Point3::from(camera_to_point).to_homogeneous();

        if pos_homogeneous.w != 0.0 {
            let ndc = pos_homogeneous.xyz() / pos_homogeneous.w;

            if ndc.x.abs() <= 1.0 && ndc.y.abs() <= 1.0 && ndc.z >= 0.0 && ndc.z <= 1.0 {
                let x = ((ndc.x + 1.0) * 0.5) * screen_width as f32;
                let y = ((1.0 - (ndc.y + 1.0) * 0.5)) * screen_height as f32;

                Some(Point::new(x as i32, y as i32))
            } else {
                None
            }
        } else {
            None
        }
    }
}

/// The scene-scoped half of the camera system - every camera the currently
/// active scene has registered, which one's active, and any in-progress
/// blend between two of them. Lives on `Scene` as `scene.cameras`, so it's
/// dropped and rebuilt fresh on every scene switch the same way
/// `SceneContent` is - a camera a previous scene registered
/// (`game::camera::camera::Camera` behaviors, a scene's own direct
/// `Scene::create_camera` calls, ...) never lingers into the next one.
///
/// This is the "base" of the camera system - `Camera` (see `camera.rs`) only
/// defines a camera's own settings and functions; this is what actually
/// creates one, translating a `Transform` into that representation (see
/// `create_camera`), tracks every registered camera by name, and drives what
/// `CameraResources::update_buffer` reads each frame.
pub struct SceneCameras {
    cameras: HashMap<String, CameraInstance>,
    active: String,
    // Copied from CameraResources at construction (see `Scene::new`) - kept
    // here too so create_camera never needs width/height passed in
    // explicitly. Stays in sync with CameraResources via `resize`.
    width: u32,
    height: u32,
    // Some(...) while transition_to(...) is blending - see its doc comment and
    // update_transition, which advances/clears this every frame.
    transition: Option<CameraTransition>,
}

impl SceneCameras {
    pub fn new(width: u32, height: u32) -> Self {
        let mut cameras = SceneCameras {
            cameras: HashMap::new(),
            active: NO_CAMERA_SENTINEL.to_owned(),
            width,
            height,
            transition: None,
        };

        // Placeholder so active()'s .expect() has something to find before
        // this scene creates its own first camera - never meant to be seen,
        // the render loop skips drawing while this is active.
        cameras.create_camera(NO_CAMERA_SENTINEL, Transform::new(Vector3::new(0.0, 0.0, 0.0), Quaternion::identity(), Vector3::new(1.0, 1.0, 1.0)), 45.0);
        cameras
    }

    /// Registers (or replaces) a named camera from a `Transform` (the same
    /// type `data.ron`-sourced objects use) - held on `Camera` as-is now, no
    /// decomposition happens here; yaw/pitch are derived lazily (see
    /// `Camera::yaw`/`pitch`) only when something actually needs them.
    /// `transform.scale` is ignored entirely (meaningless for a camera).
    /// `fovy` is the only lens setting exposed here since it's the one that
    /// actually varies per camera in practice - near/far default to the
    /// usual values but remain plain public fields on the returned
    /// instance's `.projection` if a caller ever needs to override them.
    /// Width/height are deliberately not parameters: aspect ratio always
    /// comes from this scene's own tracked screen size, kept current by
    /// `resize`.
    ///
    /// If nothing else is currently active (see `NO_CAMERA_SENTINEL`), this
    /// camera becomes active automatically - a scene with exactly one camera
    /// never needs its own `select_camera` call. A second `create_camera`
    /// doesn't repeat that - `active` is real by then, so choosing between
    /// multiple cameras still needs an explicit `select_camera`.
    pub fn create_camera(&mut self, name: impl Into<String>, transform: Transform, fovy: f32) -> &mut CameraInstance {
        let camera = Camera::new(transform);
        // Far plane kept in sync with CameraResources::new's near_far_uniform
        // and water.wgsl's `const FAR` - see the comment there.
        let projection = Projection::new(self.width, self.height, fovy, 0.1, 4_000_000.0);

        let name = name.into();
        if self.active == NO_CAMERA_SENTINEL {
            self.active = name.clone();
        }
        self.cameras.insert(name.clone(), CameraInstance { camera, projection, look_at: None });
        self.cameras.get_mut(&name).unwrap()
    }

    /// Whether a real, scene-created camera is currently active - `false`
    /// until something calls `create_camera`. The render loop uses this to
    /// fall back to a black screen instead of drawing from a meaningless
    /// placeholder.
    pub fn has_active_camera(&self) -> bool {
        self.active != NO_CAMERA_SENTINEL
    }

    /// Switches the active camera. Returns false (no-op) if `name` isn't
    /// registered, rather than panicking - callers can decide whether that's
    /// worth logging.
    pub fn select_camera(&mut self, name: &str) -> bool {
        if self.cameras.contains_key(name) {
            // An instant cut here should really be instant - cancel any
            // in-progress transition_to(...) so update_transition doesn't
            // snap the view back to the transition camera next frame.
            self.transition = None;
            self.active = name.to_owned();
            true
        } else {
            false
        }
    }

    /// Smoothly blends the active view from wherever it is right now to camera
    /// `name` over `duration` seconds, instead of `select_camera`'s instant cut -
    /// opt-in, for cases like switching between two fixed viewpoints where a hard
    /// cut would be jarring. Not meant to run continuously - call it once when you
    /// want the switch to start; `update_transition` (already called every frame
    /// from `App::render`) advances it and hands off to the real target camera via
    /// `select_camera` once it finishes. Snapshots the *current* active camera's
    /// position/yaw/pitch/fovy as the starting point, so it composes fine with a
    /// camera that itself moves every frame - it just blends from wherever that
    /// happened to be this frame. Returns `false` (no-op, same as
    /// `select_camera`) if `name` isn't registered.
    pub fn transition_to(&mut self, name: &str, duration: f32) -> bool {
        if !self.cameras.contains_key(name) {
            return false;
        }
        let active = self.active();
        self.transition = Some(CameraTransition {
            from: active.camera,
            from_fovy: active.projection.fovy,
            to: name.to_owned(),
            elapsed: 0.0,
            duration: duration.max(0.0001),
        });
        true
    }

    /// Advances any in-progress `transition_to(...)` by `delta_time` - a cheap
    /// no-op if none is running. Blends into `TRANSITION_CAMERA_NAME` (created
    /// lazily) rather than overwriting either the source or destination camera's own stored
    /// settings, so `get`/`get_mut` on either always reflect their real values
    /// regardless of whether a transition happens to be running. `smoothstep`
    /// gives the blend an ease-in/ease-out instead of a constant-speed linear pan.
    pub fn update_transition(&mut self, delta_time: f32) {
        let Some(transition) = &mut self.transition else { return };
        transition.elapsed += delta_time;
        let t = smoothstep(transition.elapsed / transition.duration);
        let done = transition.elapsed >= transition.duration;

        let Some(target) = self.cameras.get(&transition.to) else {
            // Target camera got removed mid-transition - bail out rather than
            // keep blending toward something that no longer exists.
            self.transition = None;
            return;
        };
        let position = lerp_point3(transition.from.position(), target.camera.position(), t);
        let yaw = lerp(transition.from.yaw(), target.camera.yaw(), t);
        let pitch = lerp(transition.from.pitch(), target.camera.pitch(), t);
        let fovy = lerp(transition.from_fovy, target.projection.fovy, t);
        let to = transition.to.clone();

        if self.get(TRANSITION_CAMERA_NAME).is_none() {
            // Rotation here is a throwaway identity - overwritten field-by-field
            // right below with yaw/pitch already lerped as radians, no reason to
            // route those back through a Transform/degrees round-trip.
            self.create_camera(TRANSITION_CAMERA_NAME, Transform::new(position.coords, Quaternion::identity(), Vector3::new(1.0, 1.0, 1.0)), fovy);
        }
        // yaw/pitch computed above are already radians (same units Camera's
        // own accessors always work in) - set directly rather than going
        // through create_camera's degrees-in constructor again and
        // double-converting.
        if let Some(transition_camera) = self.cameras.get_mut(TRANSITION_CAMERA_NAME) {
            transition_camera.camera.set_position(position);
            transition_camera.camera.set_yaw_pitch(yaw, pitch);
            transition_camera.projection.fovy = fovy;
        }
        self.active = TRANSITION_CAMERA_NAME.to_owned();

        if done {
            self.transition = None;
            self.select_camera(&to);
        }
    }

    pub fn active_name(&self) -> &str {
        &self.active
    }

    pub fn active(&self) -> &CameraInstance {
        self.cameras.get(&self.active).expect("active camera must always exist")
    }

    pub fn active_mut(&mut self) -> &mut CameraInstance {
        self.cameras.get_mut(&self.active).expect("active camera must always exist")
    }

    pub fn get(&self, name: &str) -> Option<&CameraInstance> {
        self.cameras.get(name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut CameraInstance> {
        self.cameras.get_mut(name)
    }

    // Every registered camera's aspect stays in sync with the screen, not just
    // the active one - so selecting a different camera later never shows a
    // stale aspect ratio for a frame.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
        for instance in self.cameras.values_mut() {
            instance.projection.resize(width, height);
        }
    }

    /// Sets (or clears, via `None`) `name`'s `look_at` - see
    /// `CameraInstance::look_at`'s own doc comment. Returns `false` (no-op),
    /// same convention as `select_camera`, if `name` isn't registered.
    pub fn set_look_at(&mut self, name: &str, target: Option<LookAtTarget>) -> bool {
        if let Some(instance) = self.cameras.get_mut(name) {
            instance.look_at = target;
            true
        } else {
            false
        }
    }

    /// Re-aims every camera with a `look_at` configured at its current
    /// target, every frame - called once per rendered frame from `App::run`,
    /// after anything else (a `Behavior`, `camera_control`, ...) has already
    /// had its turn moving cameras around this frame, so `look_at` always
    /// gets the final word on orientation for the cameras that opted into
    /// it. `renderizable_instances` is `Scene::content`'s own map - passed in
    /// rather than reaching for it directly so this module stays independent
    /// of `engine::scene_manager` (see this camera system's own design - it
    /// isn't part of the node tree).
    pub fn update(&mut self, renderizable_instances: &HashMap<String, InstanceData>) {
        for instance in self.cameras.values_mut() {
            let Some(target) = &instance.look_at else { continue };
            let target_point = match target {
                LookAtTarget::Point(point) => Some(*point),
                LookAtTarget::Node(id) => renderizable_instances.get(id).map(|renderizable| Point3::from(renderizable.instance.transform.position)),
            };
            if let Some(target_point) = target_point {
                instance.camera.look_at(target_point);
            }
        }
    }
}
