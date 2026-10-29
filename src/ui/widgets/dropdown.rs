//! A box showing one choice of several, opening a list of them when
//! clicked.
//!
//!     dropdown(app, &["Low", "Medium", "High"], 1, &DropdownStyle::default())
//!         .on_select(|app, index| set_quality(app, index))
//!
//! The list opens under it (or over it, near the bottom of the screen),
//! drawn over the rest of the UI - modals included - and scrolls when it has
//! more than `max_visible` options. While it's open nothing under it can be
//! hovered or clicked; picking an option, or clicking anywhere else (the box
//! itself included), closes it. From code: `node.dropdown_selected()` /
//! `node.set_dropdown_selected(index)`.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};

use glyphon::FontSystem;

use crate::app::App;
use crate::rendering::shader_library::ShaderLibrary;
use crate::scene_manager::node::properties::Material;
use crate::ui::color::{Fill, UiColor};
use crate::ui::text_style::TextStyle;
use crate::ui::ui_node::{UiNode, UiNodeContent};
use crate::ui::ui_transform::{Anchor, Orientation, Padding, PositionValue, Rect, SizeValue};
use crate::ui::widgets::button::{button, ButtonStyle};
use crate::ui::{Align, CursorIcon};

/// The UI shader drawing the chevron - see `add_icon_shaders`.
const CHEVRON: &str = "engine/icon_chevron";
/// Space between the box and its list.
const LIST_GAP: f32 = 4.0;

/// Gives every dropdown its own key for its list.
static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

/// Registers the chevron's shader - once, when the renderer starts.
pub(crate) fn add_icon_shaders(shaders: &mut ShaderLibrary) {
    shaders.add(CHEVRON, include_str!("../../shaders/ui/ui_icon_chevron.wgsl"), Some("(kind: Ui)"));
}

/// How a `dropdown` and its list look.
#[derive(Clone)]
pub struct DropdownStyle {
    /// The box's width, padding included - its list is as wide.
    pub width: f32,
    pub text: TextStyle,
    pub padding: Padding,
    pub background: Fill,
    /// The box's background while the mouse is over it.
    pub hover_background: Fill,
    pub border_color: Fill,
    /// In pixels - 0 for no border.
    pub border_width: f32,
    pub corner_radius: f32,
    pub chevron_size: f32,
    pub list_background: Fill,
    /// An option's background while the mouse is over it.
    pub option_hover: Fill,
    /// The selected option's background in the list.
    pub option_selected: Fill,
    /// The most options shown at once - past it, the list scrolls.
    pub max_visible: usize,
}

impl Default for DropdownStyle {
    fn default() -> Self {
        Self {
            width: 200.0,
            text: TextStyle::default().size(14.0),
            padding: Padding::from((10.0, 3.0)),
            background: UiColor::Rgba(0, 0, 0, 90).into(),
            hover_background: UiColor::Rgba(255, 255, 255, 25).into(),
            border_color: UiColor::Rgba(255, 255, 255, 60).into(),
            border_width: 1.0,
            corner_radius: 4.0,
            chevron_size: 10.0,
            list_background: UiColor::Rgba(18, 18, 22, 245).into(),
            option_hover: UiColor::Rgba(255, 255, 255, 30).into(),
            option_selected: UiColor::Rgba(255, 255, 255, 14).into(),
            max_visible: 8,
        }
    }
}

impl DropdownStyle {
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
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

