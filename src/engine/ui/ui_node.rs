use glyphon::{cosmic_text::Align, Color, FontSystem, TextArea};
use nalgebra::vector;
use crate::app::{App, Size};
use crate::engine::input::input;
use crate::engine::rendering::ui::ui::UiRendering;
use crate::engine::rendering::vertex::{ImageVertex, VertexUi};
use crate::engine::utils::lerps::lerp;
use super::components::label::{Label, DEFAULT_FONT_SIZE};
use super::components::container::Container;
use super::components::image::ImageNode;
use super::ui_transform::{Anchor, ChildAnchor, Orientation, Fit, Padding, PositionValue, Rect, SizeValue, UiTransform};
use super::ui_structure;

pub enum UiNodeContent {
    Text(Label),
    Container(Container),
    Image(ImageNode),
}

// Resolved background+border+radius for a single frame's quad - not stored on
// UiNode, just built fresh from the merged Style each frame (see
// node_content_preparation).
struct Visibility {
    pub background_color: [f32; 4],
    pub border_color: [f32; 4],
    pub corner_radius: f32,
    pub border_width: f32,
}

impl Visibility {
    pub fn new(background_color: [f32; 4], border_color: [f32; 4], corner_radius: f32, border_width: f32) -> Self {
        Self { background_color, border_color, corner_radius, border_width }
    }
}

/// Every stylable property any content type has, one field each, independently
/// optional so a node only has to state what it actually wants to override. Used
/// in two roles: a node's own style (`UiNode.style`), and a hover override layered
/// on top of the resting style (see `UiNode::on_hover`). Deliberately doesn't
/// cascade from a container to its children - each component (a `button()`, a
/// panel, ...) is expected to fully describe its own look via its own `.set_X(...)`
/// calls, not depend on ambient style inherited from wherever it happens to be
/// placed. A field that doesn't apply to a node's content type is simply inert,
/// same as `.set_background_color`/`.set_border_color` already being no-ops where
/// they don't apply.
#[derive(Clone, Default)]
pub struct Style {
    pub background_color: Option<[f32; 4]>,
    pub border_color: Option<[f32; 4]>,
    /// In pixels - universal, same as `border_color`. Unset falls back to `2.0` (the
    /// old hardcoded shader default), not `0.0` - so existing `.set_border_color(...)`
    /// callers that never set a width keep the border they already had.
    pub border_width: Option<f32>,
    /// Corner rounding radius, in pixels - universal, same as background/border
    /// (works on a container's own box or a label's own box, per
    /// `.set_corner_radius(...)`).
    pub corner_radius: Option<f32>,
    /// Text only.
    pub text_color: Option<Color>,
    /// Text only - re-shapes the label's buffer, see `Label::apply_style`.
    pub font_size: Option<f32>,
    /// Text only.
    pub align: Option<Align>,
    /// Image only.
    pub alpha: Option<f32>,
}

impl Style {
    /// Fields this doesn't set fall back to `fallback`'s - every field participates.
    /// Used for layering a hover override on top of a node's resting style (see
    /// `node_content_preparation`), where overriding a child's own background/border
    /// is exactly the point (e.g. highlighting a button on hover).
    fn or(&self, fallback: &Style) -> Style {
        Style {
            background_color: self.background_color.or(fallback.background_color),
            border_color: self.border_color.or(fallback.border_color),
            border_width: self.border_width.or(fallback.border_width),
            corner_radius: self.corner_radius.or(fallback.corner_radius),
            text_color: self.text_color.or(fallback.text_color),
            font_size: self.font_size.or(fallback.font_size),
            align: self.align.or(fallback.align),
            alpha: self.alpha.or(fallback.alpha),
        }
    }

    /// Fills in every field's hard default (opaque white text, transparent
    /// background, no rounding, `DEFAULT_FONT_SIZE`, fully opaque) - the concrete
    /// numbers `.set_transition(...)` actually interpolates between, since you can't
    /// lerp toward `None`. `align` isn't included - it's not meaningfully
    /// interpolable (there's no "halfway between Left and Center"), so it always
    /// snaps instantly, see `node_content_preparation`.
    fn resolve_concrete(&self) -> ResolvedStyle {
        let text_color = self.text_color.unwrap_or(Color::rgba(255, 255, 255, 255));
        let background_color = self.background_color.unwrap_or([0.0, 0.0, 0.0, 0.0]);
        ResolvedStyle {
            background_color,
            // Falls back to the background color, not a hardcoded transparent - the
            // shader (see text_shader.wgsl) always paints a border-width band along
            // the edge in border_color, so a hardcoded transparent default carves a
            // literal hairline of see-through out of every box that has a
            // background but never explicitly set its own border color - subtle on
            // a straight edge, very visible tracing a rounded corner.
            border_color: self.border_color.unwrap_or(background_color),
            border_width: self.border_width.unwrap_or(2.0),
            corner_radius: self.corner_radius.unwrap_or(0.0),
            text_color: [
                text_color.r() as f32 / 255.0,
                text_color.g() as f32 / 255.0,
                text_color.b() as f32 / 255.0,
                text_color.a() as f32 / 255.0,
            ],
            font_size: self.font_size.unwrap_or(DEFAULT_FONT_SIZE),
            alpha: self.alpha.unwrap_or(1.0),
        }
    }
}

fn lerp_rgba(from: [f32; 4], to: [f32; 4], t: f32) -> [f32; 4] {
    [lerp(from[0], to[0], t), lerp(from[1], to[1], t), lerp(from[2], to[2], t), lerp(from[3], to[3], t)]
}

