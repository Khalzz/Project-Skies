use std::collections::HashMap;

use glyphon::{cosmic_text::Align, Color, FontSystem, TextArea};
use nalgebra::vector;
use crate::app::Size;
use crate::rendering::ui::UiRendering;
use crate::rendering::vertex::VertexUi;
use crate::utils::lerps::lerp;
use super::components::label::Label;
use super::components::container::Container;
use super::ui_transform::{Anchor, ChildAnchor, Direction, Fit, Rect, SelfAnchor, UiTransform};
use super::ui_structure;

pub enum UiNodeContent {
    Text(Label),
    Container(Container),
}

pub struct Visibility {
    pub background_color: [f32; 4],
    pub border_color: [f32; 4],
}

impl Visibility {
    pub fn new(background_color: [f32; 4], border_color: [f32; 4]) -> Self {
        Self { background_color, border_color }
    }
}

pub struct UiNode {
    pub transform: UiTransform,
    pub visibility: Visibility,
    pub content: UiNodeContent,
}

impl UiNode {
    pub const NUM_INDICES: u32 = 6;

    pub fn as_label_mut(&mut self) -> Option<&mut Label> {
        match &mut self.content {
            UiNodeContent::Text(label) => Some(label),
            _ => None,
        }
    }

    pub fn get_children_mut(&mut self) -> Option<&mut HashMap<String, UiNode>> {
        match &mut self.content {
            UiNodeContent::Container(container) => Some(&mut container.children),
            _ => None,
        }
    }

    // ── Rendering ──

    pub fn node_content_preparation(&mut self, size: &Size, ui: &mut UiRendering, font_system: &mut FontSystem, delta_time: f32) -> (Vec<TextArea>, u16, u32) {
        let mut text_areas: Vec<TextArea> = Vec::new();

        match &mut self.content {
            UiNodeContent::Text(label) => {
                let vertices_slice = Self::compute_vertices(&self.transform, &self.visibility, size);
                let indice_slice = Self::compute_indices(ui.num_vertices);
                label.buffer.set_size(font_system, Some(self.transform.rect.right - self.transform.rect.left), Some(self.transform.rect.bottom - self.transform.rect.top));

                let (text_area, added_vertices, added_indices) = label.ui_node_data_creation(size, &mut ui.vertices, &vertices_slice, &mut ui.indices, &indice_slice, &self.transform.rect);
                text_areas.push(text_area);
                ui.num_vertices += added_vertices;
                ui.num_indices += added_indices;
            },
            UiNodeContent::Container(container) => {
                let auto_width = self.transform.width == 0.0 || self.transform.fit.horizontal;
                let auto_height = self.transform.height == 0.0 || self.transform.fit.vertical;

                // Fit/auto-size width: measure children first
                if auto_width {
                    let max_child_width: f32 = container.children.values()
                        .map(|c| c.transform.width)
                        .fold(0.0f32, |a, b| a.max(b));
                    self.transform.width = max_child_width + container.margin * 2.0;
                    self.transform.rect.right = self.transform.rect.left + self.transform.width;
                }

                // Render container background
                let vertices_slice = Self::compute_vertices(&self.transform, &self.visibility, size);
                let indice_slice = Self::compute_indices(ui.num_vertices);
                let (cv, ci) = container.ui_node_data_creation(size, &mut ui.vertices, &vertices_slice, &mut ui.indices, &indice_slice);
                ui.num_vertices += cv;
                ui.num_indices += ci;

                // Layout children
                let parent_rect = &self.transform.rect.clone();
                let direction = &self.transform.direction.clone();
                let child_anchor = &self.transform.child_anchor.clone();
                let margin = container.margin;
                let gap = container.gap;

                let content_left = parent_rect.left + margin;
                let content_top = parent_rect.top + margin;
                let content_right = parent_rect.right - margin;
                let content_bottom = parent_rect.bottom - margin;
                let content_w = content_right - content_left;
                let content_h = content_bottom - content_top;

                // Calculate total children size for centering/end alignment on main axis
                let total_children_main: f32 = container.children.values()
                    .map(|c| match direction {
                        Direction::Vertical => c.transform.height,
                        Direction::Horizontal => c.transform.width,
                    })
                    .sum::<f32>() + gap * (container.children.len().saturating_sub(1) as f32);

                // Starting cursor on main axis based on child_anchor
                let mut cursor = match direction {
                    Direction::Vertical => match child_anchor.vertical {
                        Anchor::Start => content_top,
                        Anchor::Center => content_top + (content_h - total_children_main) / 2.0,
                        Anchor::End => content_bottom - total_children_main,
                    },
                    Direction::Horizontal => match child_anchor.horizontal {
                        Anchor::Start => content_left,
                        Anchor::Center => content_left + (content_w - total_children_main) / 2.0,
                        Anchor::End => content_right - total_children_main,
                    },
                };

                let mut end_extent = cursor;

                for (_id, child) in &mut container.children {

                    // Position on main axis
                    match direction {
                        Direction::Vertical => {
                            child.transform.y = cursor;
                            cursor += child.transform.height + gap;
                            end_extent = child.transform.y + child.transform.height;

                            // Cross-axis alignment
                            child.transform.x = match child_anchor.horizontal {
                                Anchor::Start => content_left,
                                Anchor::Center => content_left + (content_w - child.transform.width) / 2.0,
                                Anchor::End => content_right - child.transform.width,
                            };
                        }
                        Direction::Horizontal => {
                            child.transform.x = cursor;
                            cursor += child.transform.width + gap;
                            end_extent = child.transform.x + child.transform.width;

                            // Cross-axis alignment
                            child.transform.y = match child_anchor.vertical {
                                Anchor::Start => content_top,
                                Anchor::Center => content_top + (content_h - child.transform.height) / 2.0,
                                Anchor::End => content_bottom - child.transform.height,
                            };
                        }
                    }

                    child.transform.apply_transformation();

                    let (child_text_areas, _cv, _ci) = child.node_content_preparation(size, ui, font_system, delta_time);
                    text_areas.extend(child_text_areas);
                }

                // Auto-height: grow to fit children
                if auto_height {
                    let target_bottom = end_extent + margin;
                    let should_lerp = self.transform.smooth_change && !self.transform.fit.vertical;
                    self.transform.rect.bottom = if should_lerp {
                        lerp(self.transform.rect.bottom, target_bottom, delta_time * 20.0)
                    } else {
                        target_bottom
                    };
                    self.transform.height = self.transform.rect.bottom - self.transform.rect.top;
                }
            },
        }

        (text_areas, 0, 0)
    }

