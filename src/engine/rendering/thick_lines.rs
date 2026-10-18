//! Thick lines - segments drawn a fixed number of pixels wide whatever their
//! distance, for views made of lines (the play scene's F10 tactical map).
//! Each segment is one GPU instance that the shader widens into a quad on
//! screen (thick_lines_template.wgsl - drawn with the set's `material`, a
//! Lines shader: engine/thick_lines unless it says otherwise), so a big, unchanging set (a terrain's
//! contours) is uploaded once (`set_static`) and only what moves is sent
//! every frame (`dynamic`).
//!
//! Drawn over whatever's there with alpha blending and no depth test - what's
//! drawn last is on top. Optionally only inside a convex outline on screen
//! (`mask` - see `convex_mask`), e.g. the cockpit's HUD glass.

use wgpu::util::DeviceExt;

use crate::engine::scene_manager::properties::Material;

/// One segment, in world space - `width` in pixels, `color` RGBA (alpha
/// blends).
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LineInstance {
    pub start: [f32; 3],
    pub end: [f32; 3],
    pub color: [f32; 4],
    pub width: f32,
}

impl LineInstance {
    const ATTRIBUTES: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32];

}

/// The segments' vertex layout - one LineInstance per instance.
pub(crate) fn line_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<LineInstance>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &LineInstance::ATTRIBUTES,
    }
}

/// A line set's own bind group (group 2 of a Lines shader): its uniform.
pub(crate) fn bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Thick lines uniform layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        }],
    })
}

/// The most edges a mask's outline can have - see `convex_mask`.
pub const MAX_MASK_EDGES: usize = 32;

/// The set's own uniform - see thick_lines_template.wgsl's LineUniform.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct LineUniform {
    camera_position: [f32; 4],
    screen: [f32; 4],
    /// x: how many of `mask`'s edges are used (0: no mask); y: 1 when the
    /// lines are in screen pixels (see `ThickLines::screen_space`).
    mask_info: [f32; 4],
    /// Each edge of the mask's outline: (a, b, c) - a pixel (x, y) is inside
    /// it while a*x + b*y + c >= 0.
    mask: [[f32; 4]; MAX_MASK_EDGES],
}

pub struct ThickLines {
    /// What it's drawn with - a Lines shader (see engine::rendering::
    /// shader_library), engine/thick_lines unless set otherwise.
    pub material: Material,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    /// Uploaded once by `set_static` - drawn first.
    static_buffer: Option<(wgpu::Buffer, u32)>,
    /// Refilled every frame by whoever draws with it - drawn after the
    /// static set, in order.
    pub dynamic: Vec<LineInstance>,
    dynamic_buffer: Option<(wgpu::Buffer, usize)>,
    /// Only drawn inside this outline - its edges, in screen pixels (see
    /// `convex_mask`). None: everywhere.
    pub mask: Option<Vec<[f32; 3]>>,
    /// The lines are in screen pixels (x, y; z unused - from the top left),
    /// not the world - drawn where they're put, whatever the camera does.
    pub screen_space: bool,
}

/// The edges of the smallest convex outline around `points` (screen
/// pixels), as `ThickLines::mask` wants them - each one facing in, moved
/// `inset` pixels inward (a margin inside the outline). None when they
/// don't make an outline (fewer than 3 apart) or it has more than
/// MAX_MASK_EDGES edges.
pub fn convex_mask(points: &[[f32; 2]], inset: f32) -> Option<Vec<[f32; 3]>> {
    // Andrew's monotone chain.
    let mut sorted = points.to_vec();
    sorted.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    sorted.dedup();
    let cross = |o: [f32; 2], a: [f32; 2], b: [f32; 2]| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut hull: Vec<[f32; 2]> = Vec::new();
    for pass in 0..2 {
        let start = hull.len();
        let ordered: Box<dyn Iterator<Item = &[f32; 2]>> = if pass == 0 { Box::new(sorted.iter()) } else { Box::new(sorted.iter().rev()) };
        for &point in ordered {
            while hull.len() >= start + 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], point) <= 0.0 {
                hull.pop();
            }
            hull.push(point);
        }
        hull.pop();
    }
    if hull.len() < 3 || hull.len() > MAX_MASK_EDGES {
        return None;
    }
    let center = hull.iter().fold([0.0, 0.0], |sum, p| [sum[0] + p[0], sum[1] + p[1]]);
    let center = [center[0] / hull.len() as f32, center[1] / hull.len() as f32];
    Some((0..hull.len()).map(|index| {
        let (p, q) = (hull[index], hull[(index + 1) % hull.len()]);
        // Normalized, so `c` is in pixels.
        let length = ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt().max(1e-6);
        let (a, b) = ((p[1] - q[1]) / length, (q[0] - p[0]) / length);
        let c = -(a * p[0] + b * p[1]);
        // Facing in: the middle is inside.
        let [a, b, c] = if a * center[0] + b * center[1] + c < 0.0 { [-a, -b, -c] } else { [a, b, c] };
        [a, b, c - inset]
    }).collect())
}