/// Every animatable `Style` field, resolved to a concrete value (no `Option`s left)
/// - what `.set_transition(...)` actually interpolates frame to frame, see
/// `ResolvedStyle::lerp`/`node_content_preparation`. `text_color` is kept as
/// normalized `[f32; 4]` (not `glyphon::Color`) purely so it can share `lerp_rgba`
/// with `background_color`/`border_color` instead of needing its own u8-channel lerp.
#[derive(Clone, Copy)]
struct ResolvedStyle {
    background_color: [f32; 4],
    border_color: [f32; 4],
    border_width: f32,
    corner_radius: f32,
    text_color: [f32; 4],
    font_size: f32,
    alpha: f32,
}

impl ResolvedStyle {
    fn lerp(&self, target: &ResolvedStyle, t: f32) -> ResolvedStyle {
        ResolvedStyle {
            background_color: lerp_rgba(self.background_color, target.background_color, t),
            border_color: lerp_rgba(self.border_color, target.border_color, t),
            border_width: lerp(self.border_width, target.border_width, t),
            corner_radius: lerp(self.corner_radius, target.corner_radius, t),
            text_color: lerp_rgba(self.text_color, target.text_color, t),
            font_size: lerp(self.font_size, target.font_size, t),
            alpha: lerp(self.alpha, target.alpha, t),
        }
    }

    fn text_color(&self) -> Color {
        Color::rgba(
            (self.text_color[0] * 255.0) as u8,
            (self.text_color[1] * 255.0) as u8,
            (self.text_color[2] * 255.0) as u8,
            (self.text_color[3] * 255.0) as u8,
        )
    }
}

pub struct UiNode {
    pub transform: UiTransform,
    pub style: Style,
    pub content: UiNodeContent,
    // When false, this node - and everything under it, recursively - renders
    // nothing, is skipped entirely by its parent's layout (takes up no space, same
    // as CSS `display: none` rather than `visibility: hidden`), and can't be
    // hovered/clicked. See `.active(...)`/`set_active`/`node_content_preparation`.
    pub is_active: bool,
    // Inset between this node's own box edges and its content - for a Container,
    // between its edges and its children (see node_content_preparation); for
    // Text/Image, between its edges and the rendered text/image itself, growing
    // transform.width/height to make room (see apply_padding).
    pub padding: Padding,
    pub hover: Option<Style>,
    // Set by `.set_transition(...)` - how long, in ms, style changes (hover in/out,
    // or a runtime change like set_alpha) take to visually settle instead of
    // applying instantly. `None` (the default) means instant, same as before this
    // existed.
    transition_ms: Option<f32>,
    // The style actually being rendered this frame - equals the resolved target
    // instantly when there's no transition_ms, otherwise eases toward it a little
    // more each frame (see node_content_preparation). `None` until the first frame
    // this node renders, at which point it snaps to the target once (nothing to
    // transition *from* yet) and stays `Some` from then on.
    current_style: Option<ResolvedStyle>,
    // Recomputed every frame in node_content_preparation from the current mouse
    // position - exposed read-only in case callers want to react to it too, not
    // wired up here beyond driving the hover style.
    pub is_hovered: bool,
    // Callback registered via `.on_click(...)`, invoked from `take_click_handlers`
    // (called separately from `node_content_preparation` since firing it needs
    // `&mut App`, which that pass doesn't have - see `App::fire_ui_click_handlers`).
    // Boxed since each node's closure has its own capture, `FnMut` (not `FnOnce`)
    // since it can fire many times over the node's life.
    on_click: Option<Box<dyn FnMut(&mut App)>>,
    // Recomputed every frame alongside is_hovered - true only on the exact frame the
    // click happened (see node_content_preparation), so take_click_handlers firing
    // every frame regardless still only calls on_click once per actual click.
    is_clicked: bool,
    // Set by `.set_size(...)`/`.set_position(...)`, consumed by `resolve(...)` -
    // deferred rather than resolved immediately so a node can be built as a plain
    // value (no closure) before its parent's size is known; `resolve` walks the
    // tree top-down once the whole thing is assembled. `None` means that call was
    // never made - `resolve` then leaves whatever the constructor (`label`/`image`)
    // already set directly untouched.
    pending_size: Option<(SizeValue, SizeValue)>,
    pending_position: Option<(PositionValue, PositionValue)>,
}

fn point_in_rect(x: f32, y: f32, rect: &Rect) -> bool {
    x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom
}

impl UiNode {
    pub const NUM_INDICES: u32 = 6;

    /// Fills in every field every constructor (`label`/`image`/`container`/
    /// `from_component`) needs identically, so they only have to state what's
    /// actually different about them (`transform`/`style`/`content`) instead of
    /// repeating this struct literal four times.
    fn base(transform: UiTransform, style: Style, content: UiNodeContent) -> Self {
        Self {
            transform,
            style,
            content,
            is_active: true,
            padding: Padding::default(),
            hover: None,
            transition_ms: None,
            current_style: None,
            is_hovered: false,
            on_click: None,
            is_clicked: false,
            pending_size: None,
            pending_position: None,
        }
    }

    pub fn as_label_mut(&mut self) -> Option<&mut Label> {
        match &mut self.content {
            UiNodeContent::Text(label) => Some(label),
            _ => None,
        }
    }

    pub fn as_container_mut(&mut self) -> Option<&mut Container> {
        match &mut self.content {
            UiNodeContent::Container(container) => Some(container),
            _ => None,
        }
    }

