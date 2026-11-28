/*
    The head-up display: the flight symbology (the F-16's base NAV page) out of what the sensors read.

    A real HUD is collimated: its image is focused at infinity, so every symbol is a direction from the pilot's
    eye, not a spot on the glass. The flight path marker sits over where the jet is really going and the
    horizon line over the real horizon, wherever the head moves. So everything here is placed on the HUD's
    tangent plane: x right, y up, the tangent of the angle off the boresight (the jet's nose, -Z), so a
    straight line in the world stays a straight line on the HUD. `deg()` gives a place from an angle.

    It works in two steps:
    - `Hud::symbology`: the logic. Sensors in, lines and text on the tangent plane out, clipped to what the
      combiner shows. Nothing about rendering, so the same picture could feed a replay or a debug view.
    - `draw`: turns each place into a direction from the jet, and puts it far ahead of the eye along it.

    What's on it (NAV, the base page):
    - Boresight cross: where the nose points.
    - Flight path marker: where the jet goes, from the INS velocity (so it drifts with the wind like the
      real one), held at the edge of the field of view when it would leave it.
    - Horizon and pitch ladder: every 5 degrees, solid above the horizon and dashed below, end ticks pointing
      to the horizon, centered on the flight path marker. Built in the world, so they bank with it by
      themselves.
    - Airspeed (left, calibrated knots), altitude (right, feet, altimeter on 29.92) and heading (bottom) tapes.
    - G (top left), Mach and the highest G pulled (below the airspeed).
*/

use nalgebra::{UnitQuaternion, Vector2, Vector3};
use pankarta_software::prelude::App;
use pankarta_software::rendering::sdf_text::TextFrame;
use pankarta_software::rendering::thick_lines::{convex_mask, LineInstance};

use crate::prefabs::aircraft::sensors::sensors::Sensors;
use crate::ui::HUD_FONT;

// The render sets it draws in, registered by the scene and drawn after the world
pub const HUD_LINES: &str = "hud_lines";
pub const HUD_TEXT: &str = "hud_text";

const KNOTS_PER_MS: f32 = 1.943_844;
const FEET_PER_M: f32 = 3.280_84;

const COLLIMATION: f32 = 1_000.0; // m ahead of the eye its picture is drawn: far enough to be at infinity
const COLOR: [f32; 4] = [0.35, 1.0, 0.45, 0.9];
const LINE_WIDTH: f32 = 1.6; // px
/*
    The whole layout's size: every place and size below is multiplied by it, around the boresight, so the
    picture keeps its proportions. The ladder's 5 degrees between rungs and where the flight path marker sits
    never scale (they're the world's), nor does the glass (`Optics`).
*/
const HUD_SCALE: f32 = 0.85;
const TEXT_SIZE: f32 = 0.6; // degrees
const CHAR_WIDTH: f32 = 0.6; // of the text size, to place text by its edge

// Where things sit, in degrees off the boresight (x right, y up)
const TAPE_CENTER: f32 = -3.0;
const TAPE_HALF: f32 = 4.5; // each tape's half height
const AIRSPEED_X: f32 = -9.5;
const ALTITUDE_X: f32 = 9.5;
const HEADING_Y: f32 = -12.0;
const HEADING_HALF: f32 = 6.0;
const LONG_TICK: f32 = 0.6;
const SHORT_TICK: f32 = 0.3;

// A place on the HUD from an angle off the boresight
fn deg(angle: f32) -> f32 {
    angle.to_radians().tan()
}

// A place or size in the layout, from degrees, scaled by HUD_SCALE
fn scaled(angle: f32) -> f32 {
    deg(angle * HUD_SCALE)
}

fn at(x: f32, y: f32) -> Vector2<f32> {
    Vector2::new(scaled(x), scaled(y))
}

