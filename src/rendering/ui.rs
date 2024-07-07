use glyphon::{FontSystem, SwashCache, TextAtlas, TextRenderer};
use wgpu::{Buffer, Device, Queue, RenderPipeline, SurfaceConfiguration};

use crate::rendering::vertex::VertexUi;

pub struct TextRendering {
    pub text_renderer: TextRenderer,
    pub text_cache: SwashCache,
    pub font_system: FontSystem,
    pub text_atlas: TextAtlas

}

pub struct UiRendering {
    pub vertex_buffer: Buffer,
    pub index_buffer: Buffer
}

// this code will make a direct reference to the UI rendering
pub struct UI {
    pub ui_pipeline: RenderPipeline,
    pub text: TextRendering,
    pub ui_rendering: UiRendering
}

impl UI {
    pub fn new(device: &Device, queue: &Queue, config: &SurfaceConfiguration) -> Self {
        let mut font_system = FontSystem::new();
        let font = include_bytes!("../../assets/fonts/Inter-Thin.ttf");
        font_system.db_mut().load_font_data(font.to_vec());

        let text_cache = SwashCache::new();
        let mut text_atlas = TextAtlas::new(&device, queue, config.format);
        let text_renderer = TextRenderer::new(
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
                entry_point: "vertex",
                buffers: &[VertexUi::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &text_shader,
                entry_point: "fragment",
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Bgra8UnormSrgb,
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
            })
        };

        Self { 
            ui_pipeline,
            text: TextRendering {
                text_renderer,
                text_cache,
                font_system,
                text_atlas
            },
            ui_rendering
        }
    }
}