    /// Fades this node's own content (text alpha or image tint alpha) - has no
    /// effect on containers, which don't have a single color of their own. Used to
    /// animate splash screen elements in/out (see App::run_splash_screen). Text
    /// bakes the alpha straight into `style.text_color`'s alpha channel (keeping its
    /// RGB) since text has no separate alpha channel of its own to blend with;
    /// images use `style.alpha` directly, resolved at render time same as hover.
    pub fn set_alpha(&mut self, alpha: f32) {
        match &mut self.content {
            UiNodeContent::Text(_) => {
                let base = self.style.text_color.unwrap_or(Color::rgba(255, 255, 255, 255));
                let a = (alpha.clamp(0.0, 1.0) * 255.0) as u8;
                self.style.text_color = Some(Color::rgba(base.r(), base.g(), base.b(), a));
            }
            UiNodeContent::Image(_) => self.style.alpha = Some(alpha.clamp(0.0, 1.0)),
            UiNodeContent::Container(_) => {}
        }
    }

    /// Moves this node to an absolute screen position, recomputing its rect. Only
    /// meaningful for top-level nodes (children get repositioned by their parent
    /// container every frame anyway) - used to slide splash screen elements in.
    /// Named distinctly from `.set_position(...)` (the richer, deferred
    /// `PositionValue` builder) on purpose - this is an immediate, raw-pixel runtime
    /// mutator meant for a node you already have a `&mut` to (e.g. via
    /// `Ui::get_ui_node`), not a construction-time builder call.
    pub fn move_to(&mut self, x: f32, y: f32) {
        self.transform.x = x;
        self.transform.y = y;
        self.transform.apply_transformation();
    }

    /// Chainable wrapper over `move_to` for construction-time use, e.g.
    /// `UiNode::label(...).at(x, y).set_text_color(...)`.
    pub fn at(mut self, x: f32, y: f32) -> Self {
        self.move_to(x, y);
        self
    }

    /// Shows/hides this node (and, recursively, everything under it) - not rendered,
    /// not hovered/clicked, and skipped entirely by its parent's layout while
    /// inactive (see `node_content_preparation`). Resets `is_hovered`/the internal
    /// click flag when turning inactive, so a node deactivated mid-hover doesn't
    /// keep reporting stale state to anything reading `.is_hovered` directly.
    pub fn set_active(&mut self, active: bool) {
        self.is_active = active;
        if !active {
            self.is_hovered = false;
            self.is_clicked = false;
        }
    }

    /// Chainable wrapper over `set_active` for construction-time use, e.g.
    /// `UiNode::container().active(false)`.
    pub fn active(mut self, active: bool) -> Self {
        self.set_active(active);
        self
    }

    pub fn get_children_mut(&mut self) -> Option<&mut Vec<(String, UiNode)>> {
        match &mut self.content {
            UiNodeContent::Container(container) => Some(&mut container.children),
            _ => None,
        }
    }

    /// A node's own box, inset by `padding` on every side - what the actual content
    /// (label text, an image) renders into, as opposed to `transform.rect` itself
    /// which is the *outer* box the background/border quad fills. Shared by the Text
    /// and Image branches of `node_content_preparation`. Takes `transform`/`padding`
    /// as plain parameters rather than being a `&self` method - the call sites need
    /// it while `self.content` is already mutably borrowed (via `match &mut
    /// self.content`), and a method call borrows all of `self` as far as the borrow
    /// checker is concerned, unlike reading two specific fields directly.
    fn inner_rect(transform: &UiTransform, padding: &Padding) -> Rect {
        Rect {
            top: transform.rect.top + padding.top,
            left: transform.rect.left + padding.left,
            bottom: transform.rect.bottom - padding.bottom,
            right: transform.rect.right - padding.right,
        }
    }

    // ── Rendering ──

