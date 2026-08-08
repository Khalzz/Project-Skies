use std::collections::HashMap;
use nalgebra::{Matrix4, Perspective3, Point3, UnitQuaternion, Vector3};
use sdl2::rect::Point;
use wgpu::{util::DeviceExt, BindGroup, BindGroupLayout, BindGroupLayoutDescriptor, Buffer, Device, Queue};
use std::f32::consts::FRAC_PI_2;

// Maps OpenGL's [-1, 1] NDC z-range to wgpu's [0, 1] range, and reverses it
// (near -> 1, far -> 0) so depth precision concentrates on distant geometry
// instead of being wasted right in front of the near plane. Must be paired
// with CompareFunction::Greater and a depth clear value of 0.0.
#[rustfmt::skip]
pub const OPENGL_TO_WGPU_MATRIX: Matrix4<f32> = Matrix4::new(
    1.0, 0.0, 0.0, 0.0,
    0.0, 1.0, 0.0, 0.0,
    0.0, 0.0, -0.5, 0.5,
    0.0, 0.0, 0.0, 1.0,
);

const SAFE_FRAC_PI_2: f32 = FRAC_PI_2 - 0.0001;

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct NearFarUniform {
    pub near: f32,
    pub far: f32,
}

const DEFAULT_CAMERA_NAME: &str = "main";

/// A single named camera's own settings - position/orientation (`Camera`) and lens
/// (`Projection`). Cheap to create since it owns no GPU resources of its own; only
/// the active one (see `CameraHandler`) actually drives what gets rendered.
pub struct CameraInstance {
    pub camera: Camera,
    pub projection: Projection,
}

/// Owns every camera the game has registered plus the one shared set of GPU
/// resources (uniform/buffer/bind group) that actually feeds the renderer each
/// frame - only the active camera's settings ever get uploaded (see
/// `update_buffer`), the same way engines like Unity only ever render from one
/// active camera at a time even though many can exist in a scene.
pub struct CameraHandler {
    cameras: HashMap<String, CameraInstance>,
    active: String,
    // Remembered so create_camera never needs width/height passed in - every
    // camera's aspect ratio is always just "whatever the screen currently is".
    width: u32,
    height: u32,
    pub uniform: CameraUniform,
    pub buffer: Buffer,
    pub bind_group_layout: BindGroupLayout,
    pub bind_group: BindGroup,
}

