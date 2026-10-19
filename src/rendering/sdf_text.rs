//! SDF text - text drawn from signed distance fields, at any size, angle or
//! perspective, without making new glyph images for each size the way
//! glyphon's bitmap atlas does.
//!
//! Each glyph is rasterized once at REF_SIZE (with the same fonts and shaping
//! as the UI's labels - cosmic-text), turned into a distance field (how far
//! each texel is from the glyph's edge, SPREAD texels either way) and packed
//! into one atlas texture. A string is then drawn as one quad per glyph, laid
//! on a plane - `TextFrame`: its middle, and which way one em runs right and
//! down. The shader finds the edge in the distance field and smooths it over
//! one screen pixel, so the letters stay sharp from a few pixels tall to
//! filling the screen, and turn and shrink with whatever they're placed on
//! (the HUD's glass, a plate in the jet's frame).
//!
//! Drawn with the set's `material` - a Text shader (sdf_text_template.wgsl;
//! engine/sdf_text unless it says otherwise).
//!
//! Like `ThickLines`: one renderer is either in screen pixels or in the world
//! (`screen_space`), refilled every frame through `push`, drawn over what's
//! there with no depth test, optionally only inside a convex outline on
//! screen (`mask` - see `thick_lines::convex_mask`).

use std::collections::HashMap;

use glyphon::cosmic_text::{fontdb, Buffer, CacheKey, CacheKeyFlags, FontSystem, Metrics, Shaping, SwashCache, SwashContent, Wrap};
use nalgebra::Vector3;
use wgpu::util::DeviceExt;

use crate::rendering::thick_lines::MAX_MASK_EDGES;
use crate::scene_manager::properties::Material;
use crate::ui::components::label::Font;

/// The size glyphs are rasterized at (px) - their distance fields hold the
/// shape well enough for a few times this on screen.
const REF_SIZE: f32 = 48.0;
/// How far out from a glyph's edge its field reaches, either way (texels at
/// REF_SIZE) - also the empty margin round each glyph in the atlas. Needs to
/// cover at least a screen pixel at the smallest size drawn: at 8 px tall one
/// pixel is 6 texels.
const SPREAD: u32 = 8;
/// The atlas texture's side (texels) - single channel.
const ATLAS_SIZE: u32 = 2048;
/// Matches the labels' (see label::LINE_HEIGHT_RATIO), so a line of SDF text
/// is as tall as the same label's.
const LINE_HEIGHT_RATIO: f32 = 1.2;
/// Laid-out strings kept between frames - cleared once there are more.
const MAX_CACHED_LAYOUTS: usize = 1024;

/// Where a string goes: `center` is the middle of its box, `right` and `down`
/// are one em (the font size) along its lines and down its lines. In screen
/// pixels (z unused) or the world, by the renderer's `screen_space`.
#[derive(Clone, Copy, Debug)]
pub struct TextFrame {
    pub center: Vector3<f32>,
    pub right: Vector3<f32>,
    pub down: Vector3<f32>,
}

impl TextFrame {
    /// Upright on the screen, centered on pixel `(x, y)`, `size` px tall.
    pub fn screen(x: f32, y: f32, size: f32) -> Self {
        Self { center: Vector3::new(x, y, 0.0), right: Vector3::new(size, 0.0, 0.0), down: Vector3::new(0.0, size, 0.0) }
    }

    /// On a plane in the world, centered on `center`: `right` and `up` are
    /// the plane's directions (unit vectors), `size` the font size (m).
    pub fn world(center: Vector3<f32>, right: Vector3<f32>, up: Vector3<f32>, size: f32) -> Self {
        Self { center, right: right * size, down: -up * size }
    }
}

/// One glyph quad - see sdf_text_template.wgsl's GlyphInstance. `rect` is the quad in
/// ems from the frame's `origin` (x right, y down), `uv` its corners in the
/// atlas.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct GlyphInstance {
    origin: [f32; 3],
    axis_x: [f32; 3],
    axis_y: [f32; 3],
    rect: [f32; 4],
    uv: [f32; 4],
    color: [f32; 4],
}

impl GlyphInstance {
    const ATTRIBUTES: [wgpu::VertexAttribute; 6] = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3, 3 => Float32x4, 4 => Float32x4, 5 => Float32x4];

}

/// The glyphs' vertex layout - one GlyphInstance per instance.
pub(crate) fn glyph_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<GlyphInstance>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &GlyphInstance::ATTRIBUTES,
    }
}

