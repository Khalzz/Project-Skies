use glyphon::{cosmic_text::Align, Color as GlyphonColor, FontSystem, TextArea};
use nalgebra::vector;
use crate::app::{App, Size};
use crate::engine::input::input;
use crate::engine::rendering::ui::ui::UiRendering;
use crate::engine::rendering::vertex::{ImageVertex, VertexUi};
use crate::engine::utils::lerps::lerp;
use super::color::{lerp_rgba, Fill, UiColor};
use super::components::label::{Label, DEFAULT_FONT_SIZE, min_height_for_font_size};
use super::components::container::Container;
use super::components::image::ImageNode;
use super::ui_transform::{Anchor, BorderEdges, ChildAnchor, Orientation, Fit, Padding, PositionValue, Rect, SizeValue, UiTransform};
use super::ui_structure;

pub enum UiNodeContent {
    Text(Label),
    Container(Container),
    Image(ImageNode),
}

// Resolved background+border+radius for a single frame's quad - not stored on
// UiNode, just built fresh from the merged Style each frame (see
// node_content_preparation). background_color/border_color are the actual `Fill`
// (not pre-flattened to a handful of colors) so compute_vertices has everything
// it needs to subdivide the quad for a multi-stop gradient (see
// Fill::axis_split) - a Fill::Solid still renders as a single plain quad.
struct Visibility {
    pub background_color: Fill,
    pub border_color: Fill,
    pub corner_radius: f32,
    pub border_width: f32,
    pub border_edges: BorderEdges,
    pub background_blur: f32,
}

impl Visibility {
    pub fn new(background_color: Fill, border_color: Fill, corner_radius: f32, border_width: f32, border_edges: BorderEdges, background_blur: f32) -> Self {
        Self { background_color, border_color, corner_radius, border_width, border_edges, background_blur }
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
    pub background_color: Option<Fill>,
    pub border_color: Option<Fill>,
    /// In pixels - universal, same as `border_color`. Unset falls back to `2.0` (the
    /// old hardcoded shader default), not `0.0` - so existing `.set_border_color(...)`
    /// callers that never set a width keep the border they already had.
    pub border_width: Option<f32>,
    /// Which sides actually draw a border - universal, same as `border_color`.
    /// Unset falls back to `BorderEdges::ALL` (every side) - see its own doc
    /// comment.
    pub border_edges: Option<BorderEdges>,
    /// Corner rounding radius, in pixels - universal, same as background/border
    /// (works on a container's own box or a label's own box, per
    /// `.set_corner_radius(...)`).
    pub corner_radius: Option<f32>,
    /// Text only.
    pub text_color: Option<UiColor>,
    /// Text only - re-shapes the label's buffer, see `Label::apply_style`.
    pub font_size: Option<f32>,
    /// Text only.
    pub align: Option<Align>,
    /// Image only.
    pub alpha: Option<f32>,
    /// Backdrop blur radius in pixels - CSS `backdrop-filter: blur()`, universal
    /// same as `background_color` itself. `0.0` (unset) is off, exactly like before
    /// this existed. See `UiNode::set_background_blur` for the full picture,
    /// including how this combines with `background_color`'s own alpha.
    pub background_blur: Option<f32>,
}

impl Style {
    /// Fields this doesn't set fall back to `fallback`'s - every field participates.
    /// Used for layering a hover override on top of a node's resting style (see
    /// `node_content_preparation`), where overriding a child's own background/border
    /// is exactly the point (e.g. highlighting a button on hover).
    fn or(&self, fallback: &Style) -> Style {
        Style {
            background_color: self.background_color.clone().or_else(|| fallback.background_color.clone()),
            border_color: self.border_color.clone().or_else(|| fallback.border_color.clone()),
            border_width: self.border_width.or(fallback.border_width),
            border_edges: self.border_edges.or(fallback.border_edges),
            corner_radius: self.corner_radius.or(fallback.corner_radius),
            text_color: self.text_color.or(fallback.text_color),
            font_size: self.font_size.or(fallback.font_size),
            align: self.align.or(fallback.align),
            alpha: self.alpha.or(fallback.alpha),
            background_blur: self.background_blur.or(fallback.background_blur),
        }
    }

