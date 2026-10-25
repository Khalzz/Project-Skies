//! A short message in the bottom-left corner of the screen that shows
//! itself and goes away on its own - "Saved", "Controller connected"...
//!
//!     show_toast(app, "Game saved", &ToastStyle::default());
//!
//! It grows out from its left edge, stays `duration_ms`, then shrinks back
//! and is removed. A new one always appears at the bottom, pushing the
//! older ones up; when one goes, the ones above it slide down into its
//! place. More than `max_visible`, or more than fit on the screen, and the
//! oldest leave early. They draw over the rest of the UI (modals included)
//! without blocking clicks to it.
//!
//! Spacing and the limit are `app.ui.toasts` (`margin`, `gap`,
//! `max_visible`).

use crate::app::App;
use crate::rendering::ui::ui::Ui;
use crate::ui::color::{Fill, UiColor};
use crate::ui::components::label::{min_height_for_font_size, text_width};
use crate::ui::text_style::TextStyle;
use crate::ui::ui_node::UiNode;
use crate::ui::ui_transform::{Anchor, Orientation, Padding, SizeValue};
use crate::rendering::shader_library::ShaderLibrary;
use crate::scene_manager::node::properties::Material;

/// How fast a toast slides to its place in the stack - the higher, the
/// quicker (an exponential ease, per second).
const SLIDE_SPEED: f32 = 14.0;

/// The UI shaders drawing the icons - see `add_icon_shaders`.
const ALERT_ICON: &str = "engine/icon_alert";
const ERROR_ICON: &str = "engine/icon_error";

/// Registers the icons' shaders - once, when the renderer starts.
pub(crate) fn add_icon_shaders(shaders: &mut ShaderLibrary) {
    shaders.add(ALERT_ICON, include_str!("../../shaders/ui_icon_alert.wgsl"), Some("(kind: Ui)"));
    shaders.add(ERROR_ICON, include_str!("../../shaders/ui_icon_error.wgsl"), Some("(kind: Ui)"));
}

/// What a toast is about - an alert or an error shows an icon before its
/// text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ToastKind {
    /// Just the text.
    #[default]
    Info,
    /// A yellow warning triangle.
    Alert,
    /// A red circle with an x.
    Error,
}

impl ToastKind {
    fn icon_shader(self) -> Option<&'static str> {
        match self {
            ToastKind::Info => None,
            ToastKind::Alert => Some(ALERT_ICON),
            ToastKind::Error => Some(ERROR_ICON),
        }
    }
}

/// How a toast looks and how long it stays.
#[derive(Clone)]
pub struct ToastStyle {
    /// Info, alert or error - see `ToastKind`.
    pub kind: ToastKind,
    /// The icon's width and height, in pixels.
    pub icon_size: f32,
    /// Space between the icon and the text.
    pub icon_gap: f32,
    pub text: TextStyle,
    /// Space between the text and the toast's edges.
    pub padding: Padding,
    /// A fixed width (padding included) - None: as wide as its text.
    pub width: Option<f32>,
    /// Usually translucent, so the blur shows through.
    pub background: Fill,
    /// Blurs what's behind it, in pixels. 0: off.
    pub background_blur: f32,
    pub border_color: Fill,
    /// In pixels - 0 for no border.
    pub border_width: f32,
    pub corner_radius: f32,
    /// How long (ms) it takes to grow in - and to shrink out.
    pub grow_ms: f32,
    /// How long (ms) it stays fully shown.
    pub duration_ms: f32,
}

impl Default for ToastStyle {
    fn default() -> Self {
        Self {
            kind: ToastKind::Info,
            icon_size: 14.0,
            icon_gap: 7.0,
            text: TextStyle::default().size(13.0).color(UiColor::WHITE),
            padding: Padding::from((10.0, 5.0)),
            width: None,
            background: UiColor::Rgba(18, 18, 22, 140).into(),
            background_blur: 16.0,
            border_color: UiColor::Rgba(255, 255, 255, 70).into(),
            border_width: 1.0,
            corner_radius: 6.0,
            grow_ms: 250.0,
            duration_ms: 3000.0,
        }
    }
}

impl ToastStyle {
    /// The default look, as an info toast - just the text.
    pub fn info() -> Self {
        Self::default()
    }

