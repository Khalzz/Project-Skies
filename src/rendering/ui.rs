use std::collections::HashMap;

use glyphon::{Cache, FontSystem, SwashCache, TextAtlas, TextRenderer};
use ron::from_str;
use wgpu::{Buffer, Device, Queue, RenderPipeline, SurfaceConfiguration};

use crate::{rendering::{ui, vertex::VertexUi}, ui::{label::Label, ui_node::{self, ChildrenType, Visibility}, ui_structure::{self, UiStructure}}};
use crate::ui::ui_transform::UiTransform;
use crate::ui::ui_node::{UiNode, UiNodeContent};
use crate::ui::vertical_container::VerticalContainerData;


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

    fn convert_component(&mut self, component: &ui_structure::UiComponent, screen_width: f32, screen_height: f32) -> UiNode {
        let width = component.transform.size.as_ref().map(|s| s.width).unwrap_or(0.0);
        let height = component.transform.size.as_ref().map(|s| s.height).unwrap_or(0.0);
        let auto_size = component.transform.size.is_none();

        let mut x = component.transform.position.x;
        let y = component.transform.position.y;

        if let Some(anchor) = &component.transform.anchor {
            match anchor.as_str() {
                "center_x" => x = (screen_width / 2.0) - (width / 2.0) + component.transform.position.x,
                _ => {}
            }
        }

        let transform = UiTransform::new(
            x,
            y,
            height,
            width,
            0.0,
            auto_size,
        );
        let mut visibility = Visibility::new([0.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 0.0]);

        let content = match &component.child {
            ui_structure::UiNode::Label(label_data) => {
                let color = glyphon::Color::rgba(
                    (label_data.color[0] * 255.0) as u8,
                    (label_data.color[1] * 255.0) as u8,
                    (label_data.color[2] * 255.0) as u8,
                    (label_data.color[3] * 255.0) as u8,
                );
                let align = match label_data.alignment.as_deref() {
                    Some("Center") => glyphon::cosmic_text::Align::Center,
                    Some("Right") => glyphon::cosmic_text::Align::Right,
                    _ => glyphon::cosmic_text::Align::Left,
                };
                let bg = label_data.background_color.unwrap_or([0.0, 0.0, 0.0, 0.0]);
                let border = label_data.border_color.unwrap_or([0.0, 0.0, 0.0, 0.0]);
                visibility = Visibility::new(bg, border);
                UiNodeContent::Text(Label::new(
                    &mut self.text.font_system,
                    &label_data.text,
                    transform.clone(),
                    color,
                    align,
                    label_data.font_size,
                ))
            }
            ui_structure::UiNode::VerticalContainer(container_data) => {
                let bg = container_data.background_color.unwrap_or([0.0, 0.0, 0.0, 0.0]);
                let border = container_data.border_color.unwrap_or([0.0, 0.0, 0.0, 0.0]);
                visibility = Visibility::new(bg, border);
                let margin = container_data.margin.unwrap_or(0.0);
                let separation = container_data.separation.unwrap_or(0.0);

                let children = if let Some(ron_children) = &container_data.children {
                    let mut map = HashMap::new();
                    for (child_id, child_component) in ron_children {
                        map.insert(child_id.clone(), self.convert_component(child_component, screen_width, screen_height));
                    }
                    ChildrenType::MappedChildren(map)
                } else {
                    ChildrenType::MappedChildren(HashMap::new())
                };

                UiNodeContent::VerticalContainer(VerticalContainerData::new(margin, separation, children))
            }
        };

        UiNode { transform, visibility, content }
    }

    pub fn load_ui(&mut self, path: &str, collection: &str, screen_width: u32, screen_height: u32) {
        let ui_structure = self.open_ui(path);
        let sw = screen_width as f32;
        let sh = screen_height as f32;

        match ui_structure {
            Some(ui_structure) => {
                if !self.renderizable_elements.contains_key(collection) {
                    self.renderizable_elements.insert(collection.to_owned(), UiContainer::Tagged(HashMap::new()));
                }

                for (id, component) in &ui_structure.children {
                    let node = self.convert_component(component, sw, sh);
                    self.add_to_ui(collection.to_owned(), id.clone(), node);
                }
            },
            None => {
                println!("Failed to load UI from path: {}", path);
            }
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