    /// Fills in every field's hard default (opaque white text, transparent
    /// background, no rounding, `DEFAULT_FONT_SIZE`, fully opaque) - the concrete
    /// numbers `.set_transition(...)` actually interpolates between, since you can't
    /// lerp toward `None`. `align` isn't included - it's not meaningfully
    /// interpolable (there's no "halfway between Left and Center"), so it always
    /// snaps instantly, see `node_content_preparation`.
    fn resolve_concrete(&self) -> ResolvedStyle {
        let background_color = self.background_color.clone().unwrap_or(Fill::Solid(UiColor::TRANSPARENT));
        ResolvedStyle {
            // Falls back to the background color, not a hardcoded transparent - the
            // shader (see text_shader.wgsl) always paints a border-width band along
            // the edge in border_color, so a hardcoded transparent default carves a
            // literal hairline of see-through out of every box that has a
            // background but never explicitly set its own border color - subtle on
            // a straight edge, very visible tracing a rounded corner.
            border_color: self.border_color.clone().unwrap_or_else(|| background_color.clone()),
            background_color,
            border_width: self.border_width.unwrap_or(2.0),
            corner_radius: self.corner_radius.unwrap_or(0.0),
            text_color: self.text_color.unwrap_or(UiColor::WHITE).into(),
            font_size: self.font_size.unwrap_or(DEFAULT_FONT_SIZE),
            alpha: self.alpha.unwrap_or(1.0),
            background_blur: self.background_blur.unwrap_or(0.0),
        }
    }
}

fn lerp_corners(from: [[f32; 4]; 4], to: [[f32; 4]; 4], t: f32) -> [[f32; 4]; 4] {
    std::array::from_fn(|i| lerp_rgba(from[i], to[i], t))
}

// Solid<->Solid lerps as a single color, same as always. Anything involving a
// gradient (on either side - two different gradients, or a gradient and a
// solid) falls back to lerping the 4 corner colors independently instead (see
// Fill::corner_colors/Fill::Corners) - there's no single obviously "correct"
// in-between *gradient* for two arbitrary stop lists (different counts,
// different positions), but 4 corners always exist for any Fill, so that's
// what actually animates smoothly frame to frame. This does mean a >2-stop
// gradient's exact per-stop subdivision (see Fill::axis_split) is only exact
// once a transition has fully settled onto it, not mid-transition - a
// deliberate tradeoff, not a bug: the alternative (snapping the shape instantly
// while only the color lerped) is exactly what looked wrong for the sidebar
// hover accent earlier, just for a background fill instead of a border.
fn lerp_fill(from: &Fill, to: &Fill, t: f32) -> Fill {
    match (from, to) {
        (Fill::Solid(a), Fill::Solid(b)) => Fill::Solid(UiColor::from(lerp_rgba((*a).into(), (*b).into(), t))),
        _ => Fill::Corners(lerp_corners(from.corner_colors(), to.corner_colors(), t)),
    }
}

/// Every animatable `Style` field, resolved to a concrete value (no `Option`s left)
/// - what `.set_transition(...)` actually interpolates frame to frame, see
/// `ResolvedStyle::lerp`/`node_content_preparation`. `text_color` is kept as
/// normalized `[f32; 4]` (not `glyphon::Color`) purely so it can share `lerp_rgba`
/// with a `Fill::Solid`'s own lerp instead of needing its own u8-channel lerp.
#[derive(Clone)]
struct ResolvedStyle {
    background_color: Fill,
    border_color: Fill,
    border_width: f32,
    corner_radius: f32,
    text_color: [f32; 4],
    font_size: f32,
    alpha: f32,
    background_blur: f32,
}

impl ResolvedStyle {
    fn lerp(&self, target: &ResolvedStyle, t: f32) -> ResolvedStyle {
        ResolvedStyle {
            background_color: lerp_fill(&self.background_color, &target.background_color, t),
            border_color: lerp_fill(&self.border_color, &target.border_color, t),
            border_width: lerp(self.border_width, target.border_width, t),
            corner_radius: lerp(self.corner_radius, target.corner_radius, t),
            text_color: lerp_rgba(self.text_color, target.text_color, t),
            font_size: lerp(self.font_size, target.font_size, t),
            alpha: lerp(self.alpha, target.alpha, t),
            background_blur: lerp(self.background_blur, target.background_blur, t),
        }
    }