    /// The default look, with a yellow warning triangle.
    pub fn alert() -> Self {
        Self::default().kind(ToastKind::Alert)
    }

    /// The default look, with a red x.
    pub fn error() -> Self {
        Self::default().kind(ToastKind::Error)
    }

    pub fn kind(mut self, kind: ToastKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn icon_size(mut self, size: f32) -> Self {
        self.icon_size = size;
        self
    }

    pub fn text(mut self, build: impl FnOnce(TextStyle) -> TextStyle) -> Self {
        self.text = build(self.text);
        self
    }

    /// `10.0` (every side), `(x, y)`, or `[top, bottom, left, right]`.
    pub fn padding(mut self, padding: impl Into<Padding>) -> Self {
        self.padding = padding.into();
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    pub fn background(mut self, background: impl Into<Fill>) -> Self {
        self.background = background.into();
        self
    }

    pub fn background_blur(mut self, radius_px: f32) -> Self {
        self.background_blur = radius_px;
        self
    }

    pub fn border(mut self, color: impl Into<Fill>, width: f32) -> Self {
        self.border_color = color.into();
        self.border_width = width;
        self
    }

    pub fn corner_radius(mut self, radius: f32) -> Self {
        self.corner_radius = radius;
        self
    }

    pub fn grow(mut self, ms: f32) -> Self {
        self.grow_ms = ms;
        self
    }

    pub fn duration(mut self, ms: f32) -> Self {
        self.duration_ms = ms;
        self
    }
}

/// The toasts showing now, and how they're spaced - `app.ui.toasts`.
pub struct Toasts {
    /// Space between the stack and the screen's left and bottom edges.
    pub margin: f32,
    /// Space between one toast and the next.
    pub gap: f32,
    /// The most showing at once - past it, the oldest leave early.
    pub max_visible: usize,
    /// Oldest (top) first.
    active: Vec<ActiveToast>,
    next_id: u64,
}

struct ActiveToast {
    /// Its top-level UI key (in Ui::overlay too).
    key: String,
    full_width: f32,
    height: f32,
    /// Where its top is now - eased toward its place every frame. None
    /// until its first frame, when it starts right at its place.
    y: Option<f32>,
    grow_ms: f32,
    duration_ms: f32,
    age_ms: f32,
}

impl Default for Toasts {
    fn default() -> Self {
        Self { margin: 20.0, gap: 6.0, max_visible: 5, active: Vec::new(), next_id: 0 }
    }
}

impl ActiveToast {
    fn shrink_start(&self) -> f32 {
        self.grow_ms + self.duration_ms
    }

    fn is_leaving(&self) -> bool {
        self.age_ms >= self.shrink_start()
    }

    /// How much of its width shows now, 0 to 1.
    fn shown(&self) -> f32 {
        if self.age_ms < self.grow_ms {
            ease_out(self.age_ms / self.grow_ms)
        } else if self.age_ms < self.shrink_start() {
            1.0
        } else {
            1.0 - ease_in((self.age_ms - self.shrink_start()) / self.grow_ms)
        }
    }

    /// Starts shrinking now, from however wide it is - still growing in,
    /// it shrinks from there instead of jumping to full width first.
    fn leave(&mut self) {
        if self.is_leaving() {
            return;
        }
        let shown = self.shown();
        // ease_in(x) = x^3 - the point of the shrink that's this wide.
        self.age_ms = self.shrink_start() + self.grow_ms * (1.0 - shown).cbrt();
    }
}

/// Shows `message` as a toast looking like `style`.
pub fn show_toast(app: &mut App, message: &str, style: &ToastStyle) {
    // Sized to the text at its real size - a plain label is measured at the
    // default size first and only ever grows, so smaller text would leave a
    // box too big for it.
    let font_system = &mut app.ui.text.font_system;
    let text_width = text_width(font_system, message, style.text.size, style.text.font);
    let text_height = min_height_for_font_size(style.text.size);
    let label = UiNode::label(font_system, message, Some(text_width), Some(text_height))
        .set_text_style(font_system, &style.text);
    // An alert's or error's icon, before the text - a square drawn by its shader.
    let icon = style.kind.icon_shader().map(|shader| {
        UiNode::container()
            .set_size(SizeValue::Pixels(style.icon_size), SizeValue::Pixels(style.icon_size))
            .set_material(Material::shader(shader))
    });
    let icon_room = if icon.is_some() { style.icon_size + style.icon_gap } else { 0.0 };
    let full_width = style.width.unwrap_or(icon_room + label.transform.width + style.padding.left + style.padding.right);
    let content_height = if icon.is_some() { label.transform.height.max(style.icon_size) } else { label.transform.height };
    let height = content_height + style.padding.top + style.padding.bottom;

    // Starts 1px wide - `update_toasts` grows it, its contents clipped to its
    // box while it's narrower than them.
    let mut toast = UiNode::container()
        .set_size(SizeValue::Pixels(1.0), SizeValue::Fit)
        .set_orientation(Orientation::Horizontal)
        .set_child_anchor(Anchor::Start, Anchor::Center)
        .set_gap(style.icon_gap)
        .set_padding(style.padding)
        .set_background_color(style.background.clone())
        .set_background_blur(style.background_blur)
        .set_border_color(style.border_color.clone())
        .set_border_width(style.border_width)
        .set_corner_radius(style.corner_radius)
        .set_clip_children(true);
    if let Some(icon) = icon {
        toast = toast.set_child("icon", icon);
    }
    let mut toast = toast.set_child("text", label);
    let (screen_width, screen_height) = app.ui.size();
    toast.resolve(screen_width, screen_height);

    let toasts = &mut app.ui.toasts;
    let key = format!("__toast{}", toasts.next_id);
    toasts.next_id += 1;
    toasts.active.push(ActiveToast {
        key: key.clone(),
        full_width,
        height,
        y: None,
        grow_ms: style.grow_ms.max(1.0),
        duration_ms: style.duration_ms,
        age_ms: 0.0,
    });
    app.ui.add_to_ui(key.clone(), toast);
    app.ui.overlay.push(key);
    app.ui.has_changed = true;
}

/// Grows, holds, shrinks, places and removes the toasts - every frame, by
/// `delta_seconds` of real time, on a screen `screen_height` tall (see
/// App::render).
pub(crate) fn update_toasts(ui: &mut Ui, delta_seconds: f32, screen_height: f32) {
    if ui.toasts.active.is_empty() {
        return;
    }

    // Done ones go first, so the ones above them get their new places
    // (and slide down to them) this same frame.
    let delta_ms = delta_seconds * 1000.0;
    let mut finished = Vec::new();
    ui.toasts.active.retain_mut(|toast| {
        toast.age_ms += delta_ms;
        let done = toast.age_ms >= toast.shrink_start() + toast.grow_ms;
        if done {
            finished.push(std::mem::take(&mut toast.key));
        }
        !done
    });
    for key in finished {
        ui.renderizable_elements.remove(&key);
        ui.overlay.retain(|overlay_key| *overlay_key != key);
    }

    let Toasts { margin, gap, max_visible, active, .. } = &mut ui.toasts;

    // Too many: the oldest still staying start leaving.
    let staying = active.iter().filter(|toast| !toast.is_leaving()).count();
    for toast in active.iter_mut().filter(|toast| !toast.is_leaving()).take(staying.saturating_sub(*max_visible)) {
        toast.leave();
    }

    // Stacked upward from the bottom-left corner, newest lowest. One whose
    // place is past the top of the screen starts leaving too.
    let slide = 1.0 - (-SLIDE_SPEED * delta_seconds).exp();
    let mut bottom = screen_height - *margin;
    for toast in active.iter_mut().rev() {
        let place = bottom - toast.height;
        bottom = place - *gap;
        if place < *margin {
            toast.leave();
        }
        let y = match toast.y {
            Some(y) => y + (place - y) * slide,
            None => place,
        };
        toast.y = Some(y);
    }

    active.retain(|toast| {
        // Gone with the rest of the UI (a scene change).
        let Some(node) = ui.renderizable_elements.get_mut(&toast.key) else { return false };
        // Never 0 - a container 0 wide sizes itself to its children.
        node.transform.width = (toast.full_width * toast.shown()).max(1.0);
        node.move_to(*margin, toast.y.unwrap_or_default());
        true
    });
    ui.has_changed = true;
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

fn ease_in(t: f32) -> f32 {
    t.clamp(0.0, 1.0).powi(3)
}
