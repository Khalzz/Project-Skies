//! A single, consistent way to specify a UI color - replaces the previously
//! inconsistent mix of raw `[f32; 4]` arrays (0.0..=1.0 per channel) and
//! `glyphon::Color::rgba(...)` (0..=255 per channel) scattered across every UI
//! definition in `src/game`. `Style` (see `ui_node.rs`) stores colors as `UiColor`
//! directly; converting to whatever a given call site actually needs - `[f32; 4]`
//! for background/border/vertex data, `glyphon::Color` for text rendering - happens
//! via the `From` impls below, at the point it's actually needed (`Style::resolve_concrete`).
//!
//! `Fill` (below `UiColor`) is the richer type `Style.background_color`/
//! `border_color` actually store - either a flat `UiColor` or a multi-stop
//! `Gradient` - since a gradient doesn't make sense for `text_color` (glyphon
//! renders a whole `TextArea` in one `default_color`, no per-vertex interpolation
//! to lean on), that field stays a plain `UiColor`.

use crate::engine::utils::lerps::lerp;
use glyphon::Color as GlyphonColor;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum UiColor {
    /// 0..=255 per channel, fully opaque.
    Rgb(u8, u8, u8),
    /// 0..=255 per channel, including alpha.
    Rgba(u8, u8, u8, u8),
    /// `"RRGGBB"` or `"RRGGBBAA"`, with or without a leading `#` - CSS-style hex.
    /// Parsed on every use (see `to_rgba_u8`) rather than at construction - cheap
    /// enough at this UI's scale (a handful of nodes, not thousands, re-resolved
    /// once per frame each) that caching the parsed value isn't worth the
    /// complexity.
    Hex(&'static str),
}

impl UiColor {
    pub const TRANSPARENT: UiColor = UiColor::Rgba(0, 0, 0, 0);
    pub const WHITE: UiColor = UiColor::Rgb(255, 255, 255);
    pub const BLACK: UiColor = UiColor::Rgb(0, 0, 0);

    /// The same color with just the alpha channel replaced - e.g. fading text
    /// in/out (see `UiNode::set_alpha`) without losing whatever RGB it already
    /// had, regardless of which variant it was originally specified as.
    pub fn with_alpha(&self, alpha: f32) -> UiColor {
        let [r, g, b, _] = self.to_rgba_u8();
        UiColor::Rgba(r, g, b, (alpha.clamp(0.0, 1.0) * 255.0).round() as u8)
    }

    fn to_rgba_u8(&self) -> [u8; 4] {
        match *self {
            UiColor::Rgb(r, g, b) => [r, g, b, 255],
            UiColor::Rgba(r, g, b, a) => [r, g, b, a],
            UiColor::Hex(hex) => parse_hex(hex),
        }
    }
}

/// Accepts `"RRGGBB"`/`"RRGGBBAA"`, with or without a leading `#`. Panics on a
/// malformed literal instead of silently falling back to some placeholder color -
/// every call site is a hardcoded string literal (there's no user-facing/RON path
/// into this), so a bad hex string is a typo to fix at the source, not a runtime
/// condition worth handling gracefully.
fn parse_hex(hex: &str) -> [u8; 4] {
    let hex = hex.strip_prefix('#').unwrap_or(hex);
    let channel = |range: std::ops::Range<usize>| {
        let digits = hex.get(range.clone()).unwrap_or_else(|| panic!("invalid hex color {hex:?} (missing digits at {range:?})"));
        u8::from_str_radix(digits, 16).unwrap_or_else(|_| panic!("invalid hex color {hex:?} (bad digits at {range:?})"))
    };
    match hex.len() {
        6 => [channel(0..2), channel(2..4), channel(4..6), 255],
        8 => [channel(0..2), channel(2..4), channel(4..6), channel(6..8)],
        other => panic!("invalid hex color {hex:?} - expected 6 or 8 hex digits, got {other}"),
    }
}

impl From<UiColor> for [f32; 4] {
    fn from(color: UiColor) -> Self {
        let [r, g, b, a] = color.to_rgba_u8();
        [r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a as f32 / 255.0]
    }
}

impl From<UiColor> for GlyphonColor {
    fn from(color: UiColor) -> Self {
        let [r, g, b, a] = color.to_rgba_u8();
        GlyphonColor::rgba(r, g, b, a)
    }
}

/// Bridges RON-authored colors (see `ui_structure.rs`), hand-written as
/// `[f32; 4]` in the 0.0..=1.0 range - not part of the "how do I specify a color in
/// code" surface itself (use `UiColor::Rgb`/`Rgba`/`Hex` for that), just enough of
/// a bridge so `UiNode::from_component` can hand RON data straight to `Style`
/// without a separate code path.
impl From<[f32; 4]> for UiColor {
    fn from([r, g, b, a]: [f32; 4]) -> Self {
        UiColor::Rgba((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8, (a * 255.0) as u8)
    }
}

pub(crate) fn lerp_rgba(from: [f32; 4], to: [f32; 4], t: f32) -> [f32; 4] {
    [lerp(from[0], to[0], t), lerp(from[1], to[1], t), lerp(from[2], to[2], t), lerp(from[3], to[3], t)]
}

/// CSS's 8 keyword `linear-gradient(to ..., ...)` directions - see `Fill::Gradient`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GradientDirection {
    ToRight,
    ToLeft,
    ToTop,
    ToBottom,
    ToTopRight,
    ToTopLeft,
    ToBottomRight,
    ToBottomLeft,
}

impl GradientDirection {
    /// Where each of a box's 4 corners sits along this gradient (0.0 = the
    /// gradient's own start, 1.0 = its end) - `(top_left, bottom_left,
    /// bottom_right, top_right)`, the same corner order `UiNode::compute_vertices`
    /// builds a plain (non-subdivided) quad's 4 vertices in.
    fn corner_t(&self) -> [f32; 4] {
        match self {
            GradientDirection::ToRight => [0.0, 0.0, 1.0, 1.0],
            GradientDirection::ToLeft => [1.0, 1.0, 0.0, 0.0],
            GradientDirection::ToBottom => [0.0, 1.0, 1.0, 0.0],
            GradientDirection::ToTop => [1.0, 0.0, 0.0, 1.0],
            GradientDirection::ToBottomRight => [0.0, 0.5, 1.0, 0.5],
            GradientDirection::ToTopLeft => [1.0, 0.5, 0.0, 0.5],
            GradientDirection::ToBottomLeft => [0.5, 1.0, 0.5, 0.0],
            GradientDirection::ToTopRight => [0.5, 0.0, 0.5, 1.0],
        }
    }

    /// Whether this direction runs along a single box edge (`ToRight`/`ToLeft`
    /// horizontally, `ToTop`/`ToBottom` vertically) rather than diagonally - see
    /// `Fill::axis_split`, which only subdivides a quad for these 4.
    fn is_axis_aligned(&self) -> bool {
        matches!(self, GradientDirection::ToRight | GradientDirection::ToLeft | GradientDirection::ToTop | GradientDirection::ToBottom)
    }

    /// `true` for the 2 directions whose own t=0.0 is the box's right/bottom edge
    /// rather than its left/top - `UiNode::compute_vertices`' subdivision walks
    /// left-to-right/top-to-bottom regardless of direction, so it flips the split
    /// positions for these 2 to compensate (see its own comment).
    pub(crate) fn is_reversed(&self) -> bool {
        matches!(self, GradientDirection::ToLeft | GradientDirection::ToTop)
    }

    pub(crate) fn is_horizontal(&self) -> bool {
        matches!(self, GradientDirection::ToRight | GradientDirection::ToLeft)
    }
}

/// One color at one point along a gradient - CSS's `<color> <percentage>?` stop
/// syntax, just spelled out as a struct: `GradientStop::new(0.5, UiColor::WHITE)`
/// is CSS's `white 50%`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GradientStop {
    /// 0.0 (the gradient's own start) to 1.0 (its end) - see `GradientDirection`.
    /// Clamped on construction, not just assumed - see `GradientStop::new`.
    pub position: f32,
    pub color: UiColor,
}