// A direction in the jet's frame on the HUD, if it's ahead
fn on_hud(body: Vector3<f32>) -> Option<Vector2<f32>> {
    (body.z < -1e-4).then(|| Vector2::new(body.x / -body.z, body.y / -body.z))
}

/*
    What the pilot can see the picture through. Two limits, both real:
    - the projector's field of view: how far off the boresight its optics can place a symbol (degrees);
    - the combiner glass: a symbol only shows where the eye looks through it. Its corners are fixed on the jet,
      so seen from the eye they make an outline that changes as the head moves (a tilted pane close to the
      eye looks wider at its nearer edge); it's worked out again every frame from where the eye is.
*/
pub struct Optics {
    pub left: f32,
    pub right: f32,
    pub bottom: f32,
    pub top: f32,
    pub glass: Vec<Vector3<f32>>, // the combiner's outline, in order, meters in the jet's frame
}

impl Optics {
    /*
        Where the picture shows from `eye` (in the jet's frame), as a convex outline on the HUD's plane: the glass
        seen from the eye, cut to the field of view. If a corner of the glass isn't ahead of the eye (the head
        through the pane) it's the field of view alone.
    */
    pub fn visible(&self, eye: Vector3<f32>) -> Vec<Vector2<f32>> {
        let field = [
            (Vector2::new(1.0, 0.0), -deg(self.left)),
            (Vector2::new(-1.0, 0.0), deg(self.right)),
            (Vector2::new(0.0, 1.0), -deg(self.bottom)),
            (Vector2::new(0.0, -1.0), deg(self.top)),
        ];
        let glass: Option<Vec<Vector2<f32>>> = self.glass.iter().map(|corner| on_hud(corner - eye)).collect();
        let mut area = glass.filter(|glass| glass.len() >= 3).unwrap_or_else(|| {
            vec![
                Vector2::new(deg(self.left), deg(self.bottom)),
                Vector2::new(deg(self.right), deg(self.bottom)),
                Vector2::new(deg(self.right), deg(self.top)),
                Vector2::new(deg(self.left), deg(self.top)),
            ]
        });
        // Sutherland-Hodgman: keep the part on the inside of each edge (n·p + c >= 0)
        for (normal, offset) in field {
            let side = |point: &Vector2<f32>| normal.dot(point) + offset;
            let mut kept = vec![];
            for (index, from) in area.iter().enumerate() {
                let to = &area[(index + 1) % area.len()];
                let (a, b) = (side(from), side(to));
                if a >= 0.0 {
                    kept.push(*from);
                }
                if (a >= 0.0) != (b >= 0.0) {
                    kept.push(from + (to - from) * (a / (a - b)));
                }
            }
            area = kept;
            if area.len() < 3 {
                return vec![];
            }
        }
        area
    }
}

// `point`, or where the way to it from the boresight leaves the convex `area`, `margin` inside
fn clamp_to(area: &[Vector2<f32>], point: Vector2<f32>, margin: f32) -> Vector2<f32> {
    if area.len() < 3 {
        return point;
    }
    let middle = area.iter().sum::<Vector2<f32>>() / area.len() as f32;
    let mut reach = 1.0_f32;
    for (index, &from) in area.iter().enumerate() {
        let along = area[(index + 1) % area.len()] - from;
        let Some(normal) = Vector2::new(-along.y, along.x).try_normalize(1e-9) else {
            continue;
        };
        let normal = if normal.dot(&(middle - from)) < 0.0 { -normal } else { normal };
        let offset = -normal.dot(&from);
        let (start, end) = (offset, normal.dot(&point) + offset); // at the boresight, at the point
        if end < margin && start > end {
            reach = reach.min(((start - margin) / (start - end)).max(0.0));
        }
    }
    point * reach
}

#[derive(Clone, Copy)]
pub enum Align {
    Left,
    Center,
    Right,
}

pub struct HudText {
    pub at: Vector2<f32>,
    pub text: String,
    pub size: f32, // degrees tall
    pub align: Align,
    pub tilt: f32, // radians, + counterclockwise: the ladder's numbers bank with it
}

