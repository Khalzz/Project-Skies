use std::collections::HashMap;
use crate::app::Size;

use glyphon::{Cache, FontSystem, SwashCache, TextAtlas, TextRenderer};
use ron::from_str;
use wgpu::{util::DeviceExt, BindGroup, BindGroupLayout, BindGroupLayoutDescriptor, Buffer, Device, Queue, SurfaceConfiguration};

use crate::rendering::models::textures::Texture;
use crate::rendering::render_pipeline::blur_renderer::BlurRender;
use crate::rendering::shader_library::UI;
use crate::scene_manager::node::properties::Material;
use crate::rendering::vertex::{ImageVertex, VertexUi};
use crate::ui::ui_structure::UiStructure;
use crate::ui::ui_node::{UiNode, UiNodeContent};
use crate::ui::widgets::toast::Toasts;
use crate::window::window::CursorIcon;


pub struct TextRendering {
    pub text_renderer: TextRenderer,
    // A second, independent TextRenderer for always_on_top content (a modal,
    // currently) - glyphon's TextRenderer::prepare() clears and rebuilds its
    // *entire* internal batch on every call (see its own source), so calling it
    // twice in one frame on the SAME renderer would wipe out the first batch
    // rather than layering with it. Two renderers, prepared+rendered as two
    // separate draw calls in the right order (see App::prepare_ui_content /
    // App::render_ui_pass), is what actually lets a modal's own text render
    // above a panel's text instead of every node's text always landing in one
    // shared, unordered-relative-to-quads final pass. Shares text_cache/
    // font_system/text_atlas with the main renderer - those are just glyph
    // rasterization/atlas caches, not per-frame batch state, so there's nothing
    // to duplicate there.
    pub text_renderer_on_top: TextRenderer,
    pub text_cache: SwashCache,
    pub font_system: FontSystem,
    pub text_atlas: TextAtlas

}
pub struct UiRendering {
    pub vertex_buffer: Buffer,
    pub index_buffer: Buffer,
    pub vertices: Vec<VertexUi>,
    pub indices: Vec<u16>,
    pub num_vertices: u16,
    pub num_indices: u32,
    // Where always_on_top nodes' quads start within indices/vertices (everything
    // before this index is the main pass, everything from here on is the
    // on-top pass) - see App::prepare_ui_content (the only writer) and
    // App::render_ui_pass (the only reader, which draws the two ranges as
    // separate draw_indexed calls so the on-top quads - and, more importantly,
    // the on-top *text*, a wholly separate draw pass - land above the main
    // pass's instead of every node's text piling into one shared final pass
    // with no z-order relative to any quad drawn after it).
    pub main_index_count: u32,
    // Quads queued by UiNodeContent::Image this frame, keyed by material (see
    // `material_key`) and image path (not a per-node id) - several nodes
    // referencing the same file with the same material end up in the same
    // entry, batched into one draw call per distinct pair in prepare_ui_content.
    pub image_quads: HashMap<(String, String), Vec<ImageVertex>>,
    /// Where each material's quads start in `indices` - a run lasts until the
    /// next one (or the end). Drawn one draw call per run.
    pub runs: Vec<(u32, String)>,
    /// Every material the quads use, by key - see `material_key`.
    pub materials: HashMap<String, Material>,
    /// The cursor asked for by the topmost node under the mouse this frame
    /// (see `UiNode::set_cursor`) - None: no node there asks for one.
    pub hover_cursor: Option<CursorIcon>,
    /// Whether a node drawn so far this frame has `background_blur` - set by
    /// the nodes, and moved into `main_uses_blur` at the main/on-top
    /// boundary (see App::prepare_ui_content), so after it, it's only the
    /// always-on-top layer's. The blur only runs when something uses it.
    pub uses_blur: bool,
    /// Whether the main layer has a blurred node.
    pub main_uses_blur: bool,
    /// Whether a text input (see widgets::text_input) is being typed into -
    /// a game should ignore its own keyboard controls while it is.
    pub text_input_focused: bool,
    /// Screen points per UI unit - see Ui::scale_factor. Kept here too so the
    /// layout pass can turn the mouse into UI units (see `mouse`).
    pub(crate) ui_scale: f32,
}