/// A text set's own bind group (group 2 of a Text shader): its uniform, its
/// atlas and the atlas's sampler.
pub(crate) fn bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("SDF text layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

/// The set's own uniform - see sdf_text_template.wgsl's TextUniform.
#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct TextUniform {
    camera_position: [f32; 4],
    screen: [f32; 4],
    /// x: how many of `mask`'s edges are used (0: no mask); y: 1 in screen
    /// pixels.
    mask_info: [f32; 4],
    mask: [[f32; 4]; MAX_MASK_EDGES],
}

/// A glyph, by what it is - the same glyph in another string reuses it.
type GlyphKey = (fontdb::ID, u16, CacheKeyFlags);

/// A glyph's field in the atlas: its corners there (0..1), and its box
/// against the pen position (REF_SIZE px, y down).
#[derive(Clone, Copy, Debug)]
struct AtlasGlyph {
    uv: [f32; 4],
    offset: [f32; 2],
    size: [f32; 2],
}

/// One glyph of a laid-out string: which, and its pen position (REF_SIZE px
/// from the string's top left; y is the baseline).
#[derive(Clone, Copy, Debug)]
struct LaidGlyph {
    key: GlyphKey,
    x: f32,
    y: f32,
}

#[derive(Clone, Debug)]
struct TextLayout {
    glyphs: Vec<LaidGlyph>,
    width: f32,
    height: f32,
}

/// Shelf packing: rows left to right, each as tall as its tallest glyph.
#[derive(Default)]
struct Packer {
    x: u32,
    y: u32,
    row_height: u32,
}

impl Packer {
    fn place(&mut self, width: u32, height: u32) -> Option<(u32, u32)> {
        if width > ATLAS_SIZE || height > ATLAS_SIZE {
            return None;
        }
        if self.x + width > ATLAS_SIZE {
            self.x = 0;
            self.y += self.row_height + 1;
            self.row_height = 0;
        }
        if self.y + height > ATLAS_SIZE {
            return None;
        }
        let at = (self.x, self.y);
        self.x += width + 1;
        self.row_height = self.row_height.max(height);
        Some(at)
    }
}

