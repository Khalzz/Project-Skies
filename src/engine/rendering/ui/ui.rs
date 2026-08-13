use std::collections::HashMap;

use glyphon::{Cache, FontSystem, SwashCache, TextAtlas, TextRenderer};
use ron::from_str;
use tokio::task;
use wgpu::{util::DeviceExt, BindGroup, BindGroupLayout, BindGroupLayoutDescriptor, Buffer, Device, Queue, RenderPipeline, SurfaceConfiguration};

use crate::engine::rendering::models::textures::Texture;
use crate::engine::rendering::vertex::{ImageVertex, VertexUi};
use crate::engine::ui::ui_structure::UiStructure;
use crate::engine::ui::ui_node::{UiNode, UiNodeContent};


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
    // Quads queued by UiNodeContent::Image this frame, keyed by image path (not a
    // per-node id) - several nodes referencing the same file end up in the same
    // entry, batched into one draw call per distinct image in prepare_ui_content.
    pub image_quads: HashMap<String, Vec<ImageVertex>>,
}

impl UiRendering {
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

/// A texture loaded for UI display, cached by file path so the same image referenced
/// by multiple nodes (or reloaded across UI files) only ever hits disk/GPU once - see
/// Ui::load_image.
pub struct UiImage {
    pub texture: Texture,
    pub bind_group: BindGroup,
}

/// One batched draw call built from UiRendering::image_quads for a single frame -
/// rebuilt each time prepare_ui_content runs, consumed by render_ui_pass.
pub struct ImageDraw {
    pub path: String,
    pub vertex_buffer: Buffer,
    pub index_buffer: Buffer,
    pub num_indices: u32,
}


/// # Ui 
/// This is the struct defined to mainly create and render ui elements in the screen, contains:
///     - **ui_pipeline and ui_rendering**: values to render our ui elements, like the render pipeline, vertex and index buffers
///     - **renderizable elements**: a list of lists where we define what we will render, if we want to show a button it should be added to one of the lists inside of it
///     - **text**: Usable data for text rendering, like font systems, text atrlas, and more... 

pub struct Ui {
    pub renderizable_elements: HashMap<String, UiNode>,
    pub ui_pipeline: RenderPipeline,
    pub ui_rendering: UiRendering,
    pub text: TextRendering,
    pub has_changed: bool,
    pub image_pipeline: RenderPipeline,
    image_bind_group_layout: BindGroupLayout,
    // Lets the UI shader sample BlurRender's sharp scene_color texture directly
    // for any node with background_blur set (see UiNode::set_background_blur) -
    // rebuilt (texture view + screen_size contents both change) on resize, see
    // Ui::resize_blur_binding.
    blur_bind_group_layout: BindGroupLayout,
    pub(crate) blur_bind_group: BindGroup,
    screen_size_buffer: Buffer,
    // Loaded textures, cached by file path - Ui::load_image is a no-op past the first
    // call for a given path, however many nodes end up referencing it.
    pub images: HashMap<String, UiImage>,
    pub image_draws: Vec<ImageDraw>,
    // Toggled by "toggle_ui_debug" (F2, see App::run) - draws an outline at every
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
}

impl Ui {
    pub fn new(device: &Device, queue: &Queue, config: &SurfaceConfiguration, cache: &Cache, scene_color: &Texture, blurred: &Texture) -> Self {
        let mut font_system = FontSystem::new();
        let font = include_bytes!("../../../../assets/fonts/Inter-Thin.ttf");
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

        let text_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/text_shader.wgsl").into()),
        });