    fn text_color(&self) -> GlyphonColor {
        GlyphonColor::rgba(
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
    // Style overrides applied while the mouse is held down over this node - CSS's
    // `:active` (distinct from `:hover`, which alone doesn't imply the button is
    // being pressed). Layers on top of hover (see node_content_preparation) rather
    // than replacing it, so a press can override just e.g. border_color while
    // hover's background/text_color stay in effect underneath.
    pub press: Option<Style>,
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
    // Which edges a hover/press border is *currently* drawing along, held across
    // frames while it fades back out - `None` whenever there's no such fade in
    // progress (including a node that was never hovered at all - see below).
    // border_edges isn't part of ResolvedStyle (it's not a numeric value there's
    // any meaningful "halfway" for, see BorderEdges' own doc comment), but the
    // border's *color* still fades in/out smoothly via the ordinary transition
    // above (this codebase's transition model is an asymptotic ease that never
    // cleanly "finishes" - see set_transition's own doc comment), so snapping
    // edges straight to the resting value the instant a hover/press ends would
    // flash the wrong shape while that still-fading color is visible.
    //
    // Deliberately `Option`, not a bare `BorderEdges` defaulting to `ALL` - a
    // node whose *resting* style already has a visible border (e.g. a sidebar
    // tab styled as "selected" via a direct `.style.border_color` write, before
    // it's ever been hovered at all - see main_menu::ui::show_settings_tab) has
    // no hover-driven fade in progress on its very first frame, so there's
    // nothing to hold - reading the resting style directly is correct there,
    // and a bare-default field can't tell that case apart from "really is
    // fading out from BorderEdges::ALL", which showed up as the very first
    // selected sidebar tab flashing a full 4-side border before ever being
    // hovered. See node_content_preparation for how this is actually updated.
    border_edges_hold: Option<BorderEdges>,
    // Recomputed every frame in node_content_preparation from the current mouse
    // position - exposed read-only in case callers want to react to it too, not
    // wired up here beyond driving the hover style.
    pub is_hovered: bool,
    // Recomputed every frame alongside is_hovered - true for as long as the mouse
    // button stays down over this node (unlike is_clicked, which is only true the
    // one frame it goes down) - exposed read-only, drives the press style.
    pub is_pressed: bool,
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
            press: None,
            transition_ms: None,
            current_style: None,
            border_edges_hold: None,
            is_hovered: false,
            is_pressed: false,
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
                let base = self.style.text_color.unwrap_or(UiColor::WHITE);
                self.style.text_color = Some(base.with_alpha(alpha));
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
            self.is_pressed = false;
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

    /// `hit_testable` is false for everything behind a modal (see
    /// `Ui::always_on_top`/`App::prepare_ui_content`, the only caller that ever
    /// passes false) - a node processed with it false still renders and lays out
    /// completely normally, it just can't be hovered/pressed/clicked, so a modal
    /// visually sitting on top of the rest of the UI also actually blocks input
    /// to it, instead of every node hit-testing independently with no concept of
    /// what's occluding what.
    pub fn node_content_preparation(&mut self, size: &Size, ui: &mut UiRendering, font_system: &mut FontSystem, delta_time: f32, hit_testable: bool) -> (Vec<TextArea>, u16, u32) {
        // Inactive nodes render nothing and can't be hovered/clicked - their
        // *parent's* layout loop is what skips giving them any space (this alone
        // wouldn't stop them occupying a stacking slot), see the Container branch
        // below.
        if !self.is_active {
            self.is_hovered = false;
            self.is_pressed = false;
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
        // not any click-specific state of its own. Forced false when !hit_testable,
        // regardless of actual mouse position - see this fn's own doc comment.
        let mouse_over = hit_testable && point_in_rect(input::mouse_x() as f32, input::mouse_y() as f32, &self.transform.rect);
        self.is_hovered = self.hover.is_some() && mouse_over;
        self.is_pressed = self.press.is_some() && mouse_over && input::is_action_pressed("ui_click");

        // Hover's Some(...) fields win over the resting style, its None fields fall
        // back to it - built fresh every frame (not written back into self.style),
        // so un-hovering next frame just goes back to reading the untouched resting
        // style, no restore/undo bookkeeping needed. Press then layers the same way
        // on top of THAT (not the resting style directly) - a press implies the
        // mouse is also hovering, so whatever hover changed stays in effect unless
        // press explicitly overrides that same field too.
        let hover_applied = if self.is_hovered {
            self.hover.as_ref().unwrap().or(&self.style)
        } else {
            self.style.clone()
        };
        let effective_style = if self.is_pressed {
            self.press.as_ref().unwrap().or(&hover_applied)
        } else {
            hover_applied
        };
        self.is_clicked = mouse_over && input::is_action_just_pressed("ui_click");

        // The target this frame's animatable fields are heading toward (or already
        // at, with no .set_transition(...) set) - see ResolvedStyle/UiNode.current_style.
        let target = effective_style.resolve_concrete();
        let current = match (self.current_style.take(), self.transition_ms) {
            (Some(current), Some(ms)) if ms > 0.0 => {
                let t = (delta_time / (ms / 1000.0)).min(1.0);
                current.lerp(&target, t)
            }
            _ => target,
        };
        self.current_style = Some(current.clone());

        // border_edges isn't part of ResolvedStyle/ever lerped (there's no
        // meaningful halfway point between e.g. BorderEdges::LEFT and
        // BorderEdges::ALL) - while actively hovered/pressed, it just snaps
        // straight to whatever this frame's effective style says (and records
        // that in border_edges_hold, in case hover ends before the color has
        // caught up). The instant a hover/press ENDS, effective_style reverts to
        // the resting style immediately, while the border's own *color* is
        // still smoothly fading out over the transition above - snapping edges
        // too would flash the resting shape for a color that's still visibly
        // present, so this keeps reusing the held edges for as long as the
        // border still has meaningful alpha. But that hold only ever applies to
        // an actual hover/press fade-out (border_edges_hold is `None` otherwise,
        // including a node that's never been hovered at all) - a resting style
        // whose border is already visible on its own (e.g. a sidebar tab styled
        // as "selected" via a direct .style.border_color write - see
        // main_menu::ui::show_settings_tab) isn't fading out from anything, so
        // it just reads its own resting edges directly, same as if it had
        // already fully settled.
        let border_edges = if self.is_hovered || self.is_pressed {
            let edges = effective_style.border_edges.unwrap_or_default();
            self.border_edges_hold = Some(edges);
            edges
        } else if let Some(held) = self.border_edges_hold {
            if current.border_color.corner_colors()[0][3] > 0.01 {
                held
            } else {
                self.border_edges_hold = None;
                effective_style.border_edges.unwrap_or_default()
            }
        } else {
            effective_style.border_edges.unwrap_or_default()
        };

        // .clone() (not moved) - current.text_color()/font_size/alpha are still
        // read further down, in each UiNodeContent branch below, and a method call
        // like .text_color() needs the whole struct intact, not just its own field.
        let effective_visibility = Visibility::new(current.background_color.clone(), current.border_color.clone(), current.corner_radius, current.border_width, border_edges, current.background_blur);

        match &mut self.content {
            UiNodeContent::Text(label) => {
                // Background/border quad still fills the whole (already padding-grown,
                // see apply_padding) box - only the text itself renders inset within it.
                let (vertices_slice, indice_slice) = Self::compute_quad(&self.transform, &effective_visibility, size, ui.num_vertices);
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
                let (vertices_slice, indice_slice) = Self::compute_quad(&self.transform, &effective_visibility, size, ui.num_vertices);
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

                // Grow children (see SizeValue::Grow) split whatever main-axis space
                // is left after every other active child's own size and the gaps
                // between ALL active children are subtracted - same idea as CSS
                // flexbox's flex-grow: 1. Has to run before total_children_main
                // below, since that needs every active child's real main-axis size,
                // Grow children included.
                let is_grow = |c: &UiNode| match direction {
                    Orientation::Vertical => c.transform.grow.vertical,
                    Orientation::Horizontal => c.transform.grow.horizontal,
                };
                let grow_count = container.children.iter().filter(|(_, c)| c.is_active && is_grow(c)).count();
                if grow_count > 0 {
                    let fixed_main: f32 = container.children.iter()
                        .filter(|(_, c)| c.is_active && !is_grow(c))
                        .map(|(_, c)| match direction {
                            Orientation::Vertical => c.transform.height,
                            Orientation::Horizontal => c.transform.width,
                        })
                        .sum();
                    let content_main = match direction {
                        Orientation::Vertical => content_h,
                        Orientation::Horizontal => content_w,
                    };
                    let available = (content_main - fixed_main - gap * (active_count.saturating_sub(1) as f32)).max(0.0);
                    let grow_size = available / grow_count as f32;
                    for (_, child) in &mut container.children {
                        if child.is_active && is_grow(child) {
                            match direction {
                                Orientation::Vertical => child.transform.height = grow_size,
                                Orientation::Horizontal => child.transform.width = grow_size,
                            }
                            child.transform.apply_transformation();
                        }
                    }
                }

                // Cross-axis Grow ("stretch to fill the container's other axis" -
                // CSS flexbox's align-items: stretch equivalent) - unlike main-axis
                // Grow above, this used to only be resolved once, at build time
                // (see resolve_size's Grow case), against whatever the parent's size
                // happened to be then. If the parent is itself Grow/Fit-sized on ITS
                // own parent's main axis, its real size isn't final until that
                // ancestor's own per-frame pre-pass runs - so a cross-axis Grow
                // child resolved once against the parent's earlier, too-generous
                // estimate could end up wider/taller than the parent actually
                // settles to. Reapplying it here, every frame, against this
                // container's real current content_w/content_h keeps it correct
                // regardless of how ancestors resize.
                for (_, child) in &mut container.children {
                    if !child.is_active {
                        continue;
                    }
                    match direction {
                        Orientation::Vertical if child.transform.grow.horizontal => {
                            child.transform.width = content_w;
                            child.transform.apply_transformation();
                        }
                        Orientation::Horizontal if child.transform.grow.vertical => {
                            child.transform.height = content_h;
                            child.transform.apply_transformation();
                        }
                        _ => {}
                    }
                }

                // Calculate total children size for centering/end alignment on main axis
                let total_children_main: f32 = container.children.iter()
                    .filter(|(_, c)| c.is_active)
                    .map(|(_, c)| match direction {
                        Orientation::Vertical => c.transform.height,
                        Orientation::Horizontal => c.transform.width,
                    })
                    .sum::<f32>() + gap * (active_count.saturating_sub(1) as f32);

                // Cross-axis natural height for a Horizontal container's auto-height
                // Start case below - computed here (before the mutable child loop
                // borrows container.children) since that loop's borrow otherwise
                // outlives this function's return, see the E0502 this avoided.
                let horizontal_cross_natural_height: f32 = container.children.iter()
                    .filter(|(_, c)| c.is_active)
                    .map(|(_, c)| c.transform.height)
                    .fold(0.0f32, |a, b| a.max(b));

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

                    if let Some((x, y)) = child.pending_position {
                        // This child called its own .set_position(...) - CSS
                        // position:absolute, roughly: it opts out of the
                        // cursor/child_anchor flow-stacking below entirely and
                        // re-resolves its own Start/Center/End-plus-offset position
                        // (see UiTransform::resolve_position_in_rect) against this
                        // container's current content rect every frame instead - a
                        // Fit-sized self-positioned child's real width/height isn't
                        // known until its own layout pass runs below, so (like every
                        // other per-frame measurement in this function) this has to
                        // be recomputed fresh each frame, not resolved once.
                        // Doesn't advance the cursor or consume a gap - the
                        // auto_width/auto_height/total_children_main math above still
                        // counts this child's own size as if it were in flow, so
                        // mixing a self-positioned child into a Fit/Center/End-anchored
                        // container alongside flow siblings can still throw off their
                        // sizing/centering; out of scope for this to handle generally,
                        // fine for a self-positioned child that's the only one at its
                        // level (e.g. a single title/panel offset from a corner).
                        let content_rect = Rect { top: content_top, left: content_left, bottom: content_bottom, right: content_right };
                        child.transform.resolve_position_in_rect(x, y, &content_rect);
                    } else {
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
                    }

                    let (child_text_areas, _cv, _ci) = child.node_content_preparation(size, ui, font_system, delta_time, hit_testable);
                    text_areas.extend(child_text_areas);
                }

                // Auto-height for the Start case (grows down from a fixed top, using
                // where children actually ended up post-layout) - End/Center already
                // handled above, before children were positioned. end_extent tracks
                // the main-axis cursor, which only doubles as "total height" when the
                // main axis IS height (Vertical) - for Horizontal it's the stacked
                // *width*, so the cross-axis height still has to come from the
                // children's own heights directly (max, same as auto_width's
                // Horizontal->sum/Vertical->max split above, mirrored here).
                if auto_height && self.transform.self_anchor.vertical == Anchor::Start {
                    let target_bottom = match direction {
                        Orientation::Vertical => end_extent + padding.bottom,
                        Orientation::Horizontal => content_top + horizontal_cross_natural_height + padding.bottom,
                    };
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
        let outline = Visibility::new(Fill::Solid(UiColor::TRANSPARENT), Fill::Solid(UiColor::Rgb(255, 0, 0)), 0.0, 2.0, BorderEdges::ALL, 0.0);
        let (vertices_slice, indice_slice) = Self::compute_quad(&self.transform, &outline, size, ui.num_vertices);

        ui.num_vertices += vertices_slice.len() as u16;
        ui.num_indices += indice_slice.len() as u32;
        ui.vertices.extend_from_slice(&vertices_slice);
        ui.indices.extend_from_slice(&indice_slice);

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

    /// Builds a node's own background/border quad - a single independent
    /// 4-vertex/6-index quad (`TL,BL,BR,TR` order, same winding this always used)
    /// for a `Solid` or 2-stop `Fill::Gradient` (identical to how this rendered
    /// before multi-stop gradients existed), or, for an axis-aligned
    /// `Fill::Gradient` with more stops than 2 corners can represent exactly (see
    /// `Fill::axis_split`), one such quad *per band* (one band per pair of
    /// adjacent stops), each an exact 2-stop-exact blend on its own, side by side.
    /// Deliberately not sharing vertices between adjacent bands (that would need
    /// only 2 extra vertices per split instead of 4, but was tried first and
    /// produced a visible seam at the shared boundary that didn't trace back to
    /// any error in the vertex/winding math on paper - not fully root-caused) -
    /// every band here is instead built by the exact same `quad(...)` closure the
    /// plain single-quad case already uses, just called more than once, which
    /// keeps each one trivially correct by construction rather than relying on a
    /// hand-rolled shared-index scheme. `base` is the index of the first vertex
    /// this call will add, i.e. `ui.num_vertices` at the call site.
    fn compute_quad(transform: &UiTransform, visibility: &Visibility, screen_size: &Size, base: u16) -> (Vec<VertexUi>, Vec<u16>) {
        let ndc_x = |x: f32| (x / (screen_size.width as f32 / 2.0)) - 1.0;
        let ndc_y = |y: f32| 1.0 - (y / (screen_size.height as f32 / 2.0));

        let rect = [
            transform.rect.top,
            transform.rect.left,
            transform.rect.bottom,
            transform.rect.right,
        ];

        let border_edges = visibility.border_edges.to_bits();
        let quad = |left: f32, top: f32, right: f32, bottom: f32, bg: [[f32; 4]; 4], bd: [[f32; 4]; 4], base: u16| -> (Vec<VertexUi>, Vec<u16>) {
            let vertices = vec![
                VertexUi { position: vector![ndc_x(left), ndc_y(top), 0.0].into(), color: bg[0], rect, border_color: bd[0], corner_radius: visibility.corner_radius, border_width: visibility.border_width, background_blur: visibility.background_blur, border_edges },
                VertexUi { position: vector![ndc_x(left), ndc_y(bottom), 0.0].into(), color: bg[1], rect, border_color: bd[1], corner_radius: visibility.corner_radius, border_width: visibility.border_width, background_blur: visibility.background_blur, border_edges },
                VertexUi { position: vector![ndc_x(right), ndc_y(bottom), 0.0].into(), color: bg[2], rect, border_color: bd[2], corner_radius: visibility.corner_radius, border_width: visibility.border_width, background_blur: visibility.background_blur, border_edges },
                VertexUi { position: vector![ndc_x(right), ndc_y(top), 0.0].into(), color: bg[3], rect, border_color: bd[3], corner_radius: visibility.corner_radius, border_width: visibility.border_width, background_blur: visibility.background_blur, border_edges },
            ];
            let indices = vec![base, 1 + base, 2 + base, base, 2 + base, 3 + base];
            (vertices, indices)
        };

        // Whichever of background/border needs subdividing (see Fill::axis_split)
        // drives the split - in practice only one of the two is ever a >2-stop
        // gradient at a time, so the other (a Solid or plain 2-stop fill) is just
        // sampled at whatever positions the driving one needs (see Fill::sample) -
        // exact for that one too, just riding along on the same bands instead of
        // asking for its own.
        let Some((direction, positions)) = visibility.background_color.axis_split().or_else(|| visibility.border_color.axis_split()) else {
            // Fast path: the plain 4-vertex quad every non-multi-stop-gradient fill
            // (the overwhelming majority of nodes) already rendered as before this
            // feature existed - each corner still gets its own color (see
            // Fill::corner_colors) so a Fill::Gradient's endpoints still reach the
            // GPU, which interpolates the rest across the quad for free (no shader
            // changes needed - see Fill::Gradient's own doc comment); a Fill::Solid
            // just puts the same value in all 4.
            let bg = visibility.background_color.corner_colors();
            let bd = visibility.border_color.corner_colors();
            return quad(transform.rect.left, transform.rect.top, transform.rect.right, transform.rect.bottom, bg, bd, base);
        };

        // Where stop `t` actually falls in world/screen space along this box's
        // edge - increasing with `t` for ToRight/ToBottom, decreasing for
        // ToLeft/ToTop (see GradientDirection::is_reversed), since those run their
        // own t=0.0 from the box's right/bottom edge instead of its left/top one.
        let world_edge = |t: f32| -> f32 {
            let a = if direction.is_reversed() { 1.0 - t } else { t };
            if direction.is_horizontal() {
                transform.rect.left + (transform.rect.right - transform.rect.left) * a
            } else {
                transform.rect.top + (transform.rect.bottom - transform.rect.top) * a
            }
        };

        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let mut next_base = base;
        for pair in positions.windows(2) {
            let (t0, t1) = (pair[0], pair[1]);
            let (e0, e1) = (world_edge(t0), world_edge(t1));
            let (c0_bg, c1_bg) = (visibility.background_color.sample(t0), visibility.background_color.sample(t1));
            let (c0_bd, c1_bd) = (visibility.border_color.sample(t0), visibility.border_color.sample(t1));
            // world_edge can run either increasing or decreasing (see its own
            // comment) - min/max here is what keeps this band's own quad always
            // built low-edge-to-high-edge regardless of which direction the
            // gradient itself runs, matching whichever of c0/c1 actually belongs
            // to the lower edge.
            let (lo_edge, lo_bg, hi_edge, hi_bg, lo_bd, hi_bd) = if e0 <= e1 { (e0, c0_bg, e1, c1_bg, c0_bd, c1_bd) } else { (e1, c1_bg, e0, c0_bg, c1_bd, c0_bd) };

            let (bg, bd) = if direction.is_horizontal() {
                // TL=lo,BL=lo,BR=hi,TR=hi - same as Fill::corner_colors' ToRight shape.
                ([lo_bg, lo_bg, hi_bg, hi_bg], [lo_bd, lo_bd, hi_bd, hi_bd])
            } else {
                // TL=lo,BL=hi,BR=hi,TR=lo - same as Fill::corner_colors' ToBottom shape.
                ([lo_bg, hi_bg, hi_bg, lo_bg], [lo_bd, hi_bd, hi_bd, lo_bd])
            };

            let (band_vertices, band_indices) = if direction.is_horizontal() {
                quad(lo_edge, transform.rect.top, hi_edge, transform.rect.bottom, bg, bd, next_base)
            } else {
                quad(transform.rect.left, lo_edge, transform.rect.right, hi_edge, bg, bd, next_base)
            };
            next_base += band_vertices.len() as u16;
            vertices.extend(band_vertices);
            indices.extend(band_indices);
        }

        (vertices, indices)
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
                let align = match label_data.alignment.as_deref() {
                    Some("Center") => Align::Center,
                    Some("Right") => Align::Right,
                    _ => Align::Left,
                };
                style.background_color = Some(Fill::Solid(label_data.background_color.unwrap_or([0.0, 0.0, 0.0, 0.0]).into()));
                style.border_color = Some(Fill::Solid(label_data.border_color.unwrap_or([0.0, 0.0, 0.0, 0.0]).into()));
                style.text_color = Some(label_data.color.into());
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
                style.background_color = Some(Fill::Solid(container_data.background_color.unwrap_or([0.0, 0.0, 0.0, 0.0]).into()));
                style.border_color = Some(Fill::Solid(container_data.border_color.unwrap_or([0.0, 0.0, 0.0, 0.0]).into()));
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
            // SizeValue::Fit resolves to 0.0 on the assumption that a later pass
            // fills in the real value from children (see node_content_preparation's
            // auto_width/auto_height) - true for containers, but Text/Image nodes
            // have no such pass. Requesting Fit on one of those isn't meaningful
            // (there are no children to size to), but silently collapsing to a
            // 0-sized, invisible node is a footgun - keep whatever size the
            // constructor already measured/set instead.
            let is_container = matches!(self.content, UiNodeContent::Container(_));
            let width = if !is_container && matches!(width, SizeValue::Fit) { SizeValue::Pixels(self.transform.width) } else { width };
            let height = if !is_container && matches!(height, SizeValue::Fit) { SizeValue::Pixels(self.transform.height) } else { height };
            self.transform.resolve_size(width, height, parent_width, parent_height);
        }
        if let Some((x, y)) = self.pending_position {
            self.transform.resolve_position(x, y, parent_width, parent_height);
        } else {
            // resolve_position is what syncs `rect` (the actual field layout and
            // rendering read - see node_content_preparation/compute_vertices) from
            // x/y/width/height, via apply_transformation - resolve_size above
            // never touches rect itself. A node that never calls .set_position(...)
            // (e.g. a full-screen overlay only ever sized via Grow, relying on
            // x/y's 0,0 default) would otherwise keep rect stuck at its
            // all-zero construction-time default forever, even after resolve_size
            // just gave it a real width/height - every child positioned relative
            // to it then computes against that stale zero-sized rect instead.
            self.transform.apply_transformation();
        }
        if let UiNodeContent::Container(container) = &mut self.content {
            // Children resolve Percent/Grow against this container's *content* box
            // (its own size minus its own padding), not its outer box - the same
            // content_w/content_h every frame's layout pass already uses (see
            // node_content_preparation) for the exact same reason: a child sized
            // against the outer box would size itself out past this container's own
            // padding instead of stopping at it.
            let width = (self.transform.width - self.padding.left - self.padding.right).max(0.0);
            let height = (self.transform.height - self.padding.top - self.padding.bottom).max(0.0);
            for (_, child) in &mut container.children {
                child.resolve(width, height);
            }
        }
    }

    /// Sets the style overrides this node uses while the mouse is over it (see
    /// `Style`, checked fresh every frame in `node_content_preparation`) - works on
    /// any node type, same as `.set_background_color`/`.set_border_color`. Merges
    /// with (rather than replacing) any hover style already set by an earlier
    /// `.on_hover(...)` call on this same node - this call's own fields win where
    /// both set something, whatever it leaves unset falls back to the earlier
    /// call's - so a caller building on top of e.g. `button(...)`'s own hover
    /// effect (its background/text color change) can layer in just what it wants
    /// to add (e.g. a border) without needing to know/repeat the rest.
    pub fn on_hover(mut self, hover: Style) -> Self {
        self.hover = Some(match self.hover {
            Some(existing) => hover.or(&existing),
            None => hover,
        });
        self
    }

    /// Sets the style overrides this node uses while the mouse is held down over it
    /// - CSS's `:active`, checked fresh every frame in `node_content_preparation`
    /// alongside `.on_hover(...)`, and layered on top of it (a press implies the
    /// mouse is also hovering, so hover's effects stay live underneath - a press
    /// style only needs to state what's *different* about being pressed, e.g. just
    /// `border_color`).
    pub fn on_press(mut self, press: Style) -> Self {
        self.press = Some(press);
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
    pub fn set_text_color(mut self, color: UiColor) -> Self {
        self.style.text_color = Some(color);
        self
    }

    /// Works on any node type - inert on content types other than Text. Also
    /// grows this node's own box (both height and width) if it's currently too
    /// small to fit one line at `size` without clipping - a label's box is its
    /// own text clip rect (see `Label::min_height_for_font_size`/
    /// `natural_width_at`'s doc comments), and both the default auto-height and
    /// any explicit/auto width were sized for whatever font_size was in effect
    /// before this call (most commonly `DEFAULT_FONT_SIZE`), so a larger `size`
    /// would otherwise clip the text into invisibility instead of actually
    /// growing to fit it. Needs `font_system` to re-measure the real natural
    /// width at `size` (an approximation like scaling the old width by
    /// `size`/old font_size would drift from font hinting) - every call site
    /// already has one in scope (`app.ui.text.font_system`), same as `label(...)`
    /// itself needs.
    pub fn set_font_size(mut self, font_system: &mut FontSystem, size: f32) -> Self {
        self.style.font_size = Some(size);
        if let UiNodeContent::Text(label) = &self.content {
            let min_height = min_height_for_font_size(size);
            let min_width = label.natural_width_at(font_system, size);
            if min_height > self.transform.height {
                self.transform.height = min_height;
            }
            if min_width > self.transform.width {
                self.transform.width = min_width;
            }
            self.transform.apply_transformation();
        }
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

    /// Where this container's cursor-based stacking starts from on each axis -
    /// `Start`/`Center`/`End`, like CSS flexbox's `justify-content`/`align-items`
    /// (main/cross axis respectively, both controlled together here rather than
    /// as two separate properties). Was previously only reachable via the RON
    /// loading path (`UiTransform::with_anchors`) - exposing it here is what
    /// makes a single fixed-size child centered inside a full-screen `Grow`
    /// container (e.g. a modal overlay) possible without also needing per-node
    /// absolute positioning, which this layout model doesn't have.
    pub fn set_child_anchor(mut self, horizontal: Anchor, vertical: Anchor) -> Self {
        self.transform.child_anchor = ChildAnchor { horizontal, vertical };
        self
    }

    /// Works on any node type - `style` isn't container-specific. Takes a plain
    /// `UiColor` for a flat fill, or a `Fill::Gradient`/`Fill::linear(...)` for a
    /// two-stop gradient across this node's own box - see `Fill`'s own doc comment.
    pub fn set_background_color(mut self, color: impl Into<Fill>) -> Self {
        self.style.background_color = Some(color.into());
        self
    }

    /// Works on any node type - `style` isn't container-specific. Same `UiColor`-or-
    /// gradient `Fill` as `.set_background_color(...)`.
    pub fn set_border_color(mut self, color: impl Into<Fill>) -> Self {
        self.style.border_color = Some(color.into());
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

    /// Restricts this node's border to just the given sides - e.g.
    /// `BorderEdges::LEFT` for a sidebar-style accent bar, commonly paired with
    /// `.on_hover(...)` for a "highlight the left edge on hover" effect. Defaults
    /// to `BorderEdges::ALL` (every side) if never called, matching every border
    /// before this existed. Doesn't animate with `.set_transition(...)` - there's
    /// no meaningful halfway point between e.g. `LEFT` and `ALL`, so like `align`
    /// it always snaps instantly to whichever value is current this frame - see
    /// `BorderEdges`'s own doc comment for the rendering-side tradeoff this makes.
    pub fn set_border_edges(mut self, edges: BorderEdges) -> Self {
        self.style.border_edges = Some(edges);
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

    /// Blurs whatever's behind this node's fill (the 3D scene and/or other UI
    /// underneath it) - CSS's `backdrop-filter: blur()`, works on any node type,
    /// same as `.set_background_color`. `radius_px` is the blur radius in pixels,
    /// same units/scale as e.g. Tailwind's `backdrop-blur-*` utilities (`sm`≈4,
    /// `md`≈12, `lg`≈16, `xl`≈24, `2xl`≈40, `3xl`≈64, clamped at
    /// `BlurRender::MAX_BLUR_RADIUS`); `0.0` (the default) is off, behaving
    /// exactly like before this existed. Like Tailwind's own `backdrop-blur` +
    /// `bg-white/30` pattern, this is independent of `.set_background_color(...)`'s
    /// own alpha: the blur radius comes from here, while the *background color's
    /// alpha* controls how much of that blur shows through versus how strongly
    /// the flat color tints it (an opaque background_color fully hides the blur;
    /// a translucent one lets it show through, tinted) - see text_shader.wgsl's
    /// fill_color. Every node crossfades between the same shared, precomputed
    /// full-resolution blur based on its own radius (see BlurRender), rather than
    /// each sampling the live scene independently at an arbitrary radius - the
    /// latter needs either a lot of samples or shows visible ghosting at
    /// anything but a small radius, whereas this stays smooth at every strength.
    /// Animates with `.set_transition(...)` like every other style field.
    pub fn set_background_blur(mut self, radius_px: f32) -> Self {
        self.style.background_blur = Some(radius_px);
        self
    }
}