    // ── Vertex/Index helpers ──

    fn compute_indices(base: u16) -> [u16; 6] {
        [base, 1 + base, 2 + base, base, 2 + base, 3 + base]
    }

    fn compute_vertices(transform: &UiTransform, visibility: &Visibility, screen_size: &Size) -> [VertexUi; 4] {
        let top = 1.0 - (transform.rect.top / (screen_size.height as f32 / 2.0));
        let left = (transform.rect.left / (screen_size.width as f32 / 2.0)) - 1.0;
        let bottom = 1.0 - (transform.rect.bottom / (screen_size.height as f32 / 2.0));
        let right = (transform.rect.right / (screen_size.width as f32 / 2.0)) - 1.0;

        let rect = [
            transform.rect.top,
            transform.rect.left,
            transform.rect.bottom,
            transform.rect.right,
        ];

        [
            VertexUi { position: vector![left, top, 0.0].into(), color: visibility.background_color, rect, border_color: visibility.border_color },
            VertexUi { position: vector![left, bottom, 0.0].into(), color: visibility.background_color, rect, border_color: visibility.border_color },
            VertexUi { position: vector![right, bottom, 0.0].into(), color: visibility.background_color, rect, border_color: visibility.border_color },
            VertexUi { position: vector![right, top, 0.0].into(), color: visibility.background_color, rect, border_color: visibility.border_color },
        ]
    }

    // ── Construction from RON ──

    pub fn from_component(
        component: &ui_structure::UiComponent,
        font_system: &mut FontSystem,
        screen_width: f32,
        screen_height: f32,
    ) -> Self {
        let width = component.transform.size.as_ref().map(|s| s.width).unwrap_or(0.0);
        let height = component.transform.size.as_ref().map(|s| s.height).unwrap_or(0.0);
        let auto_size = component.transform.size.is_none();

        let x = component.transform.position.x;
        let y = component.transform.position.y;

        // Parse self_anchor
        let self_anchor = component.transform.self_anchor.clone().unwrap_or_default();
        // Parse child_anchor
        let child_anchor = component.transform.child_anchor.clone().unwrap_or_default();
        // Parse direction
        let direction = component.transform.direction.clone().unwrap_or_default();
        // Parse fit
        let fit = component.transform.fit.clone().unwrap_or_default();

        let mut transform = UiTransform::new(x, y, height, width, 0.0, auto_size)
            .with_anchors(self_anchor, child_anchor, direction, fit);

        // Resolve on screen for top-level elements
        transform.resolve_on_screen(screen_width, screen_height);

        let mut visibility = Visibility::new([0.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 0.0]);

        let content = match &component.content {
            ui_structure::UiContent::Label(label_data) => {
                let color = Color::rgba(
                    (label_data.color[0] * 255.0) as u8,
                    (label_data.color[1] * 255.0) as u8,
                    (label_data.color[2] * 255.0) as u8,
                    (label_data.color[3] * 255.0) as u8,
                );
                let align = match label_data.alignment.as_deref() {
                    Some("Center") => Align::Center,
                    Some("Right") => Align::Right,
                    _ => Align::Left,
                };
                let bg = label_data.background_color.unwrap_or([0.0, 0.0, 0.0, 0.0]);
                let border = label_data.border_color.unwrap_or([0.0, 0.0, 0.0, 0.0]);
                visibility = Visibility::new(bg, border);
                UiNodeContent::Text(Label::new(
                    font_system,
                    &label_data.text,
                    width,
                    height,
                    color,
                    align,
                    label_data.font_size,
                ))
            }
            ui_structure::UiContent::Container(container_data) => {
                let bg = container_data.background_color.unwrap_or([0.0, 0.0, 0.0, 0.0]);
                let border = container_data.border_color.unwrap_or([0.0, 0.0, 0.0, 0.0]);
                visibility = Visibility::new(bg, border);
                let margin = container_data.margin.unwrap_or(0.0);
                let gap = container_data.gap.unwrap_or(0.0);

                let children = if let Some(ron_children) = &container_data.children {
                    let mut map = HashMap::new();
                    for (child_id, child_component) in ron_children {
                        map.insert(
                            child_id.clone(),
                            UiNode::from_component(child_component, font_system, screen_width, screen_height),
                        );
                    }
                    map
                } else {
                    HashMap::new()
                };

                UiNodeContent::Container(Container::new(margin, gap, children))
            }
        };

        Self { transform, visibility, content }
    }

    /// Create a label node from code.
    pub fn label(
        font_system: &mut FontSystem,
        text: &str,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        font_size: f32,
        color: Color,
        align: Align,
        background_color: [f32; 4],
        border_color: [f32; 4],
    ) -> Self {
        let transform = UiTransform::new(x, y, height, width, 0.0, false);
        let visibility = Visibility::new(background_color, border_color);
        let content = UiNodeContent::Text(Label::new(font_system, text, width, height, color, align, font_size));
        Self { transform, visibility, content }
    }
}