pub struct SdfText {
    /// What it's drawn with - a Text shader (see engine::rendering::
    /// shader_library), engine/sdf_text unless set otherwise.
    pub material: Material,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    atlas: wgpu::Texture,
    instances: Vec<GlyphInstance>,
    instance_buffer: Option<(wgpu::Buffer, usize)>,
    swash: SwashCache,
    /// None: a glyph with nothing to draw (a space).
    glyphs: HashMap<GlyphKey, Option<AtlasGlyph>>,
    packer: Packer,
    /// Fields made since the last draw, to copy into the atlas then: where,
    /// how big, the texels.
    uploads: Vec<(u32, u32, u32, u32, Vec<u8>)>,
    layouts: HashMap<(String, Option<&'static str>, u16), TextLayout>,
    /// Only drawn inside this outline - its edges, in screen pixels (see
    /// `thick_lines::convex_mask`). None: everywhere.
    pub mask: Option<Vec<[f32; 3]>>,
    /// Frames are in screen pixels (x, y from the top left), not the world.
    pub screen_space: bool,
    /// The target stores sRGB - colors are turned linear for it, as glyphon
    /// does, so this text matches the labels' in the same color.
    srgb_target: bool,
}

impl SdfText {
    /// For a target of `color_format`; `layout`: ShaderLibrary::
    /// overlay_layout(Overlay::Text).
    pub fn new(device: &wgpu::Device, color_format: wgpu::TextureFormat, layout: &wgpu::BindGroupLayout) -> Self {
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("SDF text uniform"),
            contents: bytemuck::cast_slice(&[TextUniform { camera_position: [0.0; 4], screen: [1.0; 4], mask_info: [0.0; 4], mask: [[0.0; 4]; MAX_MASK_EDGES] }]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("SDF text atlas"),
            size: wgpu::Extent3d { width: ATLAS_SIZE, height: ATLAS_SIZE, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let atlas_view = atlas.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("SDF text sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("SDF text bind group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&atlas_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });

        Self {
            material: Material::shader(crate::rendering::shader_library::SDF_TEXT),
            uniform_buffer,
            bind_group,
            atlas,
            instances: Vec::new(),
            instance_buffer: None,
            swash: SwashCache::new(),
            glyphs: HashMap::new(),
            packer: Packer::default(),
            uploads: Vec::new(),
            layouts: HashMap::new(),
            mask: None,
            screen_space: false,
            srgb_target: color_format.is_srgb(),
        }
    }

    /// Drops everything pushed - call before pushing a new frame's text.
    pub fn clear(&mut self) {
        self.instances.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    /// Adds `text` in `font`, centered on `frame` (see `TextFrame`), in
    /// `color` (sRGB RGBA 0..1, like the UI's colors; alpha blends) - drawn
    /// at the next `draw`.
    pub fn push(&mut self, font_system: &mut FontSystem, text: &str, font: Font, frame: TextFrame, color: [f32; 4]) {
        if text.is_empty() {
            return;
        }
        let color = if self.srgb_target { [srgb_to_linear(color[0]), srgb_to_linear(color[1]), srgb_to_linear(color[2]), color[3]] } else { color };
        let layout = self.layout(font_system, text, font);
        let origin = frame.center - frame.right * (layout.width * 0.5 / REF_SIZE) - frame.down * (layout.height * 0.5 / REF_SIZE);
        for laid in layout.glyphs {
            let Some(glyph) = self.glyph(font_system, laid.key) else { continue };
            let x0 = (laid.x + glyph.offset[0]) / REF_SIZE;
            let y0 = (laid.y + glyph.offset[1]) / REF_SIZE;
            self.instances.push(GlyphInstance {
                origin: origin.into(),
                axis_x: frame.right.into(),
                axis_y: frame.down.into(),
                rect: [x0, y0, x0 + glyph.size[0] / REF_SIZE, y0 + glyph.size[1] / REF_SIZE],
                uv: glyph.uv,
                color,
            });
        }
    }

    /// `text` shaped at REF_SIZE - kept, as the same strings come back
    /// frame after frame.
    fn layout(&mut self, font_system: &mut FontSystem, text: &str, font: Font) -> TextLayout {
        let key = (text.to_owned(), font.family, font.weight.0);
        if let Some(layout) = self.layouts.get(&key) {
            return layout.clone();
        }
        let line_height = REF_SIZE * LINE_HEIGHT_RATIO;
        let mut buffer = Buffer::new(font_system, Metrics::new(REF_SIZE, line_height));
        buffer.set_size(font_system, None, None);
        buffer.set_wrap(font_system, Wrap::None);
        buffer.set_text(font_system, text, &font.attrs(), Shaping::Advanced);
        buffer.shape_until_scroll(font_system, true);
        let mut layout = TextLayout { glyphs: Vec::new(), width: 0.0, height: 0.0 };
        for run in buffer.layout_runs() {
            layout.width = layout.width.max(run.line_w);
            layout.height = layout.height.max(run.line_top + run.line_height);
            for glyph in run.glyphs {
                layout.glyphs.push(LaidGlyph {
                    key: (glyph.font_id, glyph.glyph_id, glyph.cache_key_flags),
                    x: glyph.x + glyph.font_size * glyph.x_offset,
                    y: run.line_y + glyph.y - glyph.font_size * glyph.y_offset,
                });
            }
        }
        if self.layouts.len() >= MAX_CACHED_LAYOUTS {
            self.layouts.clear();
        }
        self.layouts.insert(key, layout.clone());
        layout
    }

    /// The glyph's field in the atlas, made the first time it's asked for.
    fn glyph(&mut self, font_system: &mut FontSystem, key: GlyphKey) -> Option<AtlasGlyph> {
        if let Some(glyph) = self.glyphs.get(&key) {
            return *glyph;
        }
        let glyph = self.make_glyph(font_system, key);
        self.glyphs.insert(key, glyph);
        glyph
    }

    fn make_glyph(&mut self, font_system: &mut FontSystem, key: GlyphKey) -> Option<AtlasGlyph> {
        let (cache_key, _, _) = CacheKey::new(key.0, key.1, REF_SIZE, (0.0, 0.0), key.2);
        let image = self.swash.get_image_uncached(font_system, cache_key)?;
        let (width, height) = (image.placement.width as usize, image.placement.height as usize);
        if width == 0 || height == 0 {
            return None;
        }
        let coverage: Vec<u8> = match image.content {
            SwashContent::Mask => image.data.clone(),
            SwashContent::Color => image.data.chunks_exact(4).map(|pixel| pixel[3]).collect(),
            SwashContent::SubpixelMask => image.data.chunks_exact(4).map(|pixel| ((pixel[0] as u16 + pixel[1] as u16 + pixel[2] as u16) / 3) as u8).collect(),
        };
        let pad = SPREAD as usize;
        let field = signed_distance_field(&coverage, width, height, pad, SPREAD as f32);
        let (field_width, field_height) = ((width + pad * 2) as u32, (height + pad * 2) as u32);
        let (x, y) = match self.packer.place(field_width, field_height) {
            Some(at) => at,
            None => {
                // Full: start over. What was pushed this frame may show the
                // wrong glyphs for a frame, until it's pushed again.
                eprintln!("SDF text atlas full - rebuilding it");
                self.glyphs.clear();
                self.uploads.clear();
                self.packer = Packer::default();
                self.packer.place(field_width, field_height)?
            }
        };
        self.uploads.push((x, y, field_width, field_height, field));
        let atlas = ATLAS_SIZE as f32;
        Some(AtlasGlyph {
            uv: [x as f32 / atlas, y as f32 / atlas, (x + field_width) as f32 / atlas, (y + field_height) as f32 / atlas],
            offset: [(image.placement.left - pad as i32) as f32, (-image.placement.top - pad as i32) as f32],
            size: [field_width as f32, field_height as f32],
        })
    }

    /// Draws what was pushed into `render_pass` with its material
    /// (`material`: its pipeline and bind group 0 - see ShaderLibrary::
    /// overlay) - seen from the camera at world `camera_position` whose
    /// view_proj is bound in `camera_bind_group`, on a screen `screen` pixels
    /// big, with the light's bind group `light`.
    #[allow(clippy::too_many_arguments)]
    pub fn draw(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, render_pass: &mut wgpu::RenderPass<'_>, material: (&wgpu::RenderPipeline, &wgpu::BindGroup), camera_bind_group: &wgpu::BindGroup, light: &wgpu::BindGroup, camera_position: [f32; 3], screen: [f32; 2]) {
        for (x, y, width, height, texels) in self.uploads.drain(..) {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &self.atlas, mip_level: 0, origin: wgpu::Origin3d { x, y, z: 0 }, aspect: wgpu::TextureAspect::All },
                &texels,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width), rows_per_image: Some(height) },
                wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            );
        }
        if self.instances.is_empty() {
            return;
        }

