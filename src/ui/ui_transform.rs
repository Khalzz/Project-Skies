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

/// Direction children are stacked in a container.
#[derive(Clone, Debug, Deserialize, Default, PartialEq)]
pub enum Direction {
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
    pub direction: Direction,
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
            direction: Direction::default(),
            fit: Fit::default(),
        }
    }

    pub fn with_anchors(mut self, self_anchor: SelfAnchor, child_anchor: ChildAnchor, direction: Direction, fit: Fit) -> Self {
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
}