    pub fn background(mut self, background: impl Into<Fill>) -> Self {
        self.background = background.into();
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

    pub fn list_background(mut self, background: impl Into<Fill>) -> Self {
        self.list_background = background.into();
        self
    }

    pub fn max_visible(mut self, max_visible: usize) -> Self {
        self.max_visible = max_visible.max(1);
        self
    }
}

/// What runs when an option is picked - see `UiNode::on_select`.
type OnSelect = Rc<RefCell<Option<Box<dyn FnMut(&mut App, usize)>>>>;

/// A dropdown of `options`, `selected` chosen, looking like `style`.
pub fn dropdown(app: &mut App, options: &[&str], selected: usize, style: &DropdownStyle) -> UiNode {
    let options: Rc<Vec<String>> = Rc::new(options.iter().map(|option| option.to_string()).collect());
    let state = DropdownState {
        options: Rc::clone(&options),
        selected: Rc::new(Cell::new(selected.min(options.len().saturating_sub(1)))),
        rect: Rc::new(Cell::new([0.0; 4])),
        on_select: Rc::new(RefCell::new(None)),
    };
    let list_key = format!("__dropdown{}", NEXT_ID.fetch_add(1, Ordering::Relaxed));

    let font_system = &mut app.ui.text.font_system;
    let shown = options.get(state.selected.get()).map_or("", String::as_str);
    let faint = UiColor::Rgba(255, 255, 255, 60);
    let label = UiNode::label(font_system, shown, None, None)
        .set_text_style(font_system, &style.text.align(Align::Left))
        .set_size(SizeValue::Grow, SizeValue::Fit)
        .on_disabled(|disabled| disabled.set_text_color(faint));
    // Drawn by its shader in its own background color.
    let chevron = UiNode::container()
        .set_size(SizeValue::Pixels(style.chevron_size), SizeValue::Pixels(style.chevron_size))
        .set_background_color(UiColor::Rgb(230, 230, 230))
        .set_material(Material::shader(CHEVRON))
        .on_disabled(|disabled| disabled.set_background_color(faint));

    let open = {
        let (options, selected, rect, on_select, style) = (Rc::clone(&options), Rc::clone(&state.selected), Rc::clone(&state.rect), Rc::clone(&state.on_select), style.clone());
        move |app: &mut App| open_list(app, &list_key, &options, &selected, rect.get(), &on_select, &style)
    };

    let hover = style.hover_background.clone();
    UiNode::container()
        .set_orientation(Orientation::Horizontal)
        .set_child_anchor(Anchor::Start, Anchor::Center)
        .set_gap(8.0)
        .set_size(SizeValue::Pixels(style.width), SizeValue::Fit)
        .set_padding(style.padding)
        .set_background_color(style.background.clone())
        .set_border_color(style.border_color.clone())
        .set_border_width(style.border_width)
        .set_corner_radius(style.corner_radius)
        .set_transition(80.0)
        .on_hover(|hover_style| hover_style.set_background_color(hover))
        .on_disabled(|disabled| disabled.set_border_color(faint))
        .set_cursor(CursorIcon::Pointer)
        .set_child("value", label)
        .set_child("chevron", chevron)
        .on_click(open)
        .with_dropdown(state)
}

/// Opens the list of `options` under (or over) the box at `rect` - a
/// screen-wide, invisible node holding it, so a click anywhere else lands on
/// that and closes it.
fn open_list(app: &mut App, key: &str, options: &Rc<Vec<String>>, selected: &Rc<Cell<usize>>, rect: [f32; 4], on_select: &OnSelect, style: &DropdownStyle) {
    let [left, top, right, bottom] = rect;
    let width = (right - left).max(1.0);

    let row_style = ButtonStyle::default()
        .text(|_| style.text.align(Align::Left))
        .padding((8.0, 3.0))
        .background(UiColor::TRANSPARENT)
        .border(UiColor::TRANSPARENT, 0.0)
        .corner_radius(3.0)
        .on_hover(|_| crate::ui::ui_node::Style::default().set_background_color(style.option_hover.clone()))
        .on_press(|_| crate::ui::ui_node::Style::default().set_background_color(style.option_hover.clone()));
    let selected_style = row_style.clone().background(style.option_selected.clone());

    const LIST_PADDING: f32 = 4.0;
    const ROW_GAP: f32 = 2.0;
    let mut list = UiNode::container()
        .set_padding(LIST_PADDING)
        .set_gap(ROW_GAP)
        .set_background_color(style.list_background.clone())
        .set_border_color(style.border_color.clone())
        .set_border_width(style.border_width)
        .set_corner_radius(style.corner_radius);
    let mut row_height = 0.0f32;
    for (index, option) in options.iter().enumerate() {
        let (key, selected, on_select) = (key.to_owned(), Rc::clone(selected), Rc::clone(on_select));
        let row = button(app, option, if index == selected.get() { &selected_style } else { &row_style })
            .set_size(SizeValue::Grow, SizeValue::Fit)
            .on_click(move |app| {
                selected.set(index);
                close_list(app, &key);
                if let Some(f) = on_select.borrow_mut().as_mut() {
                    f(app, index);
                }
            });
        row_height = row_height.max(row.transform.height);
        list = list.set_child(format!("option{index}"), row);
    }

    // As tall as its options, up to `max_visible` of them - then it scrolls.
    let shown = options.len().min(style.max_visible);
    let height = shown as f32 * row_height + shown.saturating_sub(1) as f32 * ROW_GAP + LIST_PADDING * 2.0;
    list = if options.len() > style.max_visible {
        list.set_size(SizeValue::Pixels(width), SizeValue::Pixels(height)).set_scrollable(true)
    } else {
        list.set_size(SizeValue::Pixels(width), SizeValue::Fit)
    };

    // Under the box - over it when there's no room left below.
    let (screen_width, screen_height) = app.ui.size();
    let y = if bottom + LIST_GAP + height <= screen_height { bottom + LIST_GAP } else { (top - LIST_GAP - height).max(0.0) };
    let list = list.set_position(PositionValue::Start(left), PositionValue::Start(y));

    let close_key = key.to_owned();
    let mut catcher = UiNode::container()
        .set_size(SizeValue::Percent(100.0), SizeValue::Percent(100.0))
        .set_position(PositionValue::Start(0.0), PositionValue::Start(0.0))
        .on_click(move |app| close_list(app, &close_key))
        .set_child("list", list);
    catcher.resolve(screen_width, screen_height);
    app.ui.add_to_ui(key.to_owned(), catcher);
    // Over everything, modals included - and like a modal, nothing under it
    // can be hovered or clicked while it's open (see App::prepare_ui_content).
    if !app.ui.always_on_top.iter().any(|top_key| top_key == key) {
        app.ui.always_on_top.push(key.to_owned());
    }
    app.ui.has_changed = true;
}

fn close_list(app: &mut App, key: &str) {
    app.ui.renderizable_elements.remove(key);
    app.ui.always_on_top.retain(|top_key| top_key != key);
    app.ui.has_changed = true;
}

/// What a dropdown holds - kept on its `UiNode` (see
/// `UiNode::dropdown_selected`), shared with the list it opens.
pub struct DropdownState {
    options: Rc<Vec<String>>,
    pub(crate) selected: Rc<Cell<usize>>,
    /// Where the box is on screen (left, top, right, bottom) - where its list
    /// opens.
    rect: Rc<Cell<[f32; 4]>>,
    on_select: OnSelect,
}

impl DropdownState {
    /// Every frame (see UiNode::node_content_preparation): the selected
    /// option into the "value" label, and where the box is now.
    pub(crate) fn sync(&self, content: &mut UiNodeContent, font_system: &mut FontSystem, rect: &Rect) {
        self.rect.set([rect.left, rect.top, rect.right, rect.bottom]);
        let UiNodeContent::Container(container) = content else { return };
        let shown = self.options.get(self.selected.get()).map_or("", String::as_str);
        if let Some(label) = container.children.iter_mut().find(|(id, _)| id == "value").and_then(|(_, node)| node.as_label_mut()) {
            label.set_text(font_system, shown, false);
        }
    }

    pub(crate) fn option_count(&self) -> usize {
        self.options.len()
    }
}

impl UiNode {
    /// Runs `f` with the picked option's index whenever one is picked from
    /// this dropdown (see `dropdown`) - no-op on anything else.
    pub fn on_select(self, f: impl FnMut(&mut App, usize) + 'static) -> Self {
        if let Some(state) = self.dropdown_state() {
            *state.on_select.borrow_mut() = Some(Box::new(f));
        }
        self
    }
}