impl GradientStop {
    pub fn new(position: f32, color: UiColor) -> GradientStop {
        GradientStop { position: position.clamp(0.0, 1.0), color }
    }
}

impl From<(f32, UiColor)> for GradientStop {
    fn from((position, color): (f32, UiColor)) -> Self {
        GradientStop::new(position, color)
    }
}

/// What `Style.background_color`/`border_color` actually store (see `ui_node.rs`)
/// - either a flat `UiColor`, or a multi-stop gradient across the node's own box.
/// `text_color` stays a plain `UiColor` - glyphon renders a whole `TextArea` in one
/// `default_color`, no per-vertex interpolation for a gradient to ride on the way
/// background/border quads have.
#[derive(Clone, Debug, PartialEq)]
pub enum Fill {
    Solid(UiColor),
    /// One or more color stops (see `GradientStop`) blended along one of 8
    /// CSS-style keyword directions - `UiNode::compute_vertices` renders this by
    /// giving vertices along the gradient's axis the right color at each stop and
    /// letting the GPU's own vertex-color interpolation blend the rest, rather
    /// than any dedicated shader work. For the 4 axis-aligned directions
    /// (`ToRight`/`ToLeft`/`ToTop`/`ToBottom`), that's exact for any number of
    /// stops - the quad gets subdivided into one band per pair of adjacent stops
    /// (see `Fill::axis_split`), each an exact 2-stop linear blend on its own, so
    /// every stop lands exactly where it says. Colors before the first / after the
    /// last stop hold flat, same as CSS.
    ///
    /// The 4 diagonal directions don't get subdivided (a quad is 2 triangles -
    /// see `compute_indices` - and corner colors varying along both axes at once
    /// aren't perfectly planar, so there's no clean way to add an intermediate
    /// diagonal "band" without visible seams) - only the first and last stop's
    /// colors actually show, sampled at the box's own corners (see `Fill::sample`/
    /// `corner_colors`); any stops in between are accepted but have no visible
    /// effect. A true arbitrary-angle multi-stop gradient would need its own
    /// per-fragment shader support, which this doesn't have.
    Gradient { stops: Vec<GradientStop>, direction: GradientDirection },
    /// An intermediate state produced only by an animated `.set_transition(...)`
    /// lerp that involves a gradient on either side (see `ui_node.rs`'s own
    /// `lerp_fill`) - 4 independent corner colors with no gradient shape behind
    /// them, since an arbitrary lerp between two different gradients' (or a
    /// gradient's and a solid's) corners generally isn't itself expressible as a
    /// clean gradient. Renders through the same plain 4-corner fast path a
    /// 2-stop gradient already does (see `UiNode::compute_quad`) -
    /// `Fill::axis_split` is always `None` for this, so a multi-stop gradient's
    /// exact per-stop subdivision only re-appears once the transition actually
    /// settles back onto a real `Gradient` target. Not meant to be built
    /// directly - construct a `Solid`/`Gradient` instead.
    Corners([[f32; 4]; 4]),
}

