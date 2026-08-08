use serde::Deserialize;

/// Horizontal or vertical alignment axis
#[derive(Clone, Debug, Deserialize, PartialEq, Default)]
pub enum Anchor {
    #[default]
    Start,
    Center,
    End,
}

/// How this element positions itself within its parent.
/// The anchor resolves a base position; `x` and `y` from the transform are then added as offset.
#[derive(Clone, Debug, Deserialize, Default)]
pub struct SelfAnchor {
    #[serde(default)]
    pub horizontal: Anchor,
    #[serde(default)]
    pub vertical: Anchor,
}

/// How children are anchored/laid out inside this element (only relevant for containers).
#[derive(Clone, Debug, Deserialize, Default)]
pub struct ChildAnchor {
    #[serde(default)]
    pub horizontal: Anchor,
    #[serde(default)]
    pub vertical: Anchor,
}

/// Orientation children are stacked in a container - like a flex container's
/// `flex-direction`.
#[derive(Clone, Debug, Deserialize, Default, PartialEq)]
pub enum Orientation {
    #[default]
    Vertical,
    Horizontal,
}

/// Whether children should stretch to fill the parent on a given axis.
#[derive(Clone, Debug, Deserialize, Default)]
pub struct Fit {
    #[serde(default)]
    pub horizontal: bool,
    #[serde(default)]
    pub vertical: bool,
}

/// Inset between a node's own box edges and its content, one value per side - the
/// CSS `padding` shorthand's full generality. For a container, insets its children
/// from its own edges; for a Text/Image node (no children to inset), grows the
/// node's own box instead and insets the rendered content within it - see
/// `UiNode::apply_padding`. Built via `UiNode::set_padding(impl Into<Padding>)`,
/// which accepts any of the three `From` impls below - one method, three shapes,
/// matching CSS's own `padding` shorthand:
/// - `.set_padding(10.0)` - every side.
/// - `.set_padding((10.0, 2.0))` - `(x, y)`, i.e. left+right, top+bottom.
/// - `.set_padding([10.0, 2.0, 4.0, 4.0])` - `[top, bottom, left, right]`.
#[derive(Clone, Copy, Default, Debug)]
pub struct Padding {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl Padding {
    pub fn all(value: f32) -> Self {
        Self { top: value, right: value, bottom: value, left: value }
    }
}

impl From<f32> for Padding {
    fn from(value: f32) -> Self {
        Padding::all(value)
    }
}

/// `(x, y)` - x is left+right, y is top+bottom.
impl From<(f32, f32)> for Padding {
    fn from((x, y): (f32, f32)) -> Self {
        Padding { top: y, bottom: y, left: x, right: x }
    }
}

/// `[top, bottom, left, right]`.
impl From<[f32; 4]> for Padding {
    fn from([top, bottom, left, right]: [f32; 4]) -> Self {
        Padding { top, bottom, left, right }
    }
}

/// A richer per-axis size than a raw pixel value - resolved immediately (via
/// `UiTransform::resolve_size`) against the parent's known size, same as everything
/// else in this UI system (nothing re-resolves later, e.g. on window resize).
/// `Fit` just sets the same `Fit` flags above - the actual size-to-children
/// computation already happens in `node_content_preparation`'s per-frame layout,
/// this doesn't duplicate it. Not `Deserialize` yet - code-first only for now.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SizeValue {
    Pixels(f32),
    /// Percent of the parent's size on this axis (0.0..=100.0).
    Percent(f32),
    /// Size to children's natural size - containers only, same as `Fit` above.
    Fit,
}

/// A richer per-axis position than a raw pixel offset - resolved immediately (via
/// `UiTransform::resolve_position`) against the parent's known size and this node's
/// own already-resolved size (so resolve size first if setting both). `Start`/
/// `Center`/`End` anchor to that edge/the middle and then add their own pixel
/// offset - e.g. `End(-10.0)` sits 10px in from the far edge, `Start(0.0)` sits
/// flush against the near edge. This is the same math `SelfAnchor`/`Anchor` already
/// do for RON-loaded top-level nodes, just as one self-contained value per axis
/// instead of an anchor plus a separately-set raw offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PositionValue {
    /// Offset from the parent's start edge, as a percent of the parent's size (0.0..=100.0).
    Percent(f32),
    /// Anchored to the start (left/top) edge, plus this many pixels - `Start(0.0)` is
    /// flush against the edge; this also covers plain "N pixels from the top-left"
    /// positioning, since that's mathematically the same thing.
    Start(f32),
    /// Anchored to the center, plus this many pixels.
    Center(f32),
    /// Anchored to the end (right/bottom) edge, plus this many pixels - typically
    /// negative, to sit inward from the edge instead of past it.
    End(f32),
}

#[derive(Clone, Debug)]
pub struct Rect {
    pub top: f32,
    pub left: f32,
    pub bottom: f32,
    pub right: f32,
}

#[derive(Clone, Debug)]
pub struct UiTransform {
    pub rect: Rect,
    pub x: f32,
    pub y: f32,
    pub height: f32,
    pub width: f32,
    pub rotation: f32,
    pub smooth_change: bool,
    pub self_anchor: SelfAnchor,
    pub child_anchor: ChildAnchor,
    pub direction: Orientation,
    pub fit: Fit,
}