impl UiRendering {
    /// The mouse, in UI units - where the UI's hit-tests compare it.
    pub(crate) fn mouse(&self) -> (f32, f32) {
        let scale = self.ui_scale.max(0.0001);
        (crate::input::input::mouse_x() as f32 / scale, crate::input::input::mouse_y() as f32 / scale)
    }
}

impl UiRendering {
    /// The material key for a node's `material` - its shader, or (with values
    /// of its own) the shader and those, so nodes setting the same ones share
    /// it. None: engine/ui. Kept in `materials`.
    pub fn material_key(&mut self, material: Option<&Material>) -> String {
        let material = material.cloned().unwrap_or_else(|| Material::shader(UI));
        let key = if material.params.is_empty() { material.shader.clone() } else { format!("{}@ui{:?}", material.shader, material.params) };
        self.materials.entry(key.clone()).or_insert(material);
        key
    }

    /// The quads pushed from here on are drawn with `material` (see
    /// `material_key`) - call before pushing a node's own quad.
    pub fn use_material(&mut self, material: Option<&Material>) {
        let key = self.material_key(material);
        match self.runs.last() {
            Some((_, last)) if *last == key => {}
            Some((start, _)) if *start == self.num_indices => {
                // Nothing pushed with the last one - it's replaced.
                self.runs.last_mut().expect("just matched").1 = key;
            }
            _ => self.runs.push((self.num_indices, key)),
        }
    }

    /// The runs (see `runs`) within the indices `from..to`: each one's range
    /// there and its material key.
    pub fn runs_between(&self, from: u32, to: u32) -> Vec<(std::ops::Range<u32>, &str)> {
        let mut between = Vec::new();
        for (index, (start, key)) in self.runs.iter().enumerate() {
            let end = self.runs.get(index + 1).map_or(self.num_indices, |(next, _)| *next);
            let (start, end) = ((*start).max(from), end.min(to));
            if start < end {
                between.push((start..end, key.as_str()));
            }
        }
        between
    }

    /// Empties everything for a new frame's quads.
    pub fn clear(&mut self) {
        self.vertices.clear();
        self.num_vertices = 0;
        self.indices.clear();
        self.num_indices = 0;
        self.image_quads.clear();
        self.runs.clear();
        self.materials.clear();
        self.hover_cursor = None;
        self.uses_blur = false;
        self.main_uses_blur = false;
        self.text_input_focused = false;
    }