    pub fn node_content_preparation(&mut self, size: &Size, ui: &mut UiRendering, font_system: &mut FontSystem, delta_time: f32) -> (Vec<TextArea>, u16, u32) {
        // Inactive nodes render nothing and can't be hovered/clicked - their
        // *parent's* layout loop is what skips giving them any space (this alone
        // wouldn't stop them occupying a stacking slot), see the Container branch
        // below.
        if !self.is_active {
            self.is_hovered = false;
            self.is_clicked = false;
            return (Vec::new(), 0, 0);
        }

        let mut text_areas: Vec<TextArea> = Vec::new();

        // Hover hit-test, redone fresh every frame from the live mouse position and
        // this node's already-resolved rect - naturally stays correct as the node
        // moves/resizes or the mouse moves, no caching/invalidation needed.
        // is_clicked piggybacks on the same hit-test - is_action_just_pressed is
        // itself already edge-triggered (true for exactly the one frame the button
        // goes down), so this only needs "was the mouse over this node this frame",
        // not any click-specific state of its own.
        let mouse_over = point_in_rect(input::mouse_x() as f32, input::mouse_y() as f32, &self.transform.rect);
        self.is_hovered = self.hover.is_some() && mouse_over;

        // Hover's Some(...) fields win over the resting style, its None fields fall
        // back to it - built fresh every frame (not written back into self.style),
        // so un-hovering next frame just goes back to reading the untouched resting
        // style, no restore/undo bookkeeping needed.
        let effective_style = if self.is_hovered {
            self.hover.as_ref().unwrap().or(&self.style)
        } else {
            self.style.clone()
        };
        self.is_clicked = mouse_over && input::is_action_just_pressed("ui_click");

        // The target this frame's animatable fields are heading toward (or already
        // at, with no .set_transition(...) set) - see ResolvedStyle/UiNode.current_style.
        let target = effective_style.resolve_concrete();
        let current = match (self.current_style, self.transition_ms) {
            (Some(current), Some(ms)) if ms > 0.0 => {
                let t = (delta_time / (ms / 1000.0)).min(1.0);
                current.lerp(&target, t)
            }
            _ => target,
        };
        self.current_style = Some(current);

        let effective_visibility = Visibility::new(current.background_color, current.border_color, current.corner_radius, current.border_width);

        match &mut self.content {
            UiNodeContent::Text(label) => {
                // Background/border quad still fills the whole (already padding-grown,
                // see apply_padding) box - only the text itself renders inset within it.
                let vertices_slice = Self::compute_vertices(&self.transform, &effective_visibility, size);
                let indice_slice = Self::compute_indices(ui.num_vertices);
                let inner_rect = Self::inner_rect(&self.transform, &self.padding);
                label.buffer.set_size(font_system, Some(inner_rect.right - inner_rect.left), Some(inner_rect.bottom - inner_rect.top));
                // align isn't part of ResolvedStyle/current - not meaningfully
                // interpolable, so it always uses this frame's target directly.
                label.apply_style(font_system, current.font_size, effective_style.align.unwrap_or(Align::Left));

                let (text_area, added_vertices, added_indices) = label.ui_node_data_creation(size, &mut ui.vertices, &vertices_slice, &mut ui.indices, &indice_slice, &inner_rect, current.text_color());
                text_areas.push(text_area);
                ui.num_vertices += added_vertices;
                ui.num_indices += added_indices;
            },
            UiNodeContent::Container(container) => {
                let auto_width = self.transform.width == 0.0 || self.transform.fit.horizontal;
                let auto_height = self.transform.height == 0.0 || self.transform.fit.vertical;
                // Inactive children are skipped everywhere below - not measured for
                // Fit-sizing, not counted for gaps, not given a slot in the stacking
                // cursor, and never have node_content_preparation called on them (so
                // they render nothing) - same as CSS `display: none`, not
                // `visibility: hidden` (which would still reserve their space).
                let active_count = container.children.iter().filter(|(_, c)| c.is_active).count();

                // Fit/auto-size width: measure children first, then grow from
                // whichever edge self_anchor didn't pin - an End/Center-anchored axis
                // resolved (see resolve_position) against a 0.0 placeholder size, so
                // growing right from rect.left unconditionally would grow away from
                // the corner the node was actually anchored to.
                if auto_width {
                    // Same Vertical/Horizontal split as auto_height below (sum
                    // along the main axis, max along the cross axis) - children
                    // stack along width for Horizontal, so that's the sum here;
                    // for Vertical they're side by side only in the sense that
                    // each sits at its own cross-axis offset, so the widest one
                    // determines the container's width.
                    let natural_width: f32 = match self.transform.direction {
                        Orientation::Horizontal => container.children.iter().filter(|(_, c)| c.is_active).map(|(_, c)| c.transform.width).sum::<f32>()
                            + container.gap * (active_count.saturating_sub(1) as f32),
                        Orientation::Vertical => container.children.iter().filter(|(_, c)| c.is_active).map(|(_, c)| c.transform.width).fold(0.0f32, |a, b| a.max(b)),
                    };
                    let new_width = natural_width + self.padding.left + self.padding.right;
                    self.transform.width = new_width;
                    match self.transform.self_anchor.horizontal {
                        Anchor::End => self.transform.rect.left = self.transform.rect.right - new_width,
                        Anchor::Center => {
                            let center_x = (self.transform.rect.left + self.transform.rect.right) / 2.0;
                            self.transform.rect.left = center_x - new_width / 2.0;
                            self.transform.rect.right = center_x + new_width / 2.0;
                        }
                        Anchor::Start => self.transform.rect.right = self.transform.rect.left + new_width,
                    }
                }

                // Same idea for an End/Center-anchored height, but it has to happen
                // before children are laid out below (unlike the Start case further
                // down, which sizes from where children actually ended up after
                // layout) - otherwise children would be positioned relative to a
                // rect.top that's about to move, leaving them visually detached from
                // their own container's background this frame.
                if auto_height && self.transform.self_anchor.vertical != Anchor::Start {
                    let direction = &self.transform.direction;
                    let natural_height: f32 = match direction {
                        Orientation::Vertical => container.children.iter().filter(|(_, c)| c.is_active).map(|(_, c)| c.transform.height).sum::<f32>()
                            + container.gap * (active_count.saturating_sub(1) as f32),
                        Orientation::Horizontal => container.children.iter().filter(|(_, c)| c.is_active).map(|(_, c)| c.transform.height).fold(0.0f32, |a, b| a.max(b)),
                    };
                    let new_height = natural_height + self.padding.top + self.padding.bottom;
                    self.transform.height = new_height;
                    match self.transform.self_anchor.vertical {
                        Anchor::End => self.transform.rect.top = self.transform.rect.bottom - new_height,
                        Anchor::Center => {
                            let center_y = (self.transform.rect.top + self.transform.rect.bottom) / 2.0;
                            self.transform.rect.top = center_y - new_height / 2.0;
                            self.transform.rect.bottom = center_y + new_height / 2.0;
                        }
                        Anchor::Start => unreachable!(),
                    }
                }

                // Render container background
                let vertices_slice = Self::compute_vertices(&self.transform, &effective_visibility, size);
                let indice_slice = Self::compute_indices(ui.num_vertices);
                let (cv, ci) = container.ui_node_data_creation(size, &mut ui.vertices, &vertices_slice, &mut ui.indices, &indice_slice);
                ui.num_vertices += cv;
                ui.num_indices += ci;

                // Layout children
                let parent_rect = &self.transform.rect.clone();
                let direction = &self.transform.direction.clone();
                let child_anchor = &self.transform.child_anchor.clone();
                let padding = self.padding;
                let gap = container.gap;

                let content_left = parent_rect.left + padding.left;
                let content_top = parent_rect.top + padding.top;
                let content_right = parent_rect.right - padding.right;
                let content_bottom = parent_rect.bottom - padding.bottom;
                let content_w = content_right - content_left;
                let content_h = content_bottom - content_top;

                // Calculate total children size for centering/end alignment on main axis
                let total_children_main: f32 = container.children.iter()
                    .filter(|(_, c)| c.is_active)
                    .map(|(_, c)| match direction {
                        Orientation::Vertical => c.transform.height,
                        Orientation::Horizontal => c.transform.width,
                    })
                    .sum::<f32>() + gap * (active_count.saturating_sub(1) as f32);

                // Starting cursor on main axis based on child_anchor
                let mut cursor = match direction {
                    Orientation::Vertical => match child_anchor.vertical {
                        Anchor::Start => content_top,
                        Anchor::Center => content_top + (content_h - total_children_main) / 2.0,
                        Anchor::End => content_bottom - total_children_main,
                    },
                    Orientation::Horizontal => match child_anchor.horizontal {
                        Anchor::Start => content_left,
                        Anchor::Center => content_left + (content_w - total_children_main) / 2.0,
                        Anchor::End => content_right - total_children_main,
                    },
                };

                let mut end_extent = cursor;

                for (_id, child) in &mut container.children {
                    if !child.is_active {
                        // Never call node_content_preparation on it either - it
                        // renders nothing and takes no space, see the is_active
                        // check at the top of this function.
                        continue;
                    }

                    // Position on main axis
                    match direction {
                        Orientation::Vertical => {
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
                        Orientation::Horizontal => {
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

                // Auto-height for the Start case (grows down from a fixed top, using
                // where children actually ended up post-layout) - End/Center already
                // handled above, before children were positioned.
                if auto_height && self.transform.self_anchor.vertical == Anchor::Start {
                    let target_bottom = end_extent + padding.bottom;
                    let should_lerp = self.transform.smooth_change && !self.transform.fit.vertical;
                    self.transform.rect.bottom = if should_lerp {
                        lerp(self.transform.rect.bottom, target_bottom, delta_time * 20.0)
                    } else {
                        target_bottom
                    };
                    self.transform.height = self.transform.rect.bottom - self.transform.rect.top;
                }
            },
            UiNodeContent::Image(image) => {
                // Queued by path (not drawn here) - Ui batches every node sharing the
                // same image into one draw call, see prepare_ui_content/render_ui_pass.
                // The image itself renders inset within the (already padding-grown,
                // see apply_padding) box - same treatment as Text, though there's no
                // background quad behind an Image node today to show through the gap.
                let inner_rect = Self::inner_rect(&self.transform, &self.padding);
                let vertices = Self::compute_image_vertices(&inner_rect, size, current.alpha);
                ui.image_quads.entry(image.path.clone()).or_default().extend_from_slice(&vertices);
            },
        }

        (text_areas, 0, 0)
    }

    /// Debug-only: draws an outline (transparent fill, opaque border - reuses the
    /// same border rendering every node's own border color already gets, see
    /// text_shader.wgsl) at this node's resolved `rect`, recursing into children.
    /// Call after `node_content_preparation` has already run this frame (needs
    /// `self.transform.rect` to be current) - see `App::prepare_ui_content`, gated
    /// behind `Ui::debug_bounds` (toggled by "toggle_ui_debug", F2).
    pub fn debug_bounds_preparation(&self, size: &Size, ui: &mut UiRendering) {
        if !self.is_active {
            return;
        }
        let outline = Visibility::new([0.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 1.0], 0.0, 2.0);
        let vertices_slice = Self::compute_vertices(&self.transform, &outline, size);
        let indice_slice = Self::compute_indices(ui.num_vertices);

        ui.vertices.extend_from_slice(&vertices_slice);
        ui.indices.extend_from_slice(&indice_slice);
        ui.num_vertices += vertices_slice.len() as u16;
        ui.num_indices += Self::NUM_INDICES;

        if let UiNodeContent::Container(container) = &self.content {
            for (_, child) in &container.children {
                child.debug_bounds_preparation(size, ui);
            }
        }
    }

    /// Recursively takes (see `Option::take`) the `.on_click(...)` closure out of
    /// every node that was clicked this frame (see `is_clicked`, set in
    /// `node_content_preparation`), pairing each with its `"/"`-separated path (see
    /// `Ui::get_ui_node`) - `path` is this node's own path, extended with each
    /// child's id as the walk descends.
    ///
    /// This only ever needs `&mut UiNode`, never `&mut App` - firing the collected
    /// closures (and restoring them afterward via `restore_click_handler`) is a
    /// separate step, see `App::fire_ui_click_handlers`. Splitting it this way -
    /// instead of just calling `on_click(app)` straight from here - means
    /// `app.ui.renderizable_elements` is never emptied or otherwise touched while a
    /// handler runs, so a handler can freely look up and mutate *any* UI node via
    /// `Ui::get_ui_node`, including its own node or one of its ancestors (e.g. a
    /// button hiding the panel it's inside of).
    pub(crate) fn take_click_handlers(&mut self, path: &str, out: &mut Vec<(String, Box<dyn FnMut(&mut App)>)>) {
        if !self.is_active {
            return;
        }
        if self.is_clicked {
            if let Some(on_click) = self.on_click.take() {
                out.push((path.to_owned(), on_click));
            }
        }
        if let UiNodeContent::Container(container) = &mut self.content {
            for (id, child) in &mut container.children {
                child.take_click_handlers(&format!("{path}/{id}"), out);
            }
        }
    }

    /// Puts an `.on_click(...)` closure taken by `take_click_handlers` back, so
    /// future clicks keep working - see `App::fire_ui_click_handlers`.
    pub(crate) fn restore_click_handler(&mut self, on_click: Box<dyn FnMut(&mut App)>) {
        self.on_click = Some(on_click);
    }

    // ── Vertex/Index helpers ──

    fn compute_image_vertices(rect: &Rect, screen_size: &Size, alpha: f32) -> [ImageVertex; 4] {
        let top = 1.0 - (rect.top / (screen_size.height as f32 / 2.0));
        let left = (rect.left / (screen_size.width as f32 / 2.0)) - 1.0;
        let bottom = 1.0 - (rect.bottom / (screen_size.height as f32 / 2.0));
        let right = (rect.right / (screen_size.width as f32 / 2.0)) - 1.0;

        [
            ImageVertex { position: vector![left, top, 0.0].into(), uv: [0.0, 0.0], alpha },
            ImageVertex { position: vector![left, bottom, 0.0].into(), uv: [0.0, 1.0], alpha },
            ImageVertex { position: vector![right, bottom, 0.0].into(), uv: [1.0, 1.0], alpha },
            ImageVertex { position: vector![right, top, 0.0].into(), uv: [1.0, 0.0], alpha },
        ]
    }

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
            VertexUi { position: vector![left, top, 0.0].into(), color: visibility.background_color, rect, border_color: visibility.border_color, corner_radius: visibility.corner_radius, border_width: visibility.border_width },
            VertexUi { position: vector![left, bottom, 0.0].into(), color: visibility.background_color, rect, border_color: visibility.border_color, corner_radius: visibility.corner_radius, border_width: visibility.border_width },
            VertexUi { position: vector![right, bottom, 0.0].into(), color: visibility.background_color, rect, border_color: visibility.border_color, corner_radius: visibility.corner_radius, border_width: visibility.border_width },
            VertexUi { position: vector![right, top, 0.0].into(), color: visibility.background_color, rect, border_color: visibility.border_color, corner_radius: visibility.corner_radius, border_width: visibility.border_width },
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

        let mut style = Style::default();
        let mut padding = Padding::default();

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
                style.background_color = Some(label_data.background_color.unwrap_or([0.0, 0.0, 0.0, 0.0]));
                style.border_color = Some(label_data.border_color.unwrap_or([0.0, 0.0, 0.0, 0.0]));
                style.text_color = Some(color);
                style.font_size = Some(label_data.font_size);
                style.align = Some(align);
                UiNodeContent::Text(Label::new(
                    font_system,
                    &label_data.text,
                    width,
                    height,
                ))
            }
            ui_structure::UiContent::Container(container_data) => {
                style.background_color = Some(container_data.background_color.unwrap_or([0.0, 0.0, 0.0, 0.0]));
                style.border_color = Some(container_data.border_color.unwrap_or([0.0, 0.0, 0.0, 0.0]));
                padding = Padding::all(container_data.margin.unwrap_or(0.0));
                let gap = container_data.gap.unwrap_or(0.0);

                let children = if let Some(ron_children) = &container_data.children {
                    // Note: ron_children is itself a HashMap<String, UiComponent> (see
                    // ui_structure.rs), so RON-declared children still aren't ordered -
                    // only code-first .set_child(...) is fixed by Container::children
                    // being a Vec. Fixing RON too would need its schema to change from
                    // a map to a list, which would break every existing .ron UI file.
                    let mut children = Vec::new();
                    for (child_id, child_component) in ron_children {
                        children.push((
                            child_id.clone(),
                            UiNode::from_component(child_component, font_system, screen_width, screen_height),
                        ));
                    }
                    children
                } else {
                    Vec::new()
                };

                UiNodeContent::Container(Container::new(gap, children))
            }
            ui_structure::UiContent::Image(image_data) => {
                UiNodeContent::Image(ImageNode::new(image_data.path.clone()))
            }
        };

        let mut node = Self::base(transform, style, content);
        node.padding = padding;
        node
    }

    /// Create a label node from code, sized to fit `text` unless `width`/`height`
    /// are given explicitly - kept as constructor params (not deferred like
    /// `.set_size(...)`) since the label's `Buffer` needs a real size immediately to
    /// measure/shape its text, and this constructor is also used by call sites that
    /// never go through `Layer`/`resolve` (e.g. the splash screen, free camera HUD).
    /// Everything else - position, color, font size, alignment, background, border,
    /// hover, click - is set through the builder chain, e.g.
    /// `UiNode::label(font_system, "Play", None, None).at(40.0, 40.0).set_text_color(...)`.
    pub fn label(font_system: &mut FontSystem, text: &str, width: Option<f32>, height: Option<f32>) -> Self {
        let (width, height) = Label::measure_or(font_system, text, width, height);
        let transform = UiTransform::new(0.0, 0.0, height, width, 0.0, false);
        let content = UiNodeContent::Text(Label::new(font_system, text, width, height));
        Self::base(transform, Style::default(), content)
    }

    /// Create an image node from code - `width`/`height` are required (unlike
    /// `label`, there's no natural size to auto-measure without a loaded texture).
    /// The image itself must already be loaded (see Ui::load_image) before this node
    /// can draw anything. Chain `.at(x, y)` for position.
    pub fn image(path: String, width: f32, height: f32) -> Self {
        let transform = UiTransform::new(0.0, 0.0, height, width, 0.0, false);
        let content = UiNodeContent::Image(ImageNode::new(path));
        Self::base(transform, Style::default(), content)
    }

    /// Create an empty container node from code, positioned at the origin and sized
    /// to fit its children (`SizeValue::Fit` on both axes) by default - configure
    /// both through the builder instead of the constructor: `.set_size(...)` then
    /// `.set_position(...)`. Chain `.set_child(...)`/`.set_padding(...)`/etc. to
    /// fill it in, then hand the result to `Ui::add_to_ui`. No RON file needed - the
    /// per-frame layout pass (`node_content_preparation`) doesn't care whether a
    /// node came from RON or code, so containers built this way lay out and render
    /// identically either way.
    pub fn container() -> Self {
        let mut transform = UiTransform::new(0.0, 0.0, 0.0, 0.0, 0.0, false);
        transform.fit.horizontal = true;
        transform.fit.vertical = true;
        let content = UiNodeContent::Container(Container::new(0.0, Vec::new()));
        Self::base(transform, Style::default(), content)
    }

    /// Sets this node's size from a richer value than a raw pixel - `Pixels`,
    /// `Percent` (of the parent's size), or `Fit` (size to children, containers
    /// only). Not resolved here - just recorded, since the parent's size usually
    /// isn't known yet at the point a node is being built as a plain value (see
    /// `resolve`, which every node goes through exactly once, top-down, from
    /// `Layer::build`). `Percent`/`Fit` on a grandchild still resolves against `0.0`
    /// if the immediate parent is itself `Fit`-sized - its real size isn't known
    /// until the per-frame layout pass runs - use `SizeValue::Pixels` there for now.
    pub fn set_size(mut self, width: SizeValue, height: SizeValue) -> Self {
        self.pending_size = Some((width, height));
        self
    }

    /// Sets this node's position from a richer value than a raw pixel offset -
    /// `Percent`, or `Start`/`Center`/`End` (each anchored to that edge/the middle,
    /// plus its own pixel offset - e.g. `End(-10.0)` sits 10px in from the far edge).
    /// Same deferred-to-`resolve` handling as `.set_size(...)` - `Center`/`End`
    /// measure from the size `.set_size(...)` recorded, so call that first if using
    /// both (order between the two calls doesn't matter to `resolve` itself, but
    /// does to what `Center`/`End` measure against).
    pub fn set_position(mut self, x: PositionValue, y: PositionValue) -> Self {
        self.pending_position = Some((x, y));
        self
    }

    /// Resolves this node's pending `.set_size(...)`/`.set_position(...)` (if either
    /// was called) against `parent_width`/`parent_height`, then recurses into
    /// children (containers only) using this node's own just-resolved width/height
    /// as their parent size. Runs exactly once per node, top-down, starting from
    /// `Layer::build` - not the same thing as the per-frame layout pass in
    /// `node_content_preparation`, which repositions children within their
    /// container's padding/gap/orientation every frame regardless.
    pub fn resolve(&mut self, parent_width: f32, parent_height: f32) {
        if let Some((width, height)) = self.pending_size {
            self.transform.resolve_size(width, height, parent_width, parent_height);
        }
        if let Some((x, y)) = self.pending_position {
            self.transform.resolve_position(x, y, parent_width, parent_height);
        }
        if let UiNodeContent::Container(container) = &mut self.content {
            let (width, height) = (self.transform.width, self.transform.height);
            for (_, child) in &mut container.children {
                child.resolve(width, height);
            }
        }
    }

    /// Sets the style overrides this node uses while the mouse is over it (see
    /// `Style`, checked fresh every frame in `node_content_preparation`) - works on
    /// any node type, same as `.set_background_color`/`.set_border_color`.
    pub fn on_hover(mut self, hover: Style) -> Self {
        self.hover = Some(hover);
        self
    }

    /// Smooths style changes (hover in/out, or a runtime change like `set_alpha`)
    /// over `ms` milliseconds instead of snapping instantly - like Tailwind's
    /// `transition` utility. Approximate rather than a strict linear/eased duration:
    /// each frame moves `delta_time / (ms / 1000)` of the *remaining* gap toward the
    /// target (an exponential ease-out, same style of smoothing this file already
    /// uses for auto-height, see `smooth_change`), so `ms` reads as "about how long
    /// this takes to settle" rather than an exact stopwatch duration - but it means
    /// the target can change again mid-transition (e.g. hovering on/off quickly)
    /// with no jank, since there's no fixed start/end snapshot to invalidate.
    /// Covers `background_color`/`border_color`/`text_color`/`font_size`/`alpha` -
    /// `align` always snaps instantly, see `Style::resolve_concrete`.
    pub fn set_transition(mut self, ms: f32) -> Self {
        self.transition_ms = Some(ms);
        self
    }

    /// Registers a callback that runs once, on the frame this node gets clicked
    /// (mouse button pressed while over it - see the `is_clicked` hit-test in
    /// `node_content_preparation`, and `App::fire_ui_click_handlers` for how it
    /// actually gets called with `&mut App`). Same `ui.has_changed = true` every
    /// frame requirement as `.on_hover(...)` - the hit-test only runs when the UI
    /// pass does. Most closures won't need to capture anything - they just act
    /// directly on `app`, e.g. `.on_click(|app| app.scene_manager.open_scene("playing"))`.
    pub fn on_click(mut self, f: impl FnMut(&mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }

    /// Inserts `child` under `id`, in the order this is called - no-op if this node
    /// isn't a container. Replaces the existing entry in place (same position) if
    /// `id` was already used, same as a map's `insert` would. `id` still matters
    /// even though nothing cascades through it anymore (see `Style`'s doc comment) -
    /// `Ui::get_ui_node`'s `"a/b/c"`-style path lookups depend on every child being
    /// addressable.
    ///
    /// `child` is a plain, already-built `UiNode` - any `.set_size(...)`/
    /// `.set_position(...)` it carries is still just pending at this point (see
    /// `resolve`), so building it doesn't need this container's size to be known yet.
    pub fn set_child(mut self, id: impl Into<String>, child: UiNode) -> Self {
        if let UiNodeContent::Container(container) = &mut self.content {
            let id = id.into();
            match container.children.iter_mut().find(|(existing_id, _)| *existing_id == id) {
                Some(entry) => entry.1 = child,
                None => container.children.push((id, child)),
            }
        }
        self
    }

    /// Works on any node type - inert on content types other than Text.
    pub fn set_text_color(mut self, color: Color) -> Self {
        self.style.text_color = Some(color);
        self
    }

    /// Works on any node type - inert on content types other than Text.
    pub fn set_font_size(mut self, size: f32) -> Self {
        self.style.font_size = Some(size);
        self
    }

    /// Works on any node type - inert on content types other than Text.
    pub fn set_align(mut self, align: Align) -> Self {
        self.style.align = Some(align);
        self
    }

    /// Updates this node's padding - works on any node type, unlike most other
    /// container-flavored builders. Accepts any of three shapes (see `Padding`'s
    /// `From` impls): `.set_padding(10.0)` (every side), `.set_padding((10.0, 2.0))`
    /// (`(x, y)` - left+right, top+bottom), or `.set_padding([10.0, 2.0, 4.0, 4.0])`
    /// (`[top, bottom, left, right]`).
    ///
    /// For a Container, only insets its *children* from its own edges (see
    /// `node_content_preparation`) - its own size already comes from
    /// `.set_size(...)`/fitting to those children, so padding doesn't touch it here.
    /// For Text/Image, there's no child to inset, so instead this grows/shrinks
    /// `self.transform`'s own width/height by exactly the delta between the old and
    /// new padding (see `apply_padding`) - e.g. a label auto-measured to fit its
    /// text actually gets visibly bigger once padded, with the content itself then
    /// rendering inset within that bigger box (see `inner_rect`).
    pub fn set_padding(mut self, padding: impl Into<Padding>) -> Self {
        self.apply_padding(padding.into());
        self
    }

    /// The actual padding-applying logic `set_padding` calls into - see its doc
    /// comment. Computing the *delta* between old and new padding (rather than just
    /// adding the new value) is what makes a later `.set_padding(...)` call
    /// overriding an earlier one not double-count the box growth.
    fn apply_padding(&mut self, new_padding: Padding) {
        if !matches!(self.content, UiNodeContent::Container(_)) {
            let dw = (new_padding.left - self.padding.left) + (new_padding.right - self.padding.right);
            let dh = (new_padding.top - self.padding.top) + (new_padding.bottom - self.padding.bottom);
            self.transform.width += dw;
            self.transform.height += dh;
            self.transform.apply_transformation();
        }
        self.padding = new_padding;
    }

    /// No-op if this node isn't a container.
    pub fn set_gap(mut self, gap: f32) -> Self {
        if let UiNodeContent::Container(container) = &mut self.content {
            container.gap = gap;
        }
        self
    }

    /// Sets whether this container stacks its children horizontally or vertically -
    /// like a flex container's `flex-direction`. Only meaningful for containers, but
    /// it's a plain `UiTransform` field so this is harmless to call on any node type.
    pub fn set_orientation(mut self, orientation: Orientation) -> Self {
        self.transform.direction = orientation;
        self
    }

    /// Works on any node type - `style` isn't container-specific.
    pub fn set_background_color(mut self, color: [f32; 4]) -> Self {
        self.style.background_color = Some(color);
        self
    }

    /// Works on any node type - `style` isn't container-specific.
    pub fn set_border_color(mut self, color: [f32; 4]) -> Self {
        self.style.border_color = Some(color);
        self
    }

    /// Sets this node's border thickness in pixels - works on any node type, same
    /// as `.set_border_color(...)`. Defaults to `2.0` if never called, matching the
    /// old hardcoded shader value. Animates with `.set_transition(...)` like every
    /// other style field.
    pub fn set_border_width(mut self, width: f32) -> Self {
        self.style.border_width = Some(width);
        self
    }

    /// Rounds this node's own box corners by `radius` pixels - works on any node
    /// type, same as `.set_background_color`/`.set_border_color` (a container's own
    /// box, or a label's/image's). Clamped in the shader to at most half the box's
    /// shorter side, so an oversized radius just caps out at a stadium/circle
    /// instead of producing garbage. Animates with `.set_transition(...)` like every
    /// other style field.
    pub fn set_corner_radius(mut self, radius: f32) -> Self {
        self.style.corner_radius = Some(radius);
        self
    }
}
