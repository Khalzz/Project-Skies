use cgmath::{perspective, InnerSpace, Matrix4, One, Point3, Quaternion, Rad, Rotation3, SquareMatrix, Vector3};
use sdl2::rect::Point;
use wgpu::{util::DeviceExt, BindGroup, BindGroupLayout, BindGroupLayoutDescriptor, Buffer, Device};
use std::f32::consts::FRAC_PI_2;

#[rustfmt::skip]
pub const OPENGL_TO_WGPU_MATRIX: cgmath::Matrix4<f32> = cgmath::Matrix4::new(
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
    pub bind_group: BindGroup
}

impl CameraRenderizable {
    pub fn new(device: &Device, config: &wgpu::SurfaceConfiguration) -> Self {

        let near_far_uniform = NearFarUniform {
            near: 0.1,
            far: 10000.0
        };

        let camera = Camera::new((-0.0, 0.0, 0.0), cgmath::Deg(-90.0), cgmath::Deg(-20.0));
        let projection = Projection::new(config.width, config.height, 45.0, near_far_uniform.near, near_far_uniform.far);

        // we create the 4x4 matrix of the camera
        let uniform = CameraUniform::new(near_far_uniform);

        // we create a buffer and a bind group
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Camera Buffer"),
            contents: bytemuck::cast_slice(&[uniform]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("camera_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform, // its a uniform buffer, duh
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("camera_bind_group"),
                layout: &bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    },
                ],
            }
        );

        return CameraRenderizable { camera, projection, uniform, buffer, bind_group, bind_group_layout };
    }

    pub fn world_to_screen(&self, pos_world: Point3<f32>, screen_width: u32, screen_height: u32) -> Option<Point> {
        // Check if the point is within the camera's view direction
        let camera_to_point = pos_world - self.camera.position;
        let forward = self.camera.calc_forward_direction();

        if camera_to_point.dot(forward) < 0.0 {
            return None; // The point is behind the camera
        }

        // Get the combined view-projection matrix
        let view_proj = Matrix4::from(self.uniform.view_proj);

        // Convert the 3D position to homogeneous coordinates
        let pos_homogeneous = view_proj * pos_world.to_homogeneous();

        // Perform the perspective divide
        if pos_homogeneous.w != 0.0 {
            let ndc = pos_homogeneous.truncate() / pos_homogeneous.w;

            // Check if the position is within the normalized device coordinates range [-1, 1] for x and y,
            // and between 0 and 1 for z to ensure it's in front of the camera
            if ndc.x.abs() <= 1.0 && ndc.y.abs() <= 1.0 && ndc.z >= 0.0 && ndc.z <= 1.0 {
                // Convert NDC to screen coordinates
                let x = ((ndc.x + 1.0) * 0.5) * screen_width as f32;
                let y = ((1.0 - (ndc.y + 1.0) * 0.5)) * screen_height as f32; // Invert Y-axis

                Some(Point::new(x as i32, y as i32))
            } else {
                None // The point is outside the view frustum
            }
        } else {
            None // The point is not visible
        }
    }
}

// we create the values that make our camera position and view angle
#[derive(Copy, Clone, Debug)]
pub struct Camera {
    pub position: Point3<f32>,
    pub yaw: Rad<f32>,
    pub pitch: Rad<f32>,
    pub look_at: Option<Point3<f32>>,
    pub up: Vector3<f32>,
    pub rotation_modifier: Quaternion<f32>
}

impl Camera {
    pub fn new< V: Into<Point3<f32>>, Y: Into<Rad<f32>>, P: Into<Rad<f32>>,>(position: V, yaw: Y, pitch: P) -> Self {
        Self {
            position: position.into(),
            yaw: yaw.into(),
            pitch: pitch.into(),
            look_at: None,
            up: Vector3::unit_y(),
            rotation_modifier: Quaternion::one()
        }    
    }

    pub fn calc_forward_direction(&self) -> Vector3<f32> {
        let (sin_pitch, cos_pitch) = self.pitch.0.sin_cos();
        let (sin_yaw, cos_yaw) = self.yaw.0.sin_cos();
        Vector3::new(cos_pitch * cos_yaw, sin_pitch, cos_pitch * sin_yaw).normalize()
    }

    pub fn calc_matrix(&self) -> Matrix4<f32> {
        let (sin_pitch, cos_pitch) = self.pitch.0.sin_cos();
        let (sin_yaw, cos_yaw) = self.yaw.0.sin_cos();
        let direction = Vector3::new(cos_pitch * cos_yaw, sin_pitch, cos_pitch * sin_yaw).normalize();

        // Apply the modifier quaternion to the direction
        let modified_direction = self.rotation_modifier * direction;
        let modified_up = self.rotation_modifier * self.up;

        return Matrix4::look_to_rh(self.position, modified_direction, modified_up)
    }

    pub fn look_at(&mut self, target: Point3<f32>) {
        // first we get access to the position of the object we want to look relative to the camera position
        let direction = target - self.position;
        let normalized_dir = direction.normalize();

        // calculate pitch and yaw transforming the vector to radians for the pitch and yaw
        self.pitch = Rad(normalized_dir.y.asin());
        self.yaw = Rad(normalized_dir.z.atan2(normalized_dir.x));
    }

    pub fn set_up(&mut self, direction: Vector3<f32>) {
        self.up = direction;
    }

    pub fn set_position(&mut self, target_position: Point3<f32>, delta_time: f32) {
        // let smooth_factor = 10.0; // Adjust this factor to make the movement smoother
        self.position += target_position - self.position;
    }
    

    pub fn rot_to_quaternion(&self) -> Quaternion<f32> {
        let cy = (self.yaw.0 * 0.5).cos();
        let sy = (self.yaw.0 * 0.5).sin();
        let cp = (self.pitch.0 * 0.5).cos();
        let sp = (self.pitch.0 * 0.5).sin();
    
        Quaternion::new(
            cy * cp,
            cy * sp,
            sy * cp,
            -sy * sp,
        )
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
            view_proj: cgmath::Matrix4::identity().into(),
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

// Projection will give us the image that the camera will see based on the position, fov and near or far values
// this will only change when we resize the window
pub struct Projection {
    aspect: f32,
    pub fovy: f32,
    pub znear: f32,
    zfar: f32
}

impl Projection {
    pub fn new(width: u32, height: u32, fovy: f32, znear: f32, zfar: f32,) -> Self {
        Self {
            aspect: width as f32 / height as f32,
            fovy: fovy,
            znear,
            zfar,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.aspect = width as f32 / height as f32;
    }

    pub fn calc_matrix(&self) -> Matrix4<f32> {
        OPENGL_TO_WGPU_MATRIX * perspective(cgmath::Deg(self.fovy), self.aspect, self.znear, self.zfar)
    }
}