// The picture: lines and text on the HUD's tangent plane
#[derive(Default)]
pub struct Symbology {
    pub lines: Vec<[Vector2<f32>; 2]>,
    pub texts: Vec<HudText>,
}

impl Symbology {
    fn line(&mut self, from: Vector2<f32>, to: Vector2<f32>) {
        self.lines.push([from, to]);
    }

    fn path(&mut self, points: &[Vector2<f32>]) {
        for pair in points.windows(2) {
            self.line(pair[0], pair[1]);
        }
    }

    // `dashes` dashes with gaps as long between them
    fn dashed(&mut self, from: Vector2<f32>, to: Vector2<f32>, dashes: usize) {
        let pieces = (2 * dashes - 1) as f32;
        for dash in 0..dashes {
            let start = from + (to - from) * (2 * dash) as f32 / pieces;
            let end = from + (to - from) * (2 * dash + 1) as f32 / pieces;
            self.line(start, end);
        }
    }

    fn circle(&mut self, center: Vector2<f32>, radius: f32, segments: usize) {
        let points: Vec<Vector2<f32>> = (0..=segments)
            .map(|step| {
                let angle = std::f32::consts::TAU * step as f32 / segments as f32;
                center + Vector2::new(angle.cos(), angle.sin()) * radius
            })
            .collect();
        self.path(&points);
    }

    fn rectangle(&mut self, center: Vector2<f32>, half: Vector2<f32>) {
        let corner = |x: f32, y: f32| center + Vector2::new(half.x * x, half.y * y);
        self.path(&[
            corner(-1.0, -1.0),
            corner(1.0, -1.0),
            corner(1.0, 1.0),
            corner(-1.0, 1.0),
            corner(-1.0, -1.0),
        ]);
    }

    fn text(&mut self, at: Vector2<f32>, text: String, align: Align) {
        self.texts.push(HudText {
            at,
            text,
            size: TEXT_SIZE,
            align,
            tilt: 0.0,
        });
    }
}

pub struct Hud {
    pub optics: Optics,
    max_g: f32, // the highest load factor pulled since the wheels left the ground
}

impl Hud {
    pub fn new(optics: Optics) -> Hud {
        Hud { optics, max_g: 1.0 }
    }

    /*
        The F-16's: the combiner pane from the cockpit model (its "Glass" mesh), 17 cm wide and 18 tall, leaning
        35 deg back toward the pilot. The field of view leaves the sides and top to the glass; its bottom is
        where the up-front controls cover the pane from the seat.
    */
    pub fn f16() -> Hud {
        Hud::new(Optics {
            left: -15.0,
            right: 15.0,
            bottom: -12.8,
            top: 13.0,
            glass: vec![
                Vector3::new(-0.082, 0.844, -6.921), // bottom left
                Vector3::new(0.085, 0.844, -6.921),  // bottom right
                Vector3::new(0.085, 1.029, -6.795),  // top right
                Vector3::new(0.038, 1.039, -6.791),  // the crest
                Vector3::new(-0.034, 1.039, -6.791),
                Vector3::new(-0.082, 1.029, -6.795), // top left
            ],
        })
    }

    // Every physics step, after the sensors read
    pub fn update(&mut self, sensors: &Sensors) {
        if sensors.weight_on_wheels {
            self.max_g = 1.0;
        } else {
            self.max_g = self.max_g.max(sensors.load_factor);
        }
    }

