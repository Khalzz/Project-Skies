use nalgebra::{Matrix4, Perspective3, Point3, UnitQuaternion, Vector3};
use sdl2::rect::Point;
use wgpu::{util::DeviceExt, BindGroup, BindGroupLayout, BindGroupLayoutDescriptor, Buffer, Device};
use std::f32::consts::FRAC_PI_2;

#[rustfmt::skip]
pub const OPENGL_TO_WGPU_MATRIX: Matrix4<f32> = Matrix4::new(
    1.0, 0.0, 0.0, 0.0,
    0.0, 1.0, 0.0, 0.0,
    0.0, 0.0, 0.5, 0.5,
    0.0, 0.0, 0.0, 1.0,
);

const SAFE_FRAC_PI_2: f32 = FRAC_PI_2 - 0.0001;

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub struct NearFarUniform {
    pub near: f32,
    pub far: f32,
}

pub struct CameraRenderizable {
    pub camera: Camera,
    pub projection: Projection,
    pub uniform: CameraUniform,
    pub buffer: Buffer,
    pub bind_group_layout: BindGroupLayout,
    pub bind_group: BindGroup,
}

impl CameraRenderizable {
    pub fn new(device: &Device, config: &wgpu::SurfaceConfiguration) -> Self {
        let near_far_uniform = NearFarUniform {
            near: 0.1,
            far: 100000.0,
        };

        let camera = Camera::new(Point3::new(0.0, 0.0, 0.0), -90.0_f32.to_radians(), -20.0_f32.to_radians());
        let projection = Projection::new(config.width, config.height, 45.0, near_far_uniform.near, near_far_uniform.far);

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

        CameraRenderizable { camera, projection, uniform, buffer, bind_group, bind_group_layout }
    }

    pub fn world_to_screen(&self, pos_world: Point3<f32>, screen_width: u32, screen_height: u32) -> Option<Point> {
        let camera_to_point = pos_world - self.camera.position;
        let forward = self.camera.calc_forward_direction();

        if camera_to_point.dot(&forward) < 0.0 {
            return None;
        }

        let view_proj = Matrix4::from(self.uniform.view_proj);
        let pos_homogeneous = view_proj * pos_world.to_homogeneous();

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

    pub fn calc_matrix(&self) -> Matrix4<f32> {
        let direction = self.calc_forward_direction();
        let modified_direction = self.rotation_modifier * direction;
        let modified_up = self.rotation_modifier * self.up;

        Matrix4::look_at_rh(&self.position, &(self.position + modified_direction), &modified_up)
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
        self.view_position = camera.position.to_homogeneous().into();
        self.view_proj = (projection.calc_matrix() * camera.calc_matrix()).into();
    }
}