impl Fill {
    /// Sugar for the common 2-stop case - `Fill::linear(black, transparent,
    /// ToRight)` is `Fill::gradient(ToRight, [(0.0, black), (1.0, transparent)])`.
    pub fn linear(from: UiColor, to: UiColor, direction: GradientDirection) -> Fill {
        Fill::gradient(direction, [GradientStop::new(0.0, from), GradientStop::new(1.0, to)])
    }

    /// Builds a `Gradient` from any number of stops, in any order - sorted by
    /// position here so every other method on `Fill` can assume ascending order.
    pub fn gradient(direction: GradientDirection, stops: impl Into<Vec<GradientStop>>) -> Fill {
        let mut stops: Vec<GradientStop> = stops.into();
        assert!(!stops.is_empty(), "Fill::gradient needs at least one stop");
        stops.sort_by(|a, b| a.position.total_cmp(&b.position));
        Fill::Gradient { stops, direction }
    }

    /// This fill's color at an arbitrary point `t` (0.0..=1.0) along its own
    /// gradient axis, regardless of where any actual vertex ends up - a `Solid`
    /// fill ignores `t` entirely; a `Gradient` does a piecewise-linear blend
    /// between whichever two stops `t` falls between, clamping to the nearest
    /// stop's color outside the stops' own range (before the first / after the
    /// last), same as CSS.
    pub(crate) fn sample(&self, t: f32) -> [f32; 4] {
        match self {
            Fill::Solid(color) => (*color).into(),
            Fill::Gradient { stops, .. } => {
                let first = stops.first().expect("Fill::gradient always has at least one stop");
                let last = stops.last().expect("Fill::gradient always has at least one stop");
                if t <= first.position {
                    return first.color.into();
                }
                if t >= last.position {
                    return last.color.into();
                }
                for pair in stops.windows(2) {
                    let (a, b) = (pair[0], pair[1]);
                    if t <= b.position {
                        let span = (b.position - a.position).max(f32::EPSILON);
                        return lerp_rgba(a.color.into(), b.color.into(), (t - a.position) / span);
                    }
                }
                last.color.into() // unreachable given the clamps above, but a safe fallback
            }
            // Never actually reached - axis_split() is always None for this
            // variant (see its own doc comment), so nothing calls sample() on
            // one; just needs some value to keep this match total.
            Fill::Corners(c) => c[0],
        }
    }