    /*
        The whole picture, uncut: the glass masks it where it's drawn (see `draw`), so a symbol at its edge is
        cut there, not dropped. `area`: what shows from the eye (`Optics::visible`), to hold the flight path
        marker inside.
    */
    pub fn symbology(&self, sensors: &Sensors, area: &[Vector2<f32>]) -> Symbology {
        let mut picture = Symbology::default();
        boresight(&mut picture);
        let path = flight_path(sensors);
        let marker = on_hud(sensors.attitude.inverse() * path)
            .map_or(Vector2::zeros(), |marker| clamp_to(area, marker, deg(1.0)));
        pitch_ladder(&mut picture, sensors, path);
        flight_path_marker(&mut picture, marker);
        airspeed_tape(&mut picture, sensors.calibrated_airspeed * KNOTS_PER_MS);
        altitude_tape(&mut picture, sensors.pressure_altitude * FEET_PER_M);
        heading_tape(&mut picture, sensors.heading);
        data(&mut picture, sensors, self.max_g);
        picture
    }
}

// Where the jet goes, in the world: the INS velocity, or the nose while it barely moves
fn flight_path(sensors: &Sensors) -> Vector3<f32> {
    sensors
        .velocity
        .try_normalize(1.0)
        .unwrap_or_else(|| sensors.attitude * -Vector3::z())
}

// The gun cross: a small cross with a gap at the boresight
fn boresight(picture: &mut Symbology) {
    let (gap, end) = (scaled(0.3), scaled(1.0));
    for direction in [Vector2::x(), -Vector2::x(), Vector2::y(), -Vector2::y()] {
        picture.line(direction * gap, direction * end);
    }
}

// A circle with wings and a tail, always upright on the HUD
fn flight_path_marker(picture: &mut Symbology, center: Vector2<f32>) {
    let radius = scaled(0.45);
    picture.circle(center, radius, 16);
    for side in [-1.0, 1.0] {
        picture.line(
            center + Vector2::new(side * radius, 0.0),
            center + Vector2::new(side * (radius + scaled(0.9)), 0.0),
        );
    }
    picture.line(center + Vector2::new(0.0, radius), center + Vector2::new(0.0, radius + scaled(0.5)));
}

/*
    A rung for every 5 degrees of pitch, in the world: at elevation e above the horizon, toward the flight
    path's heading. Each one lies on the plane square to its center direction, so it's straight on the HUD
    too; its `side` (level, to the right) and `rise` (up along the sky) are that plane's axes. Drawn only
    within 25 degrees of the flight path, the rest would be clipped anyway.
*/
fn pitch_ladder(picture: &mut Symbology, sensors: &Sensors, path: Vector3<f32>) {
    let world_up = Vector3::y();
    let level = Vector3::new(path.x, 0.0, path.z)
        .try_normalize(1e-3)
        .or_else(|| {
            let nose = sensors.attitude * -Vector3::z();
            Vector3::new(nose.x, 0.0, nose.z).try_normalize(1e-3)
        });
    let Some(level) = level else { return };
    let side = level.cross(&world_up);
    let path_elevation = path.y.clamp(-1.0, 1.0).asin().to_degrees();
    let to_body = sensors.attitude.inverse();

    for step in -18..=18 {
        let elevation = step as f32 * 5.0;
        if (elevation - path_elevation).abs() > 25.0 || elevation.abs() >= 90.0 {
            continue;
        }
        let e = elevation.to_radians();
        let center = level * e.cos() + world_up * e.sin();
        let rise = world_up * e.cos() - level * e.sin();
        // A point on the rung's plane: `along` to the right, `up` along the sky (tangents)
        let point = |along: f32, up: f32| on_hud(to_body * (center + side * along + rise * up));

        let (gap, half) = if step == 0 { (scaled(2.0), scaled(20.0)) } else { (scaled(2.0), scaled(5.0)) };
        let tick = scaled(0.8) * -elevation.signum(); // toward the horizon
        for hand in [-1.0, 1.0] {
            let (Some(inner), Some(outer)) = (point(hand * gap, 0.0), point(hand * half, 0.0)) else {
                continue;
            };
            if step == 0 {
                // The horizon runs wide, but breaks where it would cross the tapes so their numbers stay clear
                for [from, to] in around_tapes(inner, outer) {
                    picture.line(from, to);
                }
                continue;
            }
            if elevation > 0.0 {
                picture.line(inner, outer);
            } else {
                picture.dashed(inner, outer, 3);
            }
            if let Some(end) = point(hand * half, tick) {
                picture.line(outer, end);
            }
            // The number past the end, banked with the rung
            if let Some(label) = point(hand * (half + scaled(1.3)), tick / 2.0) {
                let tilt = (outer - inner) * hand;
                picture.texts.push(HudText {
                    at: label,
                    text: format!("{}", elevation.abs() as i32),
                    size: TEXT_SIZE,
                    align: Align::Center,
                    tilt: tilt.y.atan2(tilt.x),
                });
            }
        }
    }
}