        // scene_color (sharp) + blurred (see BlurRender) textures, plus a
        // screen_size uniform - lets text_shader.wgsl crossfade between the two
        // per node based on that node's own background_blur radius (see
        // UiNode::set_background_blur), and turn a fragment's pixel position into
        // a UV for sampling both.
        let blur_bind_group_layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
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
                        view_dimension: wgpu::TextureViewDimension::D2,
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
        });

        let screen_size_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ui_screen_size_buffer"),
            contents: bytemuck::cast_slice(&[config.width as f32, config.height as f32, 0.0, 0.0]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let blur_bind_group = Self::build_blur_bind_group(device, &blur_bind_group_layout, scene_color, blurred, &screen_size_buffer);

        let ui_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui render pipeline layout"),
            bind_group_layouts: &[&blur_bind_group_layout],
            push_constant_ranges: &[],
        });

        let ui_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui render pipeline"),
            layout: Some(&ui_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &text_shader,
                entry_point: Some("vertex"),
                buffers: &[VertexUi::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &text_shader,
                entry_point: Some("fragment"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            multisample: wgpu::MultisampleState::default(),
            depth_stencil: None,
            multiview: None,
            cache: None, // BE CAREFUL BOE, THIS MIGHT GENERATE WEIRD STUFF :o
        });

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
        };

        // Image pipeline: separate from ui_pipeline since it needs a texture/sampler
        // bind group ui_pipeline has no use for, and a per-image draw call rather than
        // ui_pipeline's one shared vertex/index buffer for every non-image element.
        let image_bind_group_layout = Texture::create_bind_group_layout(device);

        let image_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("UI Image Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/ui_image.wgsl").into()),
        });

        let image_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui image pipeline layout"),
            bind_group_layouts: &[&image_bind_group_layout],
            push_constant_ranges: &[],
        });

        let image_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ui image pipeline"),
            layout: Some(&image_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &image_shader,
                entry_point: Some("vertex"),
                buffers: &[ImageVertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &image_shader,
                entry_point: Some("fragment"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: Some(wgpu::Face::Back),
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            multisample: wgpu::MultisampleState::default(),
            depth_stencil: None,
            multiview: None,
            cache: None,
        });

        Self {
            ui_pipeline,
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
            image_pipeline,
            image_bind_group_layout,
            blur_bind_group_layout,
            blur_bind_group,
            screen_size_buffer,
            images: HashMap::new(),
            image_draws: Vec::new(),
            debug_bounds: false,
            always_on_top: Vec::new(),
            always_on_bottom: Vec::new(),
        }
    }

    fn build_blur_bind_group(device: &Device, layout: &BindGroupLayout, scene_color: &Texture, blurred: &Texture, screen_size_buffer: &Buffer) -> BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ui_blur_bind_group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&scene_color.view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&scene_color.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&blurred.view) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&blurred.sampler) },
                wgpu::BindGroupEntry { binding: 4, resource: screen_size_buffer.as_entire_binding() },
            ],
        })
    }

    // Called from App::resize, after Renderer::resize has already rebuilt
    // BlurRender's textures at the new resolution - their view identities change
    // on every resize (BlurRender::resize fully recreates them), so the bind
    // group has to be rebuilt to point at them; screen_size has to be rewritten
    // too, since text_shader.wgsl uses it both to turn a fragment's pixel
    // position into a UV for sampling scene_color/blurred, and to convert a
    // node's background_blur pixel radius into that same UV space.
    pub fn resize_blur_binding(&mut self, device: &Device, queue: &Queue, scene_color: &Texture, blurred: &Texture, width: u32, height: u32) {
        queue.write_buffer(&self.screen_size_buffer, 0, bytemuck::cast_slice(&[width as f32, height as f32, 0.0, 0.0]));
        self.blur_bind_group = Self::build_blur_bind_group(device, &self.blur_bind_group_layout, scene_color, blurred, &self.screen_size_buffer);
    }

    pub fn load_ui(&mut self, path: &str, screen_width: u32, screen_height: u32, device: &Device, queue: &Queue) {
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

        self.load_referenced_images(device, queue);
    }

    /// Loads (and caches) the texture for a single UI image, keyed by its path -
    /// a no-op past the first successful call for a given path, however many nodes
    /// or however many times load_ui ends up referencing it.
    pub async fn load_image(&mut self, path: &str, device: &Device, queue: &Queue) -> anyhow::Result<()> {
        if self.images.contains_key(path) {
            return Ok(());
        }

        let bytes = crate::resources::load_asset_binary(path)?;
        let texture = Texture::from_bytes(&bytes, device, queue, path)?;

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(path),
            layout: &self.image_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&texture.view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&texture.sampler) },
            ],
        });

        self.images.insert(path.to_owned(), UiImage { texture, bind_group });
        Ok(())
    }

    // Finds every UiNodeContent::Image under renderizable_elements and loads its
    // texture (via load_image, so already-cached paths are skipped) - called from
    // load_ui so RON-declared images work without the caller having to know which
    // paths to load ahead of time.
    fn load_referenced_images(&mut self, device: &Device, queue: &Queue) {
        let mut paths = Vec::new();
        for node in self.renderizable_elements.values() {
            Self::collect_image_paths(node, &mut paths);
        }

        if paths.is_empty() {
            return;
        }

        task::block_in_place(|| {
            let runtime = tokio::runtime::Runtime::new().unwrap();
            for path in paths {
                if let Err(e) = runtime.block_on(self.load_image(&path, device, queue)) {
                    eprintln!("Failed to load UI image '{}': {}", path, e);
                }
            }
        });
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
    // one small vertex/index buffer per distinct image path, however many nodes
    // share it. Called from App::prepare_ui_content, same place ui_rendering's own
    // buffers get rebuilt.
    pub fn build_image_draws(&mut self, device: &Device) {
        self.image_draws.clear();

        for (path, vertices) in &self.ui_rendering.image_quads {
            let quad_count = (vertices.len() / 4) as u32;
            let mut indices: Vec<u16> = Vec::with_capacity(vertices.len() / 4 * 6);
            for quad in 0..quad_count {
                let base = (quad * 4) as u16;
                indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
            }

            let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(path),
                contents: bytemuck::cast_slice(vertices),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(path),
                contents: bytemuck::cast_slice(&indices),
                usage: wgpu::BufferUsages::INDEX,
            });

            self.image_draws.push(ImageDraw {
                path: path.clone(),
                vertex_buffer,
                index_buffer,
                num_indices: indices.len() as u32,
            });
        }
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