        let mut mask = [[0.0; 4]; MAX_MASK_EDGES];
        let edges = self.mask.as_deref().unwrap_or_default();
        for (slot, edge) in mask.iter_mut().zip(edges) {
            *slot = [edge[0], edge[1], edge[2], 0.0];
        }
        let uniform = TextUniform {
            camera_position: [camera_position[0], camera_position[1], camera_position[2], 1.0],
            screen: [screen[0], screen[1], 0.0, 0.0],
            mask_info: [edges.len().min(MAX_MASK_EDGES) as f32, if self.screen_space { 1.0 } else { 0.0 }, 0.0, 0.0],
            mask,
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniform]));

        let needed = self.instances.len();
        if self.instance_buffer.as_ref().is_none_or(|(_, capacity)| *capacity < needed) {
            let capacity = needed.next_power_of_two();
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("SDF text glyphs"),
                size: (capacity * std::mem::size_of::<GlyphInstance>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.instance_buffer = Some((buffer, capacity));
        }
        let Some((buffer, _)) = &self.instance_buffer else { return };
        queue.write_buffer(buffer, 0, bytemuck::cast_slice(&self.instances));

        render_pass.set_pipeline(material.0);
        render_pass.set_bind_group(0, material.1, &[]);
        render_pass.set_bind_group(1, camera_bind_group, &[]);
        render_pass.set_bind_group(2, &self.bind_group, &[]);
        render_pass.set_bind_group(3, light, &[]);
        render_pass.set_vertex_buffer(0, buffer.slice(..));
        render_pass.draw(0..6, 0..needed as u32);
    }
}

fn srgb_to_linear(channel: f32) -> f32 {
    if channel <= 0.04045 { channel / 12.92 } else { ((channel + 0.055) / 1.055).powf(2.4) }
}

/// The signed distance field of a glyph's coverage (`width` x `height`, 0..255),
/// with `pad` empty texels round it: 0.5 (128) on the edge, rising inside
/// and falling outside by 0.5 over `spread` texels.
fn signed_distance_field(coverage: &[u8], width: usize, height: usize, pad: usize, spread: f32) -> Vec<u8> {
    let (field_width, field_height) = (width + pad * 2, height + pad * 2);
    let alpha = |x: usize, y: usize| -> f32 {
        if x < pad || y < pad || x >= pad + width || y >= pad + height {
            return 0.0;
        }
        coverage[(y - pad) * width + (x - pad)] as f32 / 255.0
    };
    let count = field_width * field_height;
    let mut inside = vec![false; count];
    let mut to_inside = vec![FAR; count];
    let mut to_outside = vec![FAR; count];
    for y in 0..field_height {
        for x in 0..field_width {
            let index = y * field_width + x;
            inside[index] = alpha(x, y) >= 0.5;
            if inside[index] {
                to_inside[index] = 0.0;
            } else {
                to_outside[index] = 0.0;
            }
        }
    }
    squared_distances(&mut to_inside, field_width, field_height);
    squared_distances(&mut to_outside, field_width, field_height);

    let mut field = vec![0u8; count];
    for y in 0..field_height {
        for x in 0..field_width {
            let index = y * field_width + x;
            let a = alpha(x, y);
            // Texel centers: the edge is half a texel past the last one
            // inside. A partly covered texel says more exactly where it is.
            let distance = if a > 0.0 && a < 1.0 {
                0.5 - a
            } else if inside[index] {
                -(to_outside[index].sqrt() as f32 - 0.5)
            } else {
                to_inside[index].sqrt() as f32 - 0.5
            };
            let value = (0.5 - distance / (2.0 * spread)).clamp(0.0, 1.0);
            field[index] = (value * 255.0).round() as u8;
        }
    }
    field
}