// Where each tape sits with its numbers, caret and box: (low corner, high corner)
fn tape_areas() -> [(Vector2<f32>, Vector2<f32>); 2] {
    let (low, high) = (TAPE_CENTER - TAPE_HALF - 0.5, TAPE_CENTER + TAPE_HALF + 0.5);
    [
        (at(AIRSPEED_X - 3.6, low), at(AIRSPEED_X + 0.9, high)),
        (at(ALTITUDE_X - 0.9, low), at(ALTITUDE_X + 4.0, high)),
    ]
}

// A line without the parts that fall on the tapes
fn around_tapes(from: Vector2<f32>, to: Vector2<f32>) -> Vec<[Vector2<f32>; 2]> {
    let mut pieces = vec![[from, to]];
    for (low, high) in tape_areas() {
        pieces = pieces
            .into_iter()
            .flat_map(|[from, to]| outside_box(from, to, low, high))
            .collect();
    }
    pieces
}

// The parts of a line outside a box: before it goes in and after it comes out
fn outside_box(
    from: Vector2<f32>,
    to: Vector2<f32>,
    low: Vector2<f32>,
    high: Vector2<f32>,
) -> Vec<[Vector2<f32>; 2]> {
    let delta = to - from;
    let (mut enter, mut leave) = (0.0_f32, 1.0_f32);
    for (rate, start, end) in [(delta.x, from.x, (low.x, high.x)), (delta.y, from.y, (low.y, high.y))] {
        if rate.abs() < 1e-12 {
            if start < end.0 || start > end.1 {
                return vec![[from, to]];
            }
            continue;
        }
        let (a, b) = ((end.0 - start) / rate, (end.1 - start) / rate);
        enter = enter.max(a.min(b));
        leave = leave.min(a.max(b));
    }
    if enter >= leave {
        return vec![[from, to]];
    }
    let mut pieces = vec![];
    if enter > 0.0 {
        pieces.push([from, from + delta * enter]);
    }
    if leave < 1.0 {
        pieces.push([from + delta * leave, to]);
    }
    pieces
}

// Calibrated knots, ±50 kt in view: a tick every 10, a number every 50, the reading boxed at the middle
fn airspeed_tape(picture: &mut Symbology, knots: f32) {
    let per_knot = (scaled(TAPE_CENTER + TAPE_HALF) - scaled(TAPE_CENTER - TAPE_HALF)) / 100.0;
    let middle = scaled(TAPE_CENTER);
    let x = scaled(AIRSPEED_X);
    let first = ((knots - 50.0) / 10.0).ceil() as i32;
    for mark in first..=first + 10 {
        let value = mark * 10;
        if value < 0 || (value as f32 - knots).abs() > 50.0 {
            continue;
        }
        let y = middle + (value as f32 - knots) * per_knot;
        let long = value % 50 == 0;
        picture.line(Vector2::new(x, y), Vector2::new(x - scaled(if long { LONG_TICK } else { SHORT_TICK }), y));
        if long && (y - middle).abs() > scaled(0.8) {
            picture.text(Vector2::new(x - scaled(0.9), y), value.to_string(), Align::Right);
        }
    }
    // The caret pointing at the tape, and the reading in its box
    picture.path(&[
        Vector2::new(x + scaled(0.7), middle + scaled(0.3)),
        Vector2::new(x + scaled(0.15), middle),
        Vector2::new(x + scaled(0.7), middle - scaled(0.3)),
    ]);
    let reading = Vector2::new(x - scaled(2.2), middle);
    picture.rectangle(reading, Vector2::new(scaled(1.2), scaled(0.5)));
    picture.text(reading, format!("{:.0}", knots.max(0.0)), Align::Center);
}