    /// Recreates vertex_buffer if `vertices` no longer fits in it - buffers start
    /// at a fixed initial size (see Ui::new) since most frames don't need more,
    /// but a UI complex enough to exceed it (e.g. a modal with many rows, see
    /// main_menu::rebind_modal) would otherwise panic on the write_buffer call
    /// right after this (Queue::write_buffer validates the copy fits the
    /// destination buffer - it doesn't grow it). Doubles rather than growing to
    /// the exact size needed, so a UI that fluctuates near the boundary isn't
    /// reallocating every single frame.
    pub fn ensure_vertex_capacity(&mut self, device: &Device) {
        let needed = (self.vertices.len() * std::mem::size_of::<VertexUi>()) as u64;
        if needed > self.vertex_buffer.size() {
            let new_size = needed.max(self.vertex_buffer.size() * 2);
            self.vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: new_size,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
    }

    /// Same idea as ensure_vertex_capacity, for index_buffer/indices.
    pub fn ensure_index_capacity(&mut self, device: &Device) {
        let needed = (self.indices.len() * std::mem::size_of::<u16>()) as u64;
        if needed > self.index_buffer.size() {
            let new_size = needed.max(self.index_buffer.size() * 2);
            self.index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: new_size,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
    }
}

/// One batched draw call built from UiRendering::image_quads for a single frame -
/// rebuilt each time prepare_ui_content runs, consumed by render_ui_pass. Its
/// quads are a stretch of `Ui::image_vertex_buffer`/`image_index_buffer`,
/// which every image shares.
pub struct ImageDraw {
    pub path: String,
    /// Its material - see UiRendering::material_key.
    pub material: String,
    /// Its indices in `Ui::image_index_buffer` - counted from its first
    /// vertex, `base_vertex`.
    pub indices: std::ops::Range<u32>,
    pub base_vertex: i32,
}


/// # Ui 
/// This is the struct defined to mainly create and render ui elements in the screen, contains:
///     - **ui_rendering**: values to render our ui elements, like the vertex and index buffers and which material each run of quads is drawn with (Ui shaders - see engine::rendering::shader_library)
///     - **renderizable elements**: a list of lists where we define what we will render, if we want to show a button it should be added to one of the lists inside of it
///     - **text**: Usable data for text rendering, like font systems, text atrlas, and more... 

pub struct Ui {
    pub renderizable_elements: HashMap<String, UiNode>,
    pub ui_rendering: UiRendering,
    pub text: TextRendering,
    pub has_changed: bool,
    // Lets the UI shader sample BlurRender's sharp scene_color texture directly
    // for any node with background_blur set (see UiNode::set_background_blur) -
    // rebuilt (texture view + screen_size contents both change) on resize, see
    // Ui::resize_blur_binding.
    blur_bind_group_layout: BindGroupLayout,
    pub(crate) blur_bind_group: BindGroup,
    // The same, sampling BlurRender::backdrop (the frame with the HUD already
    // drawn) - what always_on_top nodes blur, so a modal's blurred backdrop
    // shows the HUD behind it instead of painting over it with the world.
    pub(crate) blur_bind_group_on_top: BindGroup,
    screen_size_buffer: Buffer,
    // This frame's image draw calls - the images themselves are in
    // app.assets (see ImageCache::get), by path.
    pub image_draws: Vec<ImageDraw>,
    // Every image draw's quads, one after another - kept from frame to
    // frame and only made bigger when they don't fit (see grow_buffer).
    pub(crate) image_vertex_buffer: Buffer,
    pub(crate) image_index_buffer: Buffer,
    // Toggled by "toggle_ui_debug" (no key any more - set it, see App::run) - draws an outline at every
    // node's resolved rect, recursively, so layout bugs (like a hover hit-test not
    // lining up with what's visually drawn) can be seen directly.
    pub debug_bounds: bool,
    // Top-level renderizable_elements keys that must render after every other
    // top-level node, in this order - e.g. a modal overlay (see
    // main_menu::rebind_modal), which needs to visually sit above every other
    // panel regardless of insertion order. Exists because renderizable_elements
    // is a HashMap: iterating it gives *some* consistent order for a given run,
    // but not one based on insertion or any other meaningful rule, so nothing
    // can rely on "the thing I built last renders last" without this - see
    // App::prepare_ui_content, the only reader.
    pub always_on_top: Vec<String>,
    // The mirror image of always_on_top - top-level keys that must render
    // *before* every other top-level node (in this order), e.g. a persistent
    // full-screen backdrop shared behind several panels that toggle active/
    // inactive on top of it (see main_menu::ui's "Backdrop"). Without this,
    // Backdrop's render order relative to those panels would be exactly as
    // arbitrary as always_on_top's own doc comment describes - it could just as
    // easily land on top and paint over their button backgrounds/hover
    // highlights (text would still show through, being a separate always-last
    // pass, which is what would make this particular failure mode easy to miss
    // in a quick look but still very visibly wrong).
    pub always_on_bottom: Vec<String>,
    // Top-level keys drawn over everything, always_on_top included - like
    // always_on_top, but they don't block input to the rest of the UI (a
    // toast, see widgets::toast). Drawn in the always-on-top layer, after it.
    pub overlay: Vec<String>,
    /// The toasts showing now, and where they stack - see widgets::toast.
    pub toasts: Toasts,
    /// How the UI follows the window's size - see `UiScaling`.
    pub scaling: UiScaling,
    /// An extra size multiplier over `scaling` - a "UI size" setting (1.0:
    /// as designed).
    pub scale: f32,
    /// The scale in use - `scaling` for the window's size, times `scale` -
    /// and the room the UI lays out in (the window's size divided by it).
    /// Kept up by App each frame; see `scale_factor`/`size`.
    applied_scale: f32,
    size: (f32, f32),
}

/// How the UI follows the window's size. Everything the UI places and sizes
/// - positions, sizes, padding, text, borders, corners, blur - is in UI
/// units; this says how many screen points one is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UiScaling {
    /// One unit is one point, whatever the window's size - a smaller window
    /// shows the same UI in less room.
    Fixed,
    /// The UI is designed for a window `reference_height` tall and scaled to
    /// the real one, so it always covers the same share of the window - at
    /// half the height (a smaller window, or a lower fullscreen resolution)
    /// it's drawn at half the size.
    WithWindow { reference_height: f32 },
}

impl Default for UiScaling {
    /// Designed for 1080p.
    fn default() -> Self {
        UiScaling::WithWindow { reference_height: 1080.0 }
    }
}

impl Ui {
    /// How many screen points one UI unit is now - see `UiScaling`.
    pub fn scale_factor(&self) -> f32 {
        self.applied_scale
    }

