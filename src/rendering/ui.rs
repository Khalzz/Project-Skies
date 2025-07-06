use std::collections::HashMap;

use glyphon::{Cache, FontSystem, SwashCache, TextAtlas, TextRenderer};
use wgpu::{Buffer, Device, Queue, RenderPipeline, SurfaceConfiguration};

use crate::{rendering::vertex::VertexUi, ui::ui_node::UiNode};

// fix this so its more presentable and add here every reference to ui_components


pub struct TextRendering {
    pub text_renderer: TextRenderer,
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
    
}

// this code will make a direct reference to the UI rendering
pub enum UiContainer {
    Tagged(HashMap<String, UiNode>),
    Untagged(Vec<UiNode>)
}

/// # Ui 
/// This is the struct defined to mainly create and render ui elements in the screen, contains:
///     - **ui_pipeline and ui_rendering**: values to render our ui elements, like the render pipeline, vertex and index buffers
///     - **renderizable elements**: a list of lists where we define what we will render, if we want to show a button it should be added to one of the lists inside of it
///     - **text**: Usable data for text rendering, like font systems, text atrlas, and more... 

pub struct Ui {
    pub renderizable_elements: HashMap<String, UiContainer>,
    pub ui_pipeline: RenderPipeline,
    pub ui_rendering: UiRendering,
    pub text: TextRendering,
    pub has_changed: bool,
}

impl Ui {
    pub fn new(device: &Device, queue: &Queue, config: &SurfaceConfiguration, cache: &Cache) -> Self {
        let mut font_system = FontSystem::new();
        let font = include_bytes!("../../assets/fonts/Inter-Thin.ttf");
        font_system.db_mut().load_font_data(font.to_vec());

        let text_cache = SwashCache::new();
        let mut text_atlas = TextAtlas::new(&device, queue, cache, config.format);
        let text_renderer: TextRenderer = TextRenderer::new(
            &mut text_atlas,
            &device,
            wgpu::MultisampleState::default(),
            None,
        );

        let text_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/text_shader.wgsl").into()),
        });

        let ui_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui render pipeline layout"),
            bind_group_layouts: &[],
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
        };

        Self {
            
            ui_pipeline,
            text: TextRendering {
                text_renderer,
                text_cache,
                font_system,
                text_atlas
            },
            ui_rendering,
            renderizable_elements: HashMap::new(),
            has_changed: true
        }
    }

    pub fn add_to_ui(&mut self, collection: String, id: String, element_to_add: UiNode) {
        if let Some(static_list) = self.renderizable_elements.get_mut(&collection) {
            match static_list{
                UiContainer::Tagged(hash_map) => {
                    hash_map.insert(id, element_to_add);
                },
                UiContainer::Untagged(vec) => {
                    vec.push(element_to_add)
                },
            }
        }
    }
}