impl ThickLines {
    /// `layout`: ShaderLibrary::overlay_layout(Overlay::Lines).
    pub fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> Self {
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Thick lines uniform"),
            contents: bytemuck::cast_slice(&[LineUniform { camera_position: [0.0; 4], screen: [1.0; 4], mask_info: [0.0; 4], mask: [[0.0; 4]; MAX_MASK_EDGES] }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Thick lines uniform bind group"),
            layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() }],
        });
        Self {
            material: Material::shader(crate::engine::rendering::shader_library::THICK_LINES),
            uniform_buffer,
            uniform_bind_group,
            static_buffer: None,
            dynamic: Vec::new(),
            dynamic_buffer: None,
            mask: None,
            screen_space: false,
        }
    }

    /// Nothing to draw - no static set and no `dynamic` lines.
    pub fn is_empty(&self) -> bool {
        self.static_buffer.is_none() && self.dynamic.is_empty()
    }

    /// Replaces the static set - drawn every time, under `dynamic`. Empty
    /// clears it.
    pub fn set_static(&mut self, device: &wgpu::Device, lines: &[LineInstance]) {
        self.static_buffer = (!lines.is_empty()).then(|| {
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Thick lines static"),
                contents: bytemuck::cast_slice(lines),
                usage: wgpu::BufferUsages::VERTEX,
            });
            (buffer, lines.len() as u32)
        });
    }

    /// Draws the static set, then `dynamic`, into `render_pass` with its
    /// material (`material`: its pipeline and bind group 0 - see
    /// ShaderLibrary::overlay) - seen from the camera at world
    /// `camera_position` whose view_proj is bound in `camera_bind_group`, on
    /// a screen `screen` pixels big, with the light's bind group `light`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, render_pass: &mut wgpu::RenderPass<'_>, material: (&wgpu::RenderPipeline, &wgpu::BindGroup), camera_bind_group: &wgpu::BindGroup, light: &wgpu::BindGroup, camera_position: [f32; 3], screen: [f32; 2]) {
        let mut mask = [[0.0; 4]; MAX_MASK_EDGES];
        let edges = self.mask.as_deref().unwrap_or_default();
        for (slot, edge) in mask.iter_mut().zip(edges) {
            *slot = [edge[0], edge[1], edge[2], 0.0];
        }
        let uniform = LineUniform {
            camera_position: [camera_position[0], camera_position[1], camera_position[2], 1.0],
            screen: [screen[0], screen[1], 0.0, 0.0],
            mask_info: [edges.len().min(MAX_MASK_EDGES) as f32, if self.screen_space { 1.0 } else { 0.0 }, 0.0, 0.0],
            mask,
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniform]));

        // The dynamic set's buffer, grown when it's outgrown.
        if !self.dynamic.is_empty() {
            let needed = self.dynamic.len();
            if self.dynamic_buffer.as_ref().is_none_or(|(_, capacity)| *capacity < needed) {
                let capacity = needed.next_power_of_two();
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Thick lines dynamic"),
                    size: (capacity * std::mem::size_of::<LineInstance>()) as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                self.dynamic_buffer = Some((buffer, capacity));
            }
            if let Some((buffer, _)) = &self.dynamic_buffer {
                queue.write_buffer(buffer, 0, bytemuck::cast_slice(&self.dynamic));
            }
        }

        render_pass.set_pipeline(material.0);
        render_pass.set_bind_group(0, material.1, &[]);
        render_pass.set_bind_group(1, camera_bind_group, &[]);
        render_pass.set_bind_group(2, &self.uniform_bind_group, &[]);
        render_pass.set_bind_group(3, light, &[]);
        if let Some((buffer, count)) = &self.static_buffer {
            render_pass.set_vertex_buffer(0, buffer.slice(..));
            render_pass.draw(0..6, 0..*count);
        }
        if let (Some((buffer, _)), false) = (&self.dynamic_buffer, self.dynamic.is_empty()) {
            render_pass.set_vertex_buffer(0, buffer.slice(..));
            render_pass.draw(0..6, 0..self.dynamic.len() as u32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::convex_mask;

    #[test]
    fn a_mask_holds_what_is_inside_its_outline() {
        // A square, with a point inside it that's no corner.
        let square = [[10.0, 10.0], [110.0, 10.0], [110.0, 110.0], [10.0, 110.0], [50.0, 60.0]];
        let edges = convex_mask(&square, 0.0).expect("an outline");
        assert_eq!(edges.len(), 4);
        let inside = |edges: &[[f32; 3]], x: f32, y: f32| edges.iter().all(|e| e[0] * x + e[1] * y + e[2] >= 0.0);
        assert!(inside(&edges, 60.0, 60.0));
        assert!(inside(&edges, 12.0, 60.0));
        assert!(!inside(&edges, 5.0, 60.0));
        assert!(!inside(&edges, 60.0, 120.0));
        // With a 10 px margin, near the edge is out.
        let padded = convex_mask(&square, 10.0).unwrap();
        assert!(inside(&padded, 60.0, 60.0));
        assert!(!inside(&padded, 15.0, 60.0));
        assert_eq!(convex_mask(&[[0.0, 0.0], [1.0, 1.0], [2.0, 2.0]], 0.0), None, "all in a line");
    }
}