    /// The room the UI lays out in, in UI units - the window's size divided
    /// by `scale_factor`. What full-screen UI (a `Layer`, a modal) is sized
    /// and placed against.
    pub fn size(&self) -> (f32, f32) {
        self.size
    }

    /// `size` as a `Size` (UI units, rounded, at least 1) - what its layout
    /// pass turns positions into screen space with.
    pub fn layout_size(&self) -> Size {
        let (width, height) = self.size;
        Size { width: width.round().max(1.0) as u32, height: height.round().max(1.0) as u32 }
    }

    /// Lays every node out again in the UI's current room (see `size`) -
    /// their Percent sizes and Center/End positions were worked out against
    /// the old one (see UiNode::resolve). The engine does it after a resize
    /// or a change of scale; a game can too, after changing nodes' layout.
    pub fn relayout(&mut self) {
        let (width, height) = self.size();
        for node in self.renderizable_elements.values_mut() {
            node.resolve(width, height);
        }
        self.has_changed = true;
    }

    /// Works out the scale for a window `width` x `height` points - true if
    /// it changed, and the UI has to be laid out again (see `relayout`).
    pub(crate) fn update_scale(&mut self, width: u32, height: u32) -> bool {
        let fitted = match self.scaling {
            UiScaling::Fixed => 1.0,
            UiScaling::WithWindow { reference_height } => height as f32 / reference_height.max(1.0),
        };
        let scale = (fitted * self.scale).max(0.05);
        let size = (width as f32 / scale, height as f32 / scale);
        let changed = scale != self.applied_scale || size != self.size;
        self.applied_scale = scale;
        self.size = size;
        self.ui_rendering.ui_scale = scale;
        changed
    }