impl UiTransform {
    pub fn new(x: f32, y: f32, height: f32, width: f32, rotation: f32, smooth_change: bool) -> Self {
        let rect = Rect {
            top: y,
            left: x,
            bottom: y + height,
            right: x + width,
        };

        Self {
            rect,
            x,
            y,
            height,
            width,
            rotation,
            smooth_change,
            self_anchor: SelfAnchor::default(),
            child_anchor: ChildAnchor::default(),
            direction: Orientation::default(),
            fit: Fit::default(),
        }
    }

    pub fn with_anchors(mut self, self_anchor: SelfAnchor, child_anchor: ChildAnchor, direction: Orientation, fit: Fit) -> Self {
        self.self_anchor = self_anchor;
        self.child_anchor = child_anchor;
        self.direction = direction;
        self.fit = fit;
        self
    }

    /// Resolve this element's position given its parent rect.
    /// The anchor determines the base position within the parent, then x/y are added as offset.
    pub fn resolve_in_parent(&mut self, parent_rect: &Rect) {
        let parent_w = parent_rect.right - parent_rect.left;
        let parent_h = parent_rect.bottom - parent_rect.top;

        let base_x = match self.self_anchor.horizontal {
            Anchor::Start => parent_rect.left,
            Anchor::Center => parent_rect.left + (parent_w - self.width) / 2.0,
            Anchor::End => parent_rect.right - self.width,
        };

        let base_y = match self.self_anchor.vertical {
            Anchor::Start => parent_rect.top,
            Anchor::Center => parent_rect.top + (parent_h - self.height) / 2.0,
            Anchor::End => parent_rect.bottom - self.height,
        };

        let final_x = base_x + self.x;
        let final_y = base_y + self.y;

        self.rect = Rect {
            top: final_y,
            left: final_x,
            bottom: final_y + self.height,
            right: final_x + self.width,
        };
    }

    /// Resolve position as a screen-level element (parent is the full screen).
    pub fn resolve_on_screen(&mut self, screen_width: f32, screen_height: f32) {
        let screen_rect = Rect {
            top: 0.0,
            left: 0.0,
            bottom: screen_height,
            right: screen_width,
        };
        self.resolve_in_parent(&screen_rect);
    }

    pub fn apply_transformation(&mut self) {
        self.rect = Rect {
            top: self.y,
            left: self.x,
            bottom: self.y + self.height,
            right: self.x + self.width,
        };
    }

    /// Resolves a `SizeValue` pair against `parent_width`/`parent_height`, writing
    /// `self.width`/`self.height` and this node's own `fit` flags directly - `Fit`
    /// leaves size at `0.0` since the real fit-to-children value only exists once
    /// the per-frame layout pass runs (see `node_content_preparation`), same as
    /// today's RON-loaded `Fit`-flagged containers.
    pub fn resolve_size(&mut self, width: SizeValue, height: SizeValue, parent_width: f32, parent_height: f32) {
        self.fit.horizontal = matches!(width, SizeValue::Fit);
        self.fit.vertical = matches!(height, SizeValue::Fit);

        self.width = match width {
            SizeValue::Pixels(px) => px,
            SizeValue::Percent(pct) => parent_width * pct / 100.0,
            SizeValue::Fit => 0.0,
        };
        self.height = match height {
            SizeValue::Pixels(px) => px,
            SizeValue::Percent(pct) => parent_height * pct / 100.0,
            SizeValue::Fit => 0.0,
        };
    }

    /// Resolves a `PositionValue` pair against `parent_width`/`parent_height` -
    /// `Center`/`End` measure using `self.width`/`self.height`, so call
    /// `resolve_size` first if you're setting both (if that size is `SizeValue::Fit`,
    /// both are still `0.0` at this point - see `self_anchor` below). Refreshes
    /// `rect` afterward, same as `apply_transformation`.
    ///
    /// Also records which anchor was used into `self.self_anchor` (Percent counts as
    /// Start) - a `Fit`-sized container's real size isn't known until the per-frame
    /// layout pass runs (`node_content_preparation`), so an End/Center-anchored axis
    /// resolves here against a `0.0` placeholder. That pass reads `self_anchor` back
    /// to grow from the correct edge once the real size is known, instead of always
    /// growing right/down from a corner that was only ever correct for `Start`.
    pub fn resolve_position(&mut self, x: PositionValue, y: PositionValue, parent_width: f32, parent_height: f32) {
        self.self_anchor.horizontal = match x {
            PositionValue::End(_) => Anchor::End,
            PositionValue::Center(_) => Anchor::Center,
            PositionValue::Percent(_) | PositionValue::Start(_) => Anchor::Start,
        };
        self.self_anchor.vertical = match y {
            PositionValue::End(_) => Anchor::End,
            PositionValue::Center(_) => Anchor::Center,
            PositionValue::Percent(_) | PositionValue::Start(_) => Anchor::Start,
        };

        self.x = match x {
            PositionValue::Percent(pct) => parent_width * pct / 100.0,
            PositionValue::Start(offset) => offset,
            PositionValue::Center(offset) => (parent_width - self.width) / 2.0 + offset,
            PositionValue::End(offset) => parent_width - self.width + offset,
        };
        self.y = match y {
            PositionValue::Percent(pct) => parent_height * pct / 100.0,
            PositionValue::Start(offset) => offset,
            PositionValue::Center(offset) => (parent_height - self.height) / 2.0 + offset,
            PositionValue::End(offset) => parent_height - self.height + offset,
        };

        self.apply_transformation();
    }
}