// Feet on 29.92, ±500 ft in view: a tick every 100, a number every 500 ("10,5" for 10,500), the reading boxed
fn altitude_tape(picture: &mut Symbology, feet: f32) {
    let per_foot = (scaled(TAPE_CENTER + TAPE_HALF) - scaled(TAPE_CENTER - TAPE_HALF)) / 1_000.0;
    let middle = scaled(TAPE_CENTER);
    let x = scaled(ALTITUDE_X);
    let first = ((feet - 500.0) / 100.0).ceil() as i32;
    for mark in first..=first + 10 {
        let value = mark * 100;
        if (value as f32 - feet).abs() > 500.0 {
            continue;
        }
        let y = middle + (value as f32 - feet) * per_foot;
        let long = value % 500 == 0;
        picture.line(Vector2::new(x, y), Vector2::new(x + scaled(if long { LONG_TICK } else { SHORT_TICK }), y));
        if long && (y - middle).abs() > scaled(0.8) {
            let label = format!("{},{}", value / 1_000, (value % 1_000).abs() / 100);
            picture.text(Vector2::new(x + scaled(0.9), y), label, Align::Left);
        }
    }
    picture.path(&[
        Vector2::new(x - scaled(0.7), middle + scaled(0.3)),
        Vector2::new(x - scaled(0.15), middle),
        Vector2::new(x - scaled(0.7), middle - scaled(0.3)),
    ]);
    let reading = Vector2::new(x + scaled(2.4), middle);
    picture.rectangle(reading, Vector2::new(scaled(1.4), scaled(0.5)));
    picture.text(reading, thousands((feet / 10.0).round() as i32 * 10), Align::Center);
}

// "10,520"
fn thousands(value: i32) -> String {
    let digits = value.abs().to_string();
    let mut grouped = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(digit);
    }
    if value < 0 {
        format!("-{grouped}")
    } else {
        grouped
    }
}

// ±15 degrees in view: a tick every 5, the tens every 10 ("09" for 090), the caret above the reading
fn heading_tape(picture: &mut Symbology, heading: f32) {
    let per_degree = scaled(HEADING_HALF) / 15.0;
    let y = scaled(HEADING_Y);
    let first = ((heading - 15.0) / 5.0).ceil() as i32;
    for mark in first..=first + 6 {
        let value = mark * 5;
        let offset = value as f32 - heading;
        if offset.abs() > 15.0 {
            continue;
        }
        let x = offset * per_degree;
        let long = value.rem_euclid(10) == 0;
        picture.line(Vector2::new(x, y), Vector2::new(x, y - scaled(if long { 0.5 } else { 0.25 })));
        if long {
            let tens = value.rem_euclid(360) / 10;
            let label = format!("{:02}", if tens == 0 { 36 } else { tens });
            picture.text(Vector2::new(x, y - scaled(1.1)), label, Align::Center);
        }
    }
    picture.path(&[
        Vector2::new(-scaled(0.3), y + scaled(0.6)),
        Vector2::new(0.0, y + scaled(0.1)),
        Vector2::new(scaled(0.3), y + scaled(0.6)),
    ]);
}