/// Stands in for "no feature here" - far past any glyph, but finite so the
/// sums below stay numbers.
const FAR: f64 = 1e20;

/// Exact squared Euclidean distance transform (Felzenszwalb & Huttenlocher):
/// `grid` holds 0 at features and FAR elsewhere, and comes out holding each
/// texel's squared distance to the nearest feature.
fn squared_distances(grid: &mut [f64], width: usize, height: usize) {
    let longest = width.max(height);
    let mut line = vec![0.0; longest];
    let mut out = vec![0.0; longest];
    let mut parabolas = vec![0usize; longest];
    let mut bounds = vec![0.0; longest + 1];
    for x in 0..width {
        for y in 0..height {
            line[y] = grid[y * width + x];
        }
        transform_line(&line[..height], &mut out[..height], &mut parabolas, &mut bounds);
        for y in 0..height {
            grid[y * width + x] = out[y];
        }
    }
    for y in 0..height {
        line[..width].copy_from_slice(&grid[y * width..(y + 1) * width]);
        transform_line(&line[..width], &mut out[..width], &mut parabolas, &mut bounds);
        grid[y * width..(y + 1) * width].copy_from_slice(&out[..width]);
    }
}

/// One row or column of `squared_distances`: the lower envelope of the
/// parabolas rooted at each sample.
fn transform_line(f: &[f64], out: &mut [f64], parabolas: &mut [usize], bounds: &mut [f64]) {
    let n = f.len();
    if n == 0 {
        return;
    }
    let mut k = 0;
    parabolas[0] = 0;
    bounds[0] = f64::NEG_INFINITY;
    bounds[1] = f64::INFINITY;
    for q in 1..n {
        let qf = q as f64;
        let mut s;
        loop {
            let r = parabolas[k];
            let rf = r as f64;
            s = ((f[q] + qf * qf) - (f[r] + rf * rf)) / (2.0 * (qf - rf));
            // Stops at k == 0 at the latest: bounds[0] is -inf.
            if s <= bounds[k] {
                k -= 1;
                continue;
            }
            break;
        }
        k += 1;
        parabolas[k] = q;
        bounds[k] = s;
        bounds[k + 1] = f64::INFINITY;
    }
    k = 0;
    for (q, slot) in out.iter_mut().enumerate() {
        while bounds[k + 1] < q as f64 {
            k += 1;
        }
        let r = parabolas[k];
        let d = q as f64 - r as f64;
        *slot = d * d + f[r];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_distance_transform_is_exact() {
        // One feature in a 7x5 grid: every texel's squared distance to it.
        let (width, height) = (7, 5);
        let mut grid = vec![FAR; width * height];
        grid[2 * width + 3] = 0.0;
        squared_distances(&mut grid, width, height);
        for y in 0..height {
            for x in 0..width {
                let expected = ((x as f64 - 3.0).powi(2) + (y as f64 - 2.0).powi(2)) as f64;
                assert_eq!(grid[y * width + x], expected, "at ({x}, {y})");
            }
        }
    }

    #[test]
    fn a_field_is_high_inside_and_low_outside() {
        // A filled 4x4 square, padded by 4.
        let coverage = vec![255u8; 16];
        let field = signed_distance_field(&coverage, 4, 4, 4, 4.0);
        let width = 12;
        let at = |x: usize, y: usize| field[y * width + x];
        assert!(at(6, 6) > 128, "the middle is inside");
        assert!(at(0, 0) < 128, "the corner of the margin is outside");
        assert!(at(4, 6) < at(5, 6), "falls toward the edge");
        assert!(at(3, 6) < 128 && at(4, 6) > 128, "the edge is between the last texel in and the first out");
    }
}