impl CameraHandler {
    pub fn new(device: &Device, config: &wgpu::SurfaceConfiguration) -> Self {
        let near_far_uniform = NearFarUniform {
            near: 0.1,
            far: 100000.0,
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

        let mut handler = CameraHandler {
            cameras: HashMap::new(),
            active: DEFAULT_CAMERA_NAME.to_owned(),
            width: config.width,
            height: config.height,
            uniform,
            buffer,
            bind_group_layout,
            bind_group,
        };

        // Same defaults CameraRenderizable used to hardcode - existing code that
        // never creates extra cameras sees no behavior change.
        handler.create_camera(DEFAULT_CAMERA_NAME, Point3::new(0.0, 0.0, 0.0), -90.0, -20.0, 45.0);
        handler
    }

    /// Registers (or replaces) a named camera. `fovy` is the only lens setting
    /// exposed here since it's the one that actually varies per camera in
    /// practice - near/far default to the handler's usual values but remain
    /// plain public fields on the returned instance's `.projection` if a caller
    /// ever needs to override them. Width/height are deliberately not
    /// parameters: aspect ratio always comes from the handler's own tracked
    /// screen size, kept current by `resize`.
    pub fn create_camera(&mut self, name: impl Into<String>, position: Point3<f32>, yaw_deg: f32, pitch_deg: f32, fovy: f32) -> &mut CameraInstance {
        let camera = Camera::new(position, yaw_deg.to_radians(), pitch_deg.to_radians());
        let projection = Projection::new(self.width, self.height, fovy, 0.1, 100000.0);

        let name = name.into();
        self.cameras.insert(name.clone(), CameraInstance { camera, projection });
        self.cameras.get_mut(&name).unwrap()
    }

    /// Switches the active camera. Returns false (no-op) if `name` isn't
    /// registered, rather than panicking - callers can decide whether that's
    /// worth logging.
    pub fn select_camera(&mut self, name: &str) -> bool {
        if self.cameras.contains_key(name) {
            self.active = name.to_owned();
            true
        } else {
            false
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

    // Recomputes the view_proj uniform from the active camera and uploads it -
    // called once per frame from App, replacing what used to be two lines
    // inlined at every call site.
    pub fn update_buffer(&mut self, queue: &Queue) {
        let active = self.cameras.get(&self.active).expect("active camera must always exist");
        self.uniform.update_view_proj(&active.camera, &active.projection);
        queue.write_buffer(&self.buffer, 0, bytemuck::cast_slice(&[self.uniform]));
    }

    pub fn world_to_screen(&self, pos_world: Point3<f32>, screen_width: u32, screen_height: u32) -> Option<Point> {
        // view_proj now assumes the camera sits at the origin, so every
        // position fed into it must first be made camera-relative.
        let active = self.active();
        let camera_to_point = pos_world - active.camera.position;
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

#[derive(Copy, Clone, Debug)]
pub struct Camera {
    pub position: Point3<f32>,
    pub yaw: f32,
    pub pitch: f32,
    pub look_at: Option<Point3<f32>>,
    pub up: Vector3<f32>,
    pub rotation_modifier: UnitQuaternion<f32>,
}

impl Camera {
    pub fn new(position: Point3<f32>, yaw: f32, pitch: f32) -> Self {
        Self {
            position,
            yaw,
            pitch,
            look_at: None,
            up: Vector3::y_axis().into_inner(),
            rotation_modifier: UnitQuaternion::identity(),
        }
    }

    pub fn look_at(&mut self, target: Point3<f32>) {
        // Compute the direction vector from the camera's position to the target.
        let direction = target - self.position;
        let normalized_dir = direction.normalize();
    
        // Calculate pitch and yaw using asin and atan2 for the direction vector.
        self.pitch = normalized_dir.y.asin();
        self.yaw = normalized_dir.z.atan2(normalized_dir.x);
    }

    pub fn calc_forward_direction(&self) -> Vector3<f32> {
        let (sin_pitch, cos_pitch) = self.pitch.sin_cos();
        let (sin_yaw, cos_yaw) = self.yaw.sin_cos();
        Vector3::new(cos_pitch * cos_yaw, sin_pitch, cos_pitch * sin_yaw).normalize()
    }

    // Builds a view matrix as if the camera were always at the world origin.
    // Object positions are made relative to the camera before being uploaded
    // (see Transform::to_raw), so the GPU never has to deal with the large
    // absolute world coordinates that cause jitter/z-fighting far from (0,0,0).
    pub fn calc_matrix(&self) -> Matrix4<f32> {
        let direction = self.calc_forward_direction();
        let modified_direction = self.rotation_modifier * direction;
        let modified_up = self.rotation_modifier * self.up;

        let origin = Point3::origin();
        Matrix4::look_at_rh(&origin, &(origin + modified_direction), &modified_up)
    }
}

// Projection struct using nalgebra's Perspective3 for perspective projection calculations.
pub struct Projection {
    aspect: f32,
    pub fovy: f32,
    pub znear: f32,
    zfar: f32,
}

impl Projection {
    pub fn new(width: u32, height: u32, fovy: f32, znear: f32, zfar: f32) -> Self {
        Self {
            aspect: width as f32 / height as f32,
            fovy,
            znear,
            zfar,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.aspect = width as f32 / height as f32;
    }

    pub fn calc_matrix(&self) -> Matrix4<f32> {
        OPENGL_TO_WGPU_MATRIX * Perspective3::new(self.aspect, self.fovy.to_radians(), self.znear, self.zfar).to_homogeneous()
    }
}

// the cameraUniform will get us the positional matrix of the camera
#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CameraUniform {
    view_proj: [[f32; 4]; 4],
    view_position: [f32; 4],
    near: f32,
    far: f32
}

impl CameraUniform {
    pub fn new(near_far: NearFarUniform) -> Self {
        Self {
            view_proj: Matrix4::identity().into(),
            view_position: [0.0; 4],
            near: near_far.near,
            far: near_far.far
        }
    }

    pub fn update_view_proj(&mut self, camera: &Camera, projection: &Projection) {
        // The camera is always at the origin in the space these matrices operate in.
        self.view_position = [0.0, 0.0, 0.0, 1.0];
        self.view_proj = (projection.calc_matrix() * camera.calc_matrix()).into();
    }
}