// G at the top left; Mach and the highest G pulled under the airspeed
fn data(picture: &mut Symbology, sensors: &Sensors, max_g: f32) {
    picture.text(at(-10.5, 6.0), format!("{:.1}", sensors.load_factor), Align::Left);
    let below = TAPE_CENTER - TAPE_HALF;
    picture.text(at(-10.0, below - 1.5), format!("{:.2}", sensors.mach), Align::Left);
    picture.text(at(-10.0, below - 2.7), format!("{:.1}", max_g), Align::Left);
}

// Empties the HUD's sets: nothing shows unless it's drawn again this frame
pub fn clear(app: &mut App) {
    if let Some(lines) = app.renderer.render_sets.get_lines_mut(HUD_LINES) {
        lines.dynamic.clear();
    }
    if let Some(text) = app.renderer.render_sets.get_text_mut(HUD_TEXT) {
        text.clear();
    }
}

// The camera the HUD is seen through: the pilot's eye, where it looks, its vertical field of view (degrees)
pub struct View {
    pub eye: Vector3<f32>,
    pub rotation: UnitQuaternion<f32>,
    pub fovy: f32,
}

/*
    The picture at infinity: each place on the HUD is a direction from the jet (x, y, -1 in its frame), drawn
    `COLLIMATION` meters from the eye along it. `attitude` is the jet's as it's drawn this frame, so the
    conformal symbols stay on what they point at. All of it is drawn, masked to `area` (what shows through the
    glass) on the screen, pixel by pixel: the eye and the camera are the same point, so the area's outline is
    where the glass's edges are on the screen.
*/
pub fn draw(app: &mut App, picture: &Symbology, area: &[Vector2<f32>], view: &View, attitude: UnitQuaternion<f32>) {
    let (width, height) = (app.window.pixel_size.width as f32, app.window.pixel_size.height as f32);
    let focal = 1.0 / (view.fovy.to_radians() / 2.0).tan();
    let on_screen = |point: &Vector2<f32>| {
        let seen = view.rotation.inverse() * (attitude * Vector3::new(point.x, point.y, -1.0));
        (seen.z < -1e-4).then(|| {
            let (x, y) = (seen.x / -seen.z * focal * height / width, seen.y / -seen.z * focal);
            [(x + 1.0) / 2.0 * width, (1.0 - y) / 2.0 * height]
        })
    };
    // Looking away, past where the glass can be seen: nothing to draw
    let Some(outline) = area.iter().map(on_screen).collect::<Option<Vec<[f32; 2]>>>() else {
        return;
    };
    let Some(mask) = convex_mask(&outline, 0.0) else { return };

    let eye = view.eye;
    let world = |point: Vector2<f32>| {
        eye + attitude * Vector3::new(point.x, point.y, -1.0).normalize() * COLLIMATION
    };

    if let Some(lines) = app.renderer.render_sets.get_lines_mut(HUD_LINES) {
        lines.mask = Some(mask.clone());
        for [from, to] in &picture.lines {
            lines.dynamic.push(LineInstance {
                start: world(*from).into(),
                end: world(*to).into(),
                color: COLOR,
                width: LINE_WIDTH,
            });
        }
    }

    if let Some(text) = app.renderer.render_sets.get_text_mut(HUD_TEXT) {
        text.mask = Some(mask);
        for label in &picture.texts {
            let size = scaled(label.size);
            let along = Vector2::new(label.tilt.cos(), label.tilt.sin());
            let half_width = label.text.chars().count() as f32 * CHAR_WIDTH * size / 2.0;
            let center = match label.align {
                Align::Left => label.at + along * half_width,
                Align::Center => label.at,
                Align::Right => label.at - along * half_width,
            };
            let right = attitude * Vector3::new(along.x, along.y, 0.0);
            let up = attitude * Vector3::new(-along.y, along.x, 0.0);
            text.push(
                &label.text,
                HUD_FONT,
                TextFrame::world(world(center), right, up, size * COLLIMATION),
                COLOR,
            );
        }
    }
}