    /// The 4 corner colors this fill resolves to, in the same order
    /// `UiNode::compute_vertices` builds a plain (non-subdivided) quad's vertices
    /// - `(top_left, bottom_left, bottom_right, top_right)`. A `Solid` fill is
    /// just that color 4 times, which is also what makes an animated
    /// `.set_transition(...)` between a solid color and a gradient work with no
    /// special-casing (see `ResolvedStyle::lerp` in `ui_node.rs`).
    pub fn corner_colors(&self) -> [[f32; 4]; 4] {
        match self {
            Fill::Solid(color) => {
                let c: [f32; 4] = (*color).into();
                [c, c, c, c]
            }
            Fill::Gradient { direction, .. } => direction.corner_t().map(|t| self.sample(t)),
            Fill::Corners(c) => *c,
        }
    }

    /// If this fill needs more than a plain quad's 4 corners to render every stop
    /// exactly - an axis-aligned `Gradient` whose stops, once normalized to always
    /// include both 0.0 and 1.0 (a stop list that doesn't reach one or both ends,
    /// e.g. just `[(0.0, black), (0.5, transparent)]`, holds flat for the
    /// remainder, same as CSS - see `Fill::sample`) has more than 2 distinct
    /// points - the direction plus those effective 0.0..=1.0 split positions
    /// (sorted, deduped). `None` for everything else (`Solid`, a gradient whose
    /// stops are already exactly `[0.0, 1.0]` with nothing in between, or a
    /// diagonal direction) - those all render fine off the plain 4-corner path
    /// already, see `UiNode::compute_vertices`. This is why the check below is on
    /// the *normalized* position count, not the raw number of stops given to
    /// `Fill::gradient` - 2 stops that don't already span the full 0.0..1.0 range
    /// still need a 3rd effective point (wherever the hold region starts) to
    /// render correctly, even though there are only 2 actual colors.
    pub fn axis_split(&self) -> Option<(GradientDirection, Vec<f32>)> {
        let Fill::Gradient { stops, direction } = self else { return None };
        if !direction.is_axis_aligned() {
            return None;
        }
        let mut positions: Vec<f32> = stops.iter().map(|s| s.position).collect();
        positions.push(0.0);
        positions.push(1.0);
        positions.sort_by(f32::total_cmp);
        positions.dedup_by(|a, b| (*a - *b).abs() < 0.0001);
        if positions.len() <= 2 {
            return None;
        }
        Some((*direction, positions))
    }
}

impl From<UiColor> for Fill {
    fn from(color: UiColor) -> Self {
        Fill::Solid(color)
    }
}