    /// The UI's own bind group for its boxes (group 2 of a Ui shader - see
    /// ui_template.wgsl): the sharp scene, its blur levels (an array - see
    /// BlurRender), the screen size - so a box can be frosted glass (see
    /// UiNode::set_background_blur).
    pub fn box_bind_group_layout(device: &Device) -> BindGroupLayout {
        device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("ui_blur_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        })
    }

    /// `box_layout`: what its boxes' bind groups are made with -
    /// ShaderLibrary::overlay_layout(Overlay::UiBox); its images' are
    /// app.assets' (see ImageCache::get). They're drawn with Ui shaders (see
    /// engine::rendering::shader_library) - engine/ui unless a node says
    /// otherwise (UiNode::set_material).
    #[allow(clippy::too_many_arguments)]
    pub fn new(device: &Device, queue: &Queue, config: &SurfaceConfiguration, cache: &Cache, blur: &BlurRender, box_layout: &BindGroupLayout) -> Self {
        let mut font_system = FontSystem::new();
        let font = include_bytes!("../../../assets/fonts/Inter-Thin.ttf");
        font_system.db_mut().load_font_data(font.to_vec());
        // Regular weight of the same family, for labels that ask for it by
        // name (`Font { family: Some("Inter"), .. }`) - loading it doesn't
        // change what the default sans-serif labels resolve to.
        let font = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
        font_system.db_mut().load_font_data(font.to_vec());

        let text_cache = SwashCache::new();
        let mut text_atlas = TextAtlas::new(&device, queue, cache, config.format);
        let text_renderer: TextRenderer = TextRenderer::new(
            &mut text_atlas,
            &device,
            wgpu::MultisampleState::default(),
            None,
        );
        let text_renderer_on_top: TextRenderer = TextRenderer::new(
            &mut text_atlas,
            &device,
            wgpu::MultisampleState::default(),
            None,
        );

        let blur_bind_group_layout = box_layout.clone();
        let screen_size_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ui_screen_size_buffer"),
            contents: bytemuck::cast_slice(&[config.width as f32, config.height as f32, 0.0, 0.0]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let blur_bind_group = Self::build_blur_bind_group(device, &blur_bind_group_layout, &blur.scene_color, blur, &screen_size_buffer);
        let blur_bind_group_on_top = Self::build_blur_bind_group(device, &blur_bind_group_layout, &blur.backdrop, blur, &screen_size_buffer);

        let ui_rendering = UiRendering {
            vertex_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 5000,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            index_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 5000,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            vertices: Vec::new(),
            indices: Vec::new(),
            num_vertices: 0,
            num_indices: 0,
            main_index_count: 0,
            image_quads: HashMap::new(),
            runs: Vec::new(),
            materials: HashMap::new(),
            hover_cursor: None,
            uses_blur: false,
            main_uses_blur: false,
            text_input_focused: false,
            ui_scale: 1.0,
        };

        Self {
            text: TextRendering {
                text_renderer,
                text_renderer_on_top,
                text_cache,
                font_system,
                text_atlas
            },
            ui_rendering,
            renderizable_elements: HashMap::new(),
            has_changed: true,
            blur_bind_group_layout,
            blur_bind_group,
            blur_bind_group_on_top,
            screen_size_buffer,
            image_draws: Vec::new(),
            image_vertex_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("UI image vertices"),
                size: 4096,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            image_index_buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("UI image indices"),
                size: 4096,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            debug_bounds: false,
            always_on_top: Vec::new(),
            always_on_bottom: Vec::new(),
            overlay: Vec::new(),
            toasts: Toasts::default(),
            scaling: UiScaling::default(),
            scale: 1.0,
            // Until App's first update_scale (see App::run).
            applied_scale: 1.0,
            size: (config.width as f32, config.height as f32),
        }
    }

    /// `sharp`: what's behind, unblurred - the scene, or the backdrop.
    fn build_blur_bind_group(device: &Device, layout: &BindGroupLayout, sharp: &Texture, blur: &BlurRender, screen_size_buffer: &Buffer) -> BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui_blur_bind_group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&sharp.view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sharp.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&blur.levels_view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&blur.sampler) },
                wgpu::BindGroupEntry { binding: 4, resource: screen_size_buffer.as_entire_binding() },
            ],
        })
    }

    // Called from App::resize, after Renderer::resize has already rebuilt
    // BlurRender's textures at the new resolution - their view identities change
    // on every resize (BlurRender::resize fully recreates them), so the bind
    // group has to be rebuilt to point at them; screen_size has to be rewritten
    // too, since ui_template.wgsl uses it both to turn a fragment's pixel
    // position into a UV for sampling scene_color/blurred, and to convert a
    // node's background_blur pixel radius into that same UV space.
    pub fn resize_blur_binding(&mut self, device: &Device, queue: &Queue, blur: &BlurRender, width: u32, height: u32) {
        queue.write_buffer(&self.screen_size_buffer, 0, bytemuck::cast_slice(&[width as f32, height as f32, 0.0, 0.0]));
        self.blur_bind_group = Self::build_blur_bind_group(device, &self.blur_bind_group_layout, &blur.scene_color, blur, &self.screen_size_buffer);
        self.blur_bind_group_on_top = Self::build_blur_bind_group(device, &self.blur_bind_group_layout, &blur.backdrop, blur, &self.screen_size_buffer);
    }

    pub fn load_ui(&mut self, path: &str, screen_width: u32, screen_height: u32) {
        let ui_structure = self.open_ui(path);
        let sw = screen_width as f32;
        let sh = screen_height as f32;

        match ui_structure {
            Some(ui_structure) => {
                for (id, component) in &ui_structure.children {
                    let node = UiNode::from_component(component, &mut self.text.font_system, sw, sh);
                    self.renderizable_elements.insert(id.clone(), node);
                }
            },
            None => {
                println!("Failed to load UI from path: {}", path);
            }
        }
    }

    /// Every image path its nodes draw - for app.assets to load with the
    /// scene (see App::open_scene).
    pub(crate) fn image_paths(&self) -> Vec<String> {
        let mut paths = Vec::new();
        for node in self.renderizable_elements.values() {
            Self::collect_image_paths(node, &mut paths);
        }
        paths
    }

    fn collect_image_paths(node: &UiNode, paths: &mut Vec<String>) {
        match &node.content {
            UiNodeContent::Image(image) => paths.push(image.path.clone()),
            UiNodeContent::Container(container) => {
                for (_, child) in &container.children {
                    Self::collect_image_paths(child, paths);
                }
            }
            UiNodeContent::Text(_) => {}
        }
    }

    // Rebuilds this frame's batched image draw calls from UiRendering::image_quads -
    // one draw per distinct image path (and material), however many nodes share
    // it, all written into the two shared image buffers. Called from
    // App::prepare_ui_content, same place ui_rendering's own buffers get rebuilt.
    pub fn build_image_draws(&mut self, device: &Device, queue: &Queue) {
        self.image_draws.clear();
        let mut all_vertices: Vec<ImageVertex> = Vec::new();
        let mut all_indices: Vec<u16> = Vec::new();

        for ((material, path), vertices) in &self.ui_rendering.image_quads {
            let first_index = all_indices.len() as u32;
            let quad_count = (vertices.len() / 4) as u32;
            for quad in 0..quad_count {
                let base = (quad * 4) as u16;
                all_indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            }
            self.image_draws.push(ImageDraw {
                path: path.clone(),
                material: material.clone(),
                indices: first_index..all_indices.len() as u32,
                base_vertex: all_vertices.len() as i32,
            });
            all_vertices.extend_from_slice(vertices);
        }
        if all_indices.is_empty() {
            return;
        }

        let vertex_bytes: &[u8] = bytemuck::cast_slice(&all_vertices);
        grow_buffer(&mut self.image_vertex_buffer, device, vertex_bytes.len() as u64, wgpu::BufferUsages::VERTEX);
        queue.write_buffer(&self.image_vertex_buffer, 0, vertex_bytes);
        // 6 indices of 2 bytes a quad - always a multiple of the 4 bytes
        // write_buffer needs.
        let index_bytes: &[u8] = bytemuck::cast_slice(&all_indices);
        grow_buffer(&mut self.image_index_buffer, device, index_bytes.len() as u64, wgpu::BufferUsages::INDEX);
        queue.write_buffer(&self.image_index_buffer, 0, index_bytes);
    }

    pub fn open_ui(&mut self, path: &str) -> Option<UiStructure> {
        match std::fs::read_to_string(path) {
            Ok(file_contents) => {
                match from_str::<UiStructure>(&file_contents) {
                    Ok(ui) => {
                        // Here we start loading elements based on this to the hashmap components
                        return Some(ui)
                    },
                    Err(e) => {
                        // Handle the error if deserialization fails
                        eprintln!("Error deserializing RON: {}", e);
                    }
                }
            },
            _ => {}
        }
        return None
    }

    pub fn add_to_ui(&mut self, id: String, element_to_add: UiNode) {
        self.renderizable_elements.insert(id, element_to_add);
    }

    /// Get a UI node by path. Supports nested access with `/` separator.
    /// e.g. `"data_box/framerate"` gets the child `framerate` from the container `data_box`.
    pub fn get_ui_node<'a>(elements: &'a mut HashMap<String, UiNode>, path: &str) -> Option<&'a mut UiNode> {
        let mut parts = path.splitn(2, '/');
        let root_key = parts.next()?;
        let node = elements.get_mut(root_key)?;
        match parts.next() {
            Some(rest) => Self::traverse_node(node, rest),
            None => Some(node),
        }
    }

    fn traverse_node<'a>(node: &'a mut UiNode, path: &str) -> Option<&'a mut UiNode> {
        let mut parts = path.splitn(2, '/');
        let key = parts.next()?;
        let child = match &mut node.content {
            UiNodeContent::Container(container) => container.children.iter_mut()
                .find(|(id, _)| id == key)
                .map(|(_, child)| child),
            _ => None,
        }?;
        match parts.next() {
            Some(rest) => Self::traverse_node(child, rest),
            None => Some(child),
        }
    }
}

/// Makes `buffer` (`usage`, written to) at least `needed` bytes - a new one,
/// twice as big at least, only when it's too small; otherwise kept as it is.
pub(crate) fn grow_buffer(buffer: &mut Buffer, device: &Device, needed: u64, usage: wgpu::BufferUsages) {
    if needed <= buffer.size() {
        return;
    }
    *buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: needed.max(buffer.size() * 2),
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
}
