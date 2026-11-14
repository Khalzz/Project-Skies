//! The flight HUD and helmet display - drawn onto the player's view each
//! frame (`update_hud`, `draw_hmd`), its power-on after a cinematic, its
//! text (`ui_control`), and F11's layout of the cockpit HUD plate
//! (`HudLayout`). Its state is `HudState`, the scene's `hud`.

use super::*;

pub(super) struct HudState {
    // F6 - the center HUD is drawn far ahead of the jet and only shows
    // through its combiner glass. Off: that center stays stuck to the screen.
    pub(super) collimated_hud: bool,
    // The helmet display is switched on ("toggle_hmd") - see draw_hmd.
    pub(super) hmd_enabled: bool,
    // Seconds into the HUD power-on after a cinematic returns to the
    // cockpit. None once it's on for good - see tick_hud_boot.
    pub(super) hud_boot: Option<f32>,
    // The flight HUD's brightness in g_vision (see tick_hud_boot) - it
    // stays until changed.
    pub(super) hud_opacity: f32,
    // F11's cockpit HUD layout panel.
    pub(super) hud_layout: HudLayout,
    // The player's velocity in its own frame (x left, y up, z forward), from
    // the latest physics pose - see update_hud's velocity marker.
    pub(super) player_body_velocity: Option<Vector3<f32>>,
    // When the HUD's text was last rewritten - it's throttled (see ui_control).
    pub(super) last_ui_update: Instant,
    // A running animation has the HUD hidden (as of last frame) - see
    // update_g_vision.
    pub(super) animation_hides_hud: bool,
}

/// Live cockpit HUD layout, edited from the F11 panel. The numbers are what
/// the HUD used before the panel existed. They last for the session.
#[derive(Clone, Copy)]
pub struct HudLayout {
    pub open: bool,
    /// Width and height as fractions of the screen, before `scale`.
    pub width: f32,
    pub height: f32,
    /// Shrinks the whole HUD around its center, letters included.
    /// 1 is the built-in size. Width and height still set the frame.
    pub scale: f32,
    /// Pixels from the screen center. Positive X is right, positive Y is down.
    pub center_x: f32,
    pub center_y: f32,
    /// How much of the combiner the center HUD covers when F6 is on.
    /// 1 sits the frame on the pane. Higher magnifies it, and the glass clips
    /// whatever crosses the edge — past about 2 the side marks are cut off.
    pub glass_fill: f32,
    /// Metres in front of the design eye, when the HUD stays with the jet.
    pub plate_distance: f32,
    /// Metres right and up from that plate's center.
    pub plate_shift_right: f32,
    pub plate_shift_up: f32,
    /// A number field in the panel has focus — flight keys stay off.
    pub typing: bool,
}

impl HudLayout {
    /// Back to the built-in layout. The panel stays however it was.
    fn reset_values(&mut self) {
        let open = self.open;
        let typing = self.typing;
        *self = Self::default();
        self.open = open;
        self.typing = typing;
    }

    /// The scale the HUD is drawn at. Stepped so a drag doesn't mint a new
    /// font size every frame (that fills the text atlas until it panics).
    fn draw_scale(self) -> f32 {
        let scale = self.scale.clamp(0.1, 4.0);
        (scale * 20.0).round() / 20.0
    }
}

impl Default for HudLayout {
    fn default() -> Self {
        Self {
            open: false,
            width: 0.333,
            height: 0.5,
            scale: 0.35,
            center_x: 0.0,
            center_y: 0.0,
            glass_fill: 0.9,
            plate_distance: 0.85,
            plate_shift_right: 0.0,
            plate_shift_up: 0.07,
            typing: false,
        }
    }
}

pub(super) fn hud_readout_font(key: &str) -> f32 {
    play_ui::HUD_READOUTS
        .iter()
        .find(|(name, _, _, _)| *name == key)
        .map(|(_, _, size, _)| *size)
        .unwrap_or(12.0)
}

/// Font scale for the atlas: the layout scale times whatever the glass
/// zoom already applied, on the same 1/16 steps that zoom uses.
pub(super) fn hud_font_scale(layout_scale: f32, print_scale: f32) -> f32 {
    let scale = (layout_scale * print_scale).clamp(0.1, 6.0);
    (scale * 16.0).round() / 16.0
}

/// F11's HUD layout panel. `Some` is text for the clipboard.
pub fn draw_hud_layout(ctx: &egui::Context, layout: &mut HudLayout) -> Option<String> {
    let mut copied = None;
    let mut open = layout.open;
    egui::Window::new("HUD layout")
        .open(&mut open)
        .default_pos(egui::pos2(20.0, 20.0))
        .resizable(false)
        .show(ctx, |ui| {
            ui.label("F11 closes this. Drag a number, or click it and type.");
            ui.separator();

            ui.label("Size");
            ui.add(egui::Slider::new(&mut layout.width, 0.02..=2.5).text("width"));
            ui.add(egui::Slider::new(&mut layout.height, 0.02..=2.5).text("height"));
            ui.label("Fractions of the screen. The frame can run past the edges.");
            ui.add(egui::Slider::new(&mut layout.scale, 0.15..=3.0).step_by(0.05).text("scale"));
            ui.label("Shrinks the whole HUD, letters included, without a separate text size.");
            ui.separator();

            ui.label("Center");
            ui.add(egui::Slider::new(&mut layout.center_x, -2500.0..=2500.0).text("X"));
            ui.add(egui::Slider::new(&mut layout.center_y, -2500.0..=2500.0).text("Y"));
            ui.label("Pixels from the screen center. Positive X is right, positive Y is down.");
            ui.separator();

            ui.label("Combiner glass (F6)");
            ui.add(egui::Slider::new(&mut layout.glass_fill, 0.05..=5.0).text("fill"));
            ui.separator();

            ui.label("Jet plate");
            ui.label("Used when Video has the HUD stay with the jet. It's drawn at infinity (collimated), like a real HUD; distance only sets how far the shifts turn it.");
            ui.add(egui::Slider::new(&mut layout.plate_distance, 0.05..=6.0).text("distance").suffix(" m"));
            ui.add(egui::Slider::new(&mut layout.plate_shift_right, -1.5..=1.5).text("right").suffix(" m"));
            ui.add(egui::Slider::new(&mut layout.plate_shift_up, -1.5..=1.5).text("up").suffix(" m"));
            ui.separator();

            ui.horizontal(|ui| {
                if ui.button("Reset").clicked() {
                    layout.reset_values();
                }
                if ui.button("Copy values").clicked() {
                    copied = Some(format!(
                        "width: {:.3}\nheight: {:.3}\nscale: {:.2}\ncenter: ({:.0}, {:.0}) px\nglass fill: {:.2}\nplate distance: {:.2} m\nplate shift: right {:.3}, up {:.3} m",
                        layout.width, layout.height, layout.scale, layout.center_x, layout.center_y,
                        layout.glass_fill, layout.plate_distance, layout.plate_shift_right, layout.plate_shift_up,
                    ));
                }
            });
        });
    layout.open = open;
    layout.typing = open && ctx.wants_keyboard_input();
    copied
}

/// The center HUD's layout, drawn on the combiner glass itself (the jet's
/// frame). Copied into the line closure so the glass outline stays free
/// for the labels.
#[derive(Clone, Copy)]
pub(super) struct GlassMap {
    pub(super) aircraft_pos: Vector3<f32>,
    pub(super) aircraft_rot: UnitQuaternion<f32>,
    /// The eye pitch and heading are measured from, in the jet's frame: on
    /// the glass (F6) the resting cockpit eye, so head bobbing doesn't slide
    /// the marks on the pane; on the collimated plate the eye this frame,
    /// which the plate moves with (see `fixed_hud_plane`).
    pub(super) eye: Vector3<f32>,
    pub(super) center: Vector3<f32>,
    /// Glass plane, in the jet's frame: through `center`, facing the eye.
    pub(super) boresight: Vector3<f32>,
    pub(super) right: Vector3<f32>,
    pub(super) up: Vector3<f32>,
    pub(super) r_min: f32,
    pub(super) r_max: f32,
    pub(super) u_min: f32,
    pub(super) u_max: f32,
    pub(super) left: f32,
    pub(super) right_px: f32,
    pub(super) top: f32,
    pub(super) bottom: f32,
}

impl GlassMap {
    /// Where HUD pixel `(x, y)` sits in the world, on the glass.
    fn at(&self, x: f32, y: f32) -> Vector3<f32> {
        let u = (x - self.left) / (self.right_px - self.left);
        let v = (y - self.top) / (self.bottom - self.top);
        let along_right = self.r_min + (self.r_max - self.r_min) * u;
        let along_up = self.u_max + (self.u_min - self.u_max) * v;
        self.on_glass(self.center + self.right * along_right + self.up * along_up)
    }

    /// Meters on the glass of one HUD pixel across it.
    fn meters_per_pixel(&self) -> f32 {
        let glass_width = (self.r_max - self.r_min).max(1e-3);
        let hud_width = (self.right_px - self.left).max(1.0);
        glass_width / hud_width
    }

    /// Text centered on `center` (in the world, on the glass), lying on the
    /// glass, `font_size` HUD pixels tall - so it grows and shrinks with
    /// the glass on screen, like the marks drawn on it.
    fn text_frame(&self, center: Vector3<f32>, font_size: f32) -> TextFrame {
        let meters_per_pixel_up =
            (self.u_max - self.u_min).max(1e-3) / (self.bottom - self.top).max(1.0);
        TextFrame {
            center,
            right: self.aircraft_rot * self.right * (font_size * self.meters_per_pixel()),
            down: -(self.aircraft_rot * self.up) * (font_size * meters_per_pixel_up),
        }
    }

    /// The design eye, in the world.
    fn eye_world(&self) -> Vector3<f32> {
        self.aircraft_pos + self.aircraft_rot * self.eye
    }

    fn on_glass(&self, local: Vector3<f32>) -> Vector3<f32> {
        self.aircraft_pos + self.aircraft_rot * local
    }

    fn plane_normal(&self) -> Vector3<f32> {
        self.aircraft_rot * self.boresight
    }

    /// Where the ray from the design eye along `dir_world` meets the glass.
    fn ray_to_glass(&self, dir_world: Vector3<f32>) -> Option<Vector3<f32>> {
        self.ray_from(self.eye_world(), dir_world)
    }

    /// Where the ray from `eye` (in the world) along `dir_world` meets the
    /// glass.
    fn ray_from(&self, eye: Vector3<f32>, dir_world: Vector3<f32>) -> Option<Vector3<f32>> {
        let normal = self.plane_normal();
        let denom = normal.dot(&dir_world);
        if denom.abs() < 1e-5 {
            return None;
        }
        let t = normal.dot(&(self.on_glass(self.center) - eye)) / denom;
        if t < 1e-3 {
            return None;
        }
        Some(eye + dir_world * t)
    }

    /// The nearest point of the glass plane to `world`.
    fn cling(&self, world: Vector3<f32>) -> Vector3<f32> {
        let origin = self.on_glass(self.center);
        let normal = self.plane_normal();
        world - normal * (world - origin).dot(&normal)
    }

    /// `world` on this plate, as metres right and up from the center.
    fn plane_coords(&self, world: Vector3<f32>) -> (f32, f32) {
        let local = self.aircraft_rot.inverse() * (world - self.aircraft_pos);
        let delta = local - self.center;
        (delta.dot(&self.right), delta.dot(&self.up))
    }

    fn from_plane(&self, x: f32, y: f32) -> Vector3<f32> {
        self.on_glass(self.center + self.right * x + self.up * y)
    }

    /// The part of the segment that lies inside the HUD square. None when it misses.
    fn clip_to_frame(
        &self,
        start: Vector3<f32>,
        end: Vector3<f32>,
    ) -> Option<(Vector3<f32>, Vector3<f32>)> {
        let (x0, y0) = self.plane_coords(start);
        let (x1, y1) = self.plane_coords(end);
        let (x0, y0, x1, y1) = clip_segment_rect(
            x0, y0, x1, y1, self.r_min, self.r_max, self.u_min, self.u_max,
        )?;
        Some((self.from_plane(x0, y0), self.from_plane(x1, y1)))
    }

    /// Where the eye's line along `dir_world` crosses the plate, held inside
    /// the HUD square `margin` metres in from its edge: past the edge (or
    /// pointing away from the plate altogether), it's brought in toward the
    /// square's middle along the same way across it - so it stops at the
    /// edge on the side it went off.
    fn ray_held_in_frame(&self, dir_world: Vector3<f32>, margin: f32) -> Vector3<f32> {
        let (x, y, _) = self.held_in_frame(dir_world, margin);
        self.from_plane(x, y)
    }

    /// `ray_held_in_frame` on the plate (metres right and up from its
    /// center), and which way across the plate the line went from the
    /// square's middle (a unit vector, right and up) - straight up when it
    /// goes through the middle.
    fn held_in_frame(
        &self,
        dir_world: Vector3<f32>,
        margin: f32,
    ) -> (f32, f32, nalgebra::Vector2<f32>) {
        let (x, y) = match self.ray_to_glass(dir_world) {
            Some(hit) => self.plane_coords(hit),
            None => {
                // Away from the plate: only which way across it counts.
                let local = self.aircraft_rot.inverse() * dir_world;
                (local.dot(&self.right) * 1e6, local.dot(&self.up) * 1e6)
            }
        };
        let (middle_x, middle_y) = (
            (self.r_min + self.r_max) * 0.5,
            (self.u_min + self.u_max) * 0.5,
        );
        let half_x = ((self.r_max - self.r_min) * 0.5 - margin).max(0.0);
        let half_y = ((self.u_max - self.u_min) * 0.5 - margin).max(0.0);
        let (dx, dy) = (x - middle_x, y - middle_y);
        let shrink_x = if dx.abs() > half_x {
            half_x / dx.abs()
        } else {
            1.0
        };
        let shrink_y = if dy.abs() > half_y {
            half_y / dy.abs()
        } else {
            1.0
        };
        let shrink = shrink_x.min(shrink_y);
        let way = nalgebra::Vector2::new(dx, dy)
            .try_normalize(1e-9)
            .unwrap_or_else(|| nalgebra::Vector2::new(0.0, 1.0));
        (middle_x + dx * shrink, middle_y + dy * shrink, way)
    }

    /// Whether `world` sits inside the HUD square, `margin` metres in from the edge.
    fn inside_frame(&self, world: Vector3<f32>, margin: f32) -> bool {
        let (x, y) = self.plane_coords(world);
        x >= self.r_min + margin
            && x <= self.r_max - margin
            && y >= self.u_min + margin
            && y <= self.u_max - margin
    }
}

/// Liang–Barsky clip of a segment to an axis-aligned rectangle.
pub(super) fn clip_segment_rect(
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    min_x: f32,
    max_x: f32,
    min_y: f32,
    max_y: f32,
) -> Option<(f32, f32, f32, f32)> {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let mut t0 = 0.0;
    let mut t1 = 1.0;
    let mut clip = |p: f32, q: f32| -> bool {
        if p.abs() < 1e-8 {
            return q >= 0.0;
        }
        let t = q / p;
        if p < 0.0 {
            if t > t1 {
                return false;
            }
            if t > t0 {
                t0 = t;
            }
        } else {
            if t < t0 {
                return false;
            }
            if t < t1 {
                t1 = t;
            }
        }
        true
    };
    if !clip(-dx, x0 - min_x)
        || !clip(dx, max_x - x0)
        || !clip(-dy, y0 - min_y)
        || !clip(dy, max_y - y0)
    {
        return None;
    }
    Some((x0 + t0 * dx, y0 + t0 * dy, x0 + t1 * dx, y0 + t1 * dy))
}

/// The jet's combiner glass this frame: the far-plane map, and the glass's
/// outline on screen (the clip). `visible` is false when the glass isn't in
/// front of the camera - the center HUD is then not drawn.
pub(super) struct CollimatedHud {
    pub(super) map: GlassMap,
    pub(super) visible: bool,
    pub(super) gpu_mask: Option<Vec<[f32; 3]>>,
    /// Line widths, relative to the screen layout. Tracks how big the glass
    /// is on screen, so zooming in grows the strokes with the pane (the text
    /// lies on the glass itself - see `GlassMap::text_frame`).
    pub(super) print_scale: f32,
}

/// Where `world` lands in UI pixels. Unlike `world_to_screen`, a point in
/// front of the camera is returned even when it falls outside the screen,
/// so the glass outline stays intact when it crosses the edge.
pub(super) fn project_hud_point(
    active: &pankarta_software::rendering::camera::handler::CameraInstance,
    world: Vector3<f32>,
    screen_width: f32,
    screen_height: f32,
) -> Option<(f32, f32)> {
    let camera_to_point = world - active.camera.position().coords;
    let forward = active.camera.rotation_modifier * active.camera.calc_forward_direction();
    if camera_to_point.dot(&forward) <= 1e-3 {
        return None;
    }
    // The uploaded view_proj is still the previous frame's here. This is the
    // matrix the upcoming draw will use, so the clip matches the glass.
    let clip = (active.projection.calc_matrix() * active.camera.calc_matrix())
        * Point3::from(camera_to_point).to_homogeneous();
    if clip.w <= 1e-4 {
        return None;
    }
    let ndc = clip.xyz() / clip.w;
    // A point grazing the camera divides out to a huge screen position.
    // That was being read as the glass's on-screen size, and the labels'
    // font followed it until the text atlas overflowed (AtlasFull).
    if !ndc.x.is_finite() || !ndc.y.is_finite() || ndc.x.abs() > 8.0 || ndc.y.abs() > 8.0 {
        return None;
    }
    let x = (ndc.x + 1.0) * 0.5 * screen_width.max(1.0);
    let y = (1.0 - (ndc.y + 1.0) * 0.5) * screen_height.max(1.0);
    Some((x, y))
}

/// How big the plate is on screen against the layout it was drawn for - its
/// height, top to bottom through its middle, so rolling the head doesn't
/// change it. Zooming in grows it; the line widths follow (the text is on the
/// plate itself - see `GlassMap::text_frame`).
pub(super) fn plate_print_scale(
    active: &pankarta_software::rendering::camera::handler::CameraInstance,
    map: &GlassMap,
    screen_width: f32,
    screen_height: f32,
) -> f32 {
    let middle = (map.left + map.right_px) * 0.5;
    let Some((x0, y0)) =
        project_hud_point(active, map.at(middle, map.top), screen_width, screen_height)
    else {
        return 1.0;
    };
    let Some((x1, y1)) = project_hud_point(
        active,
        map.at(middle, map.bottom),
        screen_width,
        screen_height,
    ) else {
        return 1.0;
    };
    let plate_span = ((x1 - x0).powi(2) + (y1 - y0).powi(2)).sqrt();
    (plate_span / (map.bottom - map.top).max(1.0)).clamp(0.1, 10.0)
}

/// How far out the collimated plate is drawn (m) - far enough that a head
/// moving a few cm shifts it by nothing you could see (5 cm: 0.003°), near
/// enough to stay well inside the camera's far plane.
pub(super) const COLLIMATED_DISTANCE: f32 = 1000.0;

/// The HUD as a real one shows it: collimated, its image at optical
/// infinity. It turns with the jet (aimed along the resting cockpit view)
/// but sits out at COLLIMATED_DISTANCE from the eye wherever the eye is, so
/// moving the head doesn't slide it against the world - the flight path
/// marker stays on the flight path, the horizon line on the horizon. What
/// the head does move is the glass it's seen through (update_hud's
/// `plate_mask`). Rolling the camera leaves it where it is too. The F11
/// layout keeps its meaning: the plate covers the same angles a plate
/// `plate_distance` in front of the eye, shifted by `plate_shift_*`, would.
/// Layout pixels `(left, top)`..`(right, bottom)` cover the plate.
pub(super) fn fixed_hud_plane(
    scene: &Scene,
    player_id: &str,
    screen_width: f32,
    screen_height: f32,
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
    layout: &HudLayout,
) -> Option<GlassMap> {
    let instance = crate::ext::drawn::get(scene, player_id)?;
    let aircraft_pos = instance.instance.transform.position;
    let aircraft_rot = instance.instance.transform.rotation;
    // The eye this frame, in the jet's frame - head motion and all.
    let eye =
        aircraft_rot.inverse() * (scene.cameras.active().camera.position().coords - aircraft_pos);
    let view_pitch = GAME_SETTINGS.lock().unwrap().cockpit_view_pitch_deg;
    // Same sign as the cockpit camera: a positive view angle looks up, and
    // a positive pitch about X looks down.
    let rest = UnitQuaternion::from_axis_angle(&Vector3::x_axis(), (-view_pitch).to_radians());
    let boresight = (rest * Vector3::z()).try_normalize(1e-4)?;
    // The layout's plate, scaled out about the eye to COLLIMATED_DISTANCE -
    // every angle from the eye stays what the layout gives.
    let reference = layout.plate_distance.max(0.05);
    let out = COLLIMATED_DISTANCE / reference;
    let distance = COLLIMATED_DISTANCE;
    let up_raw = rest * Vector3::y();
    let up = (up_raw - boresight * up_raw.dot(&boresight)).try_normalize(1e-4)?;
    let right_axis = boresight.cross(&up);
    let center = eye
        + (boresight * reference
            + right_axis * layout.plate_shift_right
            + up * layout.plate_shift_up)
            * out;
    // The plate's size is the layout's angle at the resting field of view,
    // so it matches the on-screen HUD when the head is centered.
    let fovy = 70.0_f32.to_radians();
    let span_x = (right - left).max(1.0);
    let span_y = (bottom - top).max(1.0);
    let screen_height = screen_height.max(1.0);
    let screen_width = screen_width.max(1.0);
    let angle_y = fovy * span_y / screen_height;
    let angle_x = fovy * (screen_width / screen_height) * span_x / screen_width;
    let half_r = distance * (angle_x * 0.5).tan();
    let half_u = distance * (angle_y * 0.5).tan();
    Some(GlassMap {
        aircraft_pos,
        aircraft_rot,
        eye,
        center,
        boresight,
        right: right_axis,
        up,
        r_min: -half_r,
        r_max: half_r,
        u_min: -half_u,
        u_max: half_u,
        left,
        right_px: right,
        top,
        bottom,
    })
}

/// How far a jet turned `rotation` is banked against the horizon (degrees):
/// the angle its wings make with the level line square to its nose - +
/// right wing down, - left wing down, ±180 upside down. None with the nose
/// straight up or down (no horizon to measure against).
pub(super) fn bank_angle(rotation: &UnitQuaternion<f32>) -> Option<f32> {
    let nose = rotation * Vector3::z();
    let up = rotation * Vector3::y();
    // Level, square to the nose: toward the left wing (+X) and up.
    let level_left = Vector3::y().cross(&nose).try_normalize(1e-3)?;
    let level_up = nose.cross(&level_left);
    // Banking right tips the jet's up toward its right (-left).
    Some((-up.dot(&level_left)).atan2(up.dot(&level_up)).to_degrees())
}

/// How long the HUD takes to fade in after a cinematic (s).
pub(super) const HUD_BOOT_FADE: f32 = 0.32;

pub(super) fn hud_boot_length() -> f32 {
    HUD_BOOT_FADE
}

/// 0 hidden, 1 fully on. `t` is seconds since the cockpit HUD started
/// coming on: a short fade, then steady.
pub(super) fn hud_boot_alpha(t: f32) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    if t >= HUD_BOOT_FADE {
        return 1.0;
    }
    let u = t / HUD_BOOT_FADE;
    u * u * (3.0 - 2.0 * u)
}

/// A camera key's change of view - see Play::update_camera_focus.
/// The seeker's HUD symbol - see Play::draw_hud_symbol: where it points
/// in the world (or, `track`, the aircraft it's on - drawn where that's
/// rendered this frame), which way is up for it, how big (HUD px either way
/// from its middle), and a diamond (the seeker) or a box (a lock).
#[derive(Clone)]
pub(super) struct HudSymbol {
    pub(super) point: Vector3<f32>,
    pub(super) track: Option<String>,
    pub(super) up: Vector3<f32>,
    pub(super) half_size: f32,
    pub(super) diamond: bool,
}

/// The HUD's range scale with a lock (m) - see Play::update_hud.
pub(super) struct RangeCue {
    /// To the target.
    pub(super) range: f32,
    /// The missile's shortest and longest range at it, right now.
    pub(super) min_range: f32,
    pub(super) max_range: f32,
    /// The scale's top - its seeker's range.
    pub(super) scale_top: f32,
    pub(super) in_range: bool,
}

impl Play {
    /// The player's own cockpit, the view the HUD belongs to.
    pub(super) fn in_player_cockpit(&self, scene: &Scene) -> bool {
        self.camera.cockpit_on_screen(scene)
    }

    /// Starts the HUD power-on the frame a cinematic stops hiding it and
    /// the view is the cockpit. A shot that never hid the HUD leaves it
    /// alone. Opening the editor or the map, or another shot hiding it
    /// again, drops a boot that's still running.
    pub(super) fn sync_hud_boot(&mut self, scene: &Scene, app: &mut App, was_hiding_hud: bool) {
        if self.editor.editor_active
            || self.tactical.tactical_map_active
            || self.hud.animation_hides_hud
        {
            if self.hud.hud_boot.is_some() {
                self.hud.hud_boot = None;
                self.hud.hud_opacity = 1.0;
            }
            return;
        }
        if was_hiding_hud && self.in_player_cockpit(scene) {
            self.hud.hud_boot = Some(0.0);
            self.hud.hud_opacity = 0.0;
        }
    }

    /// Advances the power-on and writes the HUD's opacity. `advance` is
    /// false while paused, so the fade holds instead of running behind the
    /// menu. Once the fade is done the HUD stays fully on.
    pub(super) fn tick_hud_boot(&mut self, app: &mut App, advance: bool) {
        if self.pilot_view.hud_hidden_by_g {
            self.hud.hud_boot = None;
            self.hud.hud_opacity = 1.0;
            return;
        }
        let Some(t) = self.hud.hud_boot else {
            self.hud.hud_opacity = 1.0;
            return;
        };
        let t = if advance { t + app.time.delta_time } else { t };
        if t >= hud_boot_length() {
            self.hud.hud_boot = None;
            self.hud.hud_opacity = 1.0;
        } else {
            self.hud.hud_boot = Some(t);
            self.hud.hud_opacity = hud_boot_alpha(t);
        }
    }

    /// Keeps the flight HUD hidden while update_g_vision says so - every
    /// frame, after the level's UI tracks (EventSystem::apply_tracks) have
    /// run, since those set "game_ui" active every frame too. The quick
    /// menu hides itself (see Plane::update_quick_menu).
    pub(super) fn hide_hud(&self, scene: &mut Scene, app: &mut App) {
        if !self.pilot_view.hud_hidden_by_g {
            return;
        }
        for hud in Self::FLIGHT_HUD {
            if let Some(node) = scene.ui_node(hud) {
                if node.is_active {
                    node.set_active(false);
                }
            }
        }
    }

    /// The flight HUD, laid out in a box in the middle of the screen - the
    /// same in the cockpit or out of it:
    /// - a vertical line either side, its ends turned in toward the middle;
    ///   the speed in a box outside the left one, the altitude in a box
    ///   outside the right one - each box against the HUD's edge;
    /// - between the lines, the horizon line and a shorter line every
    ///   LADDER_STEP_DEG above and below it, laid out toward where the jet
    ///   points (they tilt and slide as it banks and pitches, and are only
    ///   seen looking forward) - cut off at the lines and their ends;
    /// - with a lock, the right line is the missile's range scale: ticks at
    ///   its shortest and longest range, an arrow at the target's range;
    /// - over the box, the heading tape.
    /// In the Fox-2 mode (a heat-seeker selected) the horizon and ladder
    /// are left out - the seeker's diamond has the middle - and the G shows
    /// bottom left.
    pub(super) fn update_hud(&self, scene: &mut Scene, app: &mut App) {
        const COLOR: [f32; 4] = [0.0, 1.0, 0.3, 0.9];
        const WIDTH: f32 = 1.5;
        const LADDER_STEP_DEG: i32 = 5;
        // Below the horizon the ladder's lines are dashed: each dash and the
        // space after it (px).
        const DASH: f32 = 5.0;
        const DASH_GAP: f32 = 4.0;
        // Between a ladder line's outer end and the middle of its number (px).
        const LABEL_OFFSET: f32 = 12.0;
        // How far they're laid out (m) - far enough to read as the sky.
        const DISTANCE: f32 = 10_000.0;
        // Between a readout's box and its side line (px).
        const READOUT_GAP: f32 = 6.0;
        // How far the side lines' ends turn in, square to them (px).
        const END_IN: f32 = 10.0;
        // The range scale's ticks and arrow (px).
        const TICK: f32 = 8.0;
        const ARROW: f32 = 7.0;
        // The heading tape over the box: degrees either side of the middle
        // it shows (across the horizon's width), how far over the box its
        // bottom is, its ticks every 10° and 5° (standing that much over its
        // bottom), and the box round the heading in its middle - the tape's
        // own numbers left out where they'd run into it (px).
        const HEADING_SPAN_DEG: f32 = 30.0;
        const HEADING_GAP: f32 = 4.0;
        const HEADING_TICKS_UP: f32 = 6.0;
        const HEADING_MAJOR_TICK: f32 = 8.0;
        const HEADING_MINOR_TICK: f32 = 4.0;
        const HEADING_BOX: (f32, f32) = (42.0, 22.0);
        // The ladder's and the heading tape's numbers (px).
        const SMALL_LABEL_FONT: f32 = 10.0;
        // The velocity marker's "o" on the plate (px) - the screen one's.
        const VELOCITY_MARKER_FONT: f32 = 14.0;

        app.renderer
            .render_sets
            .lines("hud_horizon_lines")
            .dynamic
            .clear();
        app.renderer
            .render_sets
            .lines("hud_screen_lines")
            .dynamic
            .clear();
        app.renderer.render_sets.lines("hud_lines").dynamic.clear();
        app.renderer.render_sets.text("hud_text").clear();
        app.renderer.render_sets.text("hud_text").mask = None;
        app.renderer.render_sets.text("hmd_text").clear();
        self.weapons.hmd_cue.set(None);

        let (screen_width, screen_height) =
            (app.window.size.width as f32, app.window.size.height as f32);
        // The HUD's size (fractions of the screen), its center, and its scale,
        // from F11's layout panel. Scale shrinks the frame and every mark
        // around that center, letters included.
        let layout = self.hud.hud_layout;
        let scale = layout.draw_scale();
        let (base_width, base_height) =
            (screen_width * layout.width, screen_height * layout.height);
        let (center_x, center_y) = (
            screen_width * 0.5 + layout.center_x,
            screen_height * 0.5 + layout.center_y,
        );
        let (frame_width, frame_height) = (base_width * scale, base_height * scale);
        let (frame_x, frame_y) = (center_x - frame_width * 0.5, center_y - frame_height * 0.5);
        let (readout_width, readout_height) = (
            play_ui::HUD_READOUT_SIZE.0 * scale,
            play_ui::HUD_READOUT_SIZE.1 * scale,
        );
        let readout_gap = READOUT_GAP * scale;
        let end_in = END_IN * scale;
        let line_w = WIDTH * scale;
        let dash = DASH * scale;
        let dash_gap = DASH_GAP * scale;
        let label_offset = LABEL_OFFSET * scale;
        let tick_len = TICK * scale;
        let arrow = ARROW * scale;
        let heading_gap = HEADING_GAP * scale;
        let heading_ticks_up = HEADING_TICKS_UP * scale;
        let heading_major = HEADING_MAJOR_TICK * scale;
        let heading_minor = HEADING_MINOR_TICK * scale;
        let heading_box = (HEADING_BOX.0 * scale, HEADING_BOX.1 * scale);
        let small_label = (
            play_ui::HUD_SMALL_LABEL_SIZE.0 * scale,
            play_ui::HUD_SMALL_LABEL_SIZE.1 * scale,
        );
        let left_line = frame_x + readout_width + readout_gap;
        let right_line = frame_x + frame_width - readout_width - readout_gap;
        let line_top = frame_y;
        let line_bottom = frame_y + frame_height;

        let game_ui_shown = scene.ui_node("hud/game_ui").is_some_and(|node| node.is_active);
        // Only from the cockpit of the player's own jet - it's the pilot's
        // HUD, not the camera's.
        let in_cockpit = self.in_own_cockpit(scene);
        let shown = game_ui_shown && !self.pilot_view.hud_hidden_by_g && in_cockpit;
        let hud_fixed = self.hud_on_plate(scene);
        // The HUD's mode, from the weapon selected: Fox-2 (a heat-seeking
        // missile) drops the horizon and ladder - the seeker's diamond has
        // the middle - and shows the G, bottom left. Otherwise the normal
        // HUD.
        let fox2 = self
            .weapons
            .selected_weapon
            .as_ref()
            .and_then(|weapon| self.weapons.weapon_specs.get(weapon))
            .is_some_and(|spec| {
                spec.category == WeaponCategory::Missile && spec.guidance.seeker == Seeker::InfraRed
            });
        let g = scene
            .content
            .nodes
            .get(self.flight_manager.player_id())
            .and_then(|node| node.physics_state::<AircraftState>())
            .map(|state| state.flight_data.g_meter)
            .unwrap_or(1.0);
        if let Some(label) = scene.ui_node("hud/hud_g").and_then(|node| node.as_label_mut()) {
            label.set_text(&format!("{g:.1}G"));
        }
        // Speed on the left, altitude on the right. No captions.
        let speed_x = frame_x + readout_width * 0.5;
        let altitude_x = frame_x + frame_width - readout_width * 0.5;
        let middle_y = frame_y + frame_height * 0.5;
        // The heading tape, over the box: its bottom, its ticks' bottom, its
        // numbers' middle - and the heading's, a little higher, its digits
        // being bigger.
        let middle_x = (left_line + right_line) * 0.5;
        let tape_bottom = line_top - heading_gap;
        let ticks_bottom = tape_bottom - heading_ticks_up;
        let label_y = ticks_bottom - heading_major - 2.0 * scale - small_label.1 * 0.5;
        let heading_y = label_y - 4.0 * scale;
        let g_y = line_bottom - readout_height * 0.5;
        let weapon_y = line_bottom;
        // Radar altitude sits just outside the right line's bottom corner.
        // The fuel bar is a gauge just outside the left line's.
        // Center of the square's bottom half.
        let warning_y = (line_top + line_bottom * 3.0) * 0.25;
        let corner_width = play_ui::HUD_RADAR_ALTITUDE_WIDTH * scale;
        let corner_x = right_line + 4.0 * scale + corner_width * 0.5;
        // Its bottom edge is the line's bottom.
        let radar_y = line_bottom - readout_height * 0.5;
        let cms_y = line_bottom - 40.0 * scale;
        let player_id = self.flight_manager.player_id();
        let (fuel_kg, fuel_capacity, bingo_kg, radar_warn_m) = scene
            .content
            .nodes
            .get(player_id)
            .and_then(|node| node.get_behavior::<Plane>())
            .map(|plane| {
                let (kg, capacity) = plane.fuel();
                (kg, capacity, plane.bingo_kg, plane.radar_warn_m)
            })
            .unwrap_or((0.0, 0.0, None, self::map::DEFAULT_RADAR_WARN_M));
        let radar_altitude = crate::ext::drawn::get(scene, player_id)
            .map(|instance| {
                self.world_collision.height_below(
                    instance.instance.transform.position,
                    app.particles.sea_level().unwrap_or(0.0),
                )
            })
            .unwrap_or(0.0);
        let low_altitude = shown && radar_altitude < radar_warn_m;
        let bingo_text = self
            .weapons
            .bingo_warn_until
            .is_some_and(|until| self.game_time < until);
        // The cue after a countermeasure setting change: a few fast blinks,
        // a short hold, then a fade.
        let cms_cue = self.weapons.cms_cue.as_ref().and_then(|(text, started)| {
            let age = self.game_time - *started;
            const BLINK_END: f64 = 0.5;
            const HOLD_END: f64 = 1.7;
            const FADE_END: f64 = 2.4;
            if !(0.0..FADE_END).contains(&age) {
                return None;
            }
            let (on, alpha) = if age < BLINK_END {
                ((age * 8.0).fract() < 0.5, 1.0)
            } else if age < HOLD_END {
                (true, 1.0)
            } else {
                (
                    true,
                    (1.0 - ((age - HOLD_END) / (FADE_END - HOLD_END)) as f32).clamp(0.0, 1.0),
                )
            };
            Some((text.clone(), on, alpha))
        });
        // About twice a second. Low radar altitude blinks WARN and the
        // radar figure. For the four seconds after bingo, BINGO and the
        // fuel figure blink too, and the fuel figure does not blink again.
        const ALTITUDE_BLINK_SECONDS: f64 = 0.5;
        let blink_on = (self.game_time % ALTITUDE_BLINK_SECONDS) < ALTITUDE_BLINK_SECONDS * 0.5;
        let radar_red = low_altitude && blink_on;
        let fuel_red = bingo_text && blink_on;
        let warning_shown = shown && blink_on && (bingo_text || low_altitude);
        let (warn_width, warn_height) = (
            play_ui::HUD_WARN_SIZE.0 * scale,
            play_ui::HUD_WARN_SIZE.1 * scale,
        );
        // Fuel bar, just outside the left line, bottom-aligned with it.
        // Bottom is empty, top is the tanks' maximum. The percent sits
        // against the bar's left side. The red mark is bingo, the same
        // width as the end caps.
        let fuel_bar_width = 9.0 * scale;
        let fuel_bar_height = 60.0 * scale;
        let fuel_percent_width = play_ui::HUD_FUEL_PERCENT_WIDTH * scale;
        let fuel_bar_bottom = line_bottom;
        let fuel_bar_top = fuel_bar_bottom - fuel_bar_height;
        let fuel_bar_right = left_line - 6.0 * scale;
        let fuel_bar_left = fuel_bar_right - fuel_bar_width;
        let fuel_fraction = if fuel_capacity > 0.0 {
            (fuel_kg / fuel_capacity).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let fuel_level_y = fuel_bar_bottom - fuel_fraction * fuel_bar_height;
        let fuel_percent_x = fuel_bar_left - 2.0 * scale - fuel_percent_width * 0.5;
        let fuel_percent_y = (fuel_bar_top + fuel_bar_bottom) * 0.5;
        let bingo_fraction = bingo_kg
            .filter(|kg| *kg > 0.0 && fuel_capacity > 0.0)
            .map(|kg| (kg / fuel_capacity).clamp(0.0, 1.0));
        let fixed_map = if hud_fixed {
            fixed_hud_plane(
                scene,
                player_id,
                screen_width,
                screen_height,
                left_line,
                right_line,
                line_top,
                line_bottom,
                &layout,
            )
        } else {
            None
        };
        // How big the plate is on screen - its line widths follow.
        let plate_print = fixed_map.as_ref().map_or(1.0, |map| {
            plate_print_scale(scene.cameras.active(), map, screen_width, screen_height)
        });
        // Everything on the plate only shows through the jet's HUD glass -
        // its outline on screen, in real pixels. The plate is laid out as
        // ever; the glass just cuts it, so moving the head moves the window,
        // not the symbols. With the glass out of view nothing shows; a jet
        // without a glass outline isn't cut.
        let glass = self
            .flight_manager
            .hud_glass(player_id)
            .filter(|glass| !glass.is_empty());
        let to_pixels = app.window.pixel_size.height as f32 / screen_height.max(1.0);
        // World points on screen, in real pixels - None if any is behind the
        // camera.
        let project_all =
            |points: &mut dyn Iterator<Item = Vector3<f32>>| -> Option<Vec<[f32; 2]>> {
                points
                    .map(|point| {
                        project_hud_point(
                            scene.cameras.active(),
                            point,
                            screen_width,
                            screen_height,
                        )
                        .map(|(x, y)| [x * to_pixels, y * to_pixels])
                    })
                    .collect()
            };
        // The glass on screen, in real pixels - None while any of its corners
        // is behind the camera (then it's out of view).
        let glass_on_screen = fixed_map
            .as_ref()
            .zip(glass)
            .map(|(map, glass)| project_all(&mut glass.iter().map(|point| map.on_glass(*point))));
        let plate_mask = glass_on_screen.as_ref().map(|points| {
            points
                .as_ref()
                .and_then(|points| convex_mask(points, 0.0))
                .unwrap_or_else(|| vec![[0.0, 0.0, -1.0]])
        });
        app.renderer.render_sets.lines("hud_lines").mask = plate_mask.clone();
        app.renderer.render_sets.text("hud_text").mask = plate_mask.clone();
        // The seeker's symbol - from any view, on the plate while it's up.
        if let Some(symbol) = self.weapons.hud_symbol.clone() {
            self.draw_hud_symbol(
                scene,
                app,
                symbol,
                fixed_map.as_ref().map(|map| (map, plate_print)),
            );
        }
        // The vertical lock's line: dashed, straight up through the nose,
        // laid out far ahead so it sits on the sky like the HUD's other marks.
        // On the plate it runs the whole height of the glass (the glass cuts
        // it); on the screen, just the strip it locks in (see
        // fire_control::VERTICAL_SCAN_*). The strip is the same either way.
        if let (true, Some(instance)) = (
            self.weapons.vertical_scan,
            crate::ext::drawn::get(scene, player_id),
        ) {
            const DASH_DEG: f32 = 1.5;
            const DASH_GAP_DEG: f32 = 1.0;
            const DISTANCE: f32 = 3000.0;
            let (bottom, top) = if plate_mask.is_some() {
                (-80.0, 80.0)
            } else {
                (
                    -fire_control::VERTICAL_SCAN_DOWN_DEG,
                    fire_control::VERTICAL_SCAN_UP_DEG,
                )
            };
            let (position, rotation) = (
                instance.instance.transform.position,
                instance.instance.transform.rotation,
            );
            let at = |degrees: f32| {
                position
                    + rotation
                        * Vector3::new(0.0, degrees.to_radians().sin(), degrees.to_radians().cos())
                        * DISTANCE
            };
            let mut from = bottom;
            while from < top {
                let to = (from + DASH_DEG).min(top);
                app.renderer
                    .render_sets
                    .lines("hud_lines")
                    .dynamic
                    .push(LineInstance {
                        start: at(from).into(),
                        end: at(to).into(),
                        color: COLOR,
                        width: 1.5 * scale * plate_print,
                    });
                from += DASH_DEG + DASH_GAP_DEG;
            }
        }
        // The velocity marker: on the plate, where the jet's path crosses it
        // seen from the eye (the screen one is hidden meanwhile - see
        // ui_control); held at the HUD square's edge once the path is past
        // it, on the side it went off. The path is the physics' velocity in
        // the jet's frame, turned by the jet as it's drawn this frame - the
        // raw velocity is up to two physics steps old while the drawn jet
        // (and the plate on it) is moved on to now, so it would jitter
        // against the plate at the physics' rate.
        let marker_shown = scene
            .ui_node("hud/velocity_marker")
            .is_some_and(|node| node.is_active);
        if let (Some(map), Some(body_velocity), true) = (
            &fixed_map,
            self.hud.player_body_velocity,
            shown && marker_shown,
        ) {
            if let Some(direction) = (map.aircraft_rot * body_velocity).try_normalize(0.1) {
                let at = map.ray_held_in_frame(
                    direction,
                    VELOCITY_MARKER_FONT * 0.5 * scale * map.meters_per_pixel(),
                );
                let color = UiColor::Rgb(0, 255, 77).to_rgba_f32();
                app.renderer.render_sets.text("hud_text").push(
                    "o",
                    HUD_FONT,
                    map.text_frame(at, VELOCITY_MARKER_FONT * scale),
                    color,
                );
            }
        }
        // The helmet display, looking away from the HUD.
        if let (Some(map), true) = (&fixed_map, shown && self.hud.hmd_enabled) {
            self.draw_hmd(scene, app, map, screen_width, screen_height);
        }
        // Same angle for every readout: the plate's, about each label's own
        // center. None leaves the letters upright with the head.
        let placed = [
            (
                "hud_speed",
                speed_x,
                middle_y,
                shown,
                readout_width,
                readout_height,
            ),
            (
                "hud_altitude",
                altitude_x,
                middle_y,
                shown,
                readout_width,
                readout_height,
            ),
            (
                "hud_heading",
                middle_x,
                heading_y,
                shown,
                readout_width,
                readout_height,
            ),
            (
                "hud_g",
                speed_x,
                g_y,
                shown && fox2,
                readout_width,
                readout_height,
            ),
            (
                "hud_weapon",
                middle_x,
                weapon_y,
                shown && self.weapons.selected_weapon.is_some(),
                160.0 * scale,
                readout_height,
            ),
            (
                "hud_altitude_warning",
                middle_x,
                warning_y,
                warning_shown,
                warn_width,
                warn_height,
            ),
            (
                "hud_corner_fuel",
                fuel_percent_x,
                fuel_percent_y,
                shown && fuel_capacity > 0.0,
                fuel_percent_width,
                readout_height,
            ),
            (
                "hud_radar_altitude",
                corner_x,
                radar_y,
                shown,
                corner_width,
                readout_height,
            ),
            (
                "hud_cms_cue",
                middle_x,
                cms_y,
                shown && cms_cue.as_ref().is_some_and(|(_, on, _)| *on),
                160.0 * scale,
                readout_height,
            ),
        ];
        for (key, x, y, visible, width, height) in placed {
            let Some(node) = scene.ui_node(&play_ui::in_hud(key)) else {
                continue;
            };
            // On the plate the readout is drawn on it (below), not as the
            // screen label.
            node.set_active(visible && fixed_map.is_none());
            node.style.font_size = Some(hud_readout_font(key) * scale);
            if key == "hud_radar_altitude" {
                node.style.text_color = Some(if radar_red {
                    UiColor::Rgb(255, 64, 64)
                } else {
                    UiColor::Rgb(0, 255, 77)
                });
                if let Some(label) = node.as_label_mut() {
                    label.set_text(&format!("R: {radar_altitude:.0}"));
                }
            }
            if key == "hud_corner_fuel" {
                node.style.text_color = Some(if fuel_red {
                    UiColor::Rgb(255, 64, 64)
                } else {
                    UiColor::Rgb(0, 255, 77)
                });
                if let Some(label) = node.as_label_mut() {
                    label.set_text(&format!("{:.0}%", fuel_fraction * 100.0));
                }
            }
            if key == "hud_weapon" {
                if let Some(label) = node.as_label_mut() {
                    let name = self
                        .weapons
                        .selected_weapon
                        .as_deref()
                        .map(weapon_label)
                        .unwrap_or_default();
                    label.set_text(&name);
                }
            }
            if key == "hud_altitude_warning" {
                if let Some(label) = node.as_label_mut() {
                    label.set_text(if bingo_text { "BINGO" } else { "WARN" });
                }
            }
            if key == "hud_cms_cue" {
                if let Some((text, _, alpha)) = &cms_cue {
                    node.style.text_color = Some(UiColor::Rgba(0, 255, 77, (alpha * 255.0) as u8));
                    if let Some(label) = node.as_label_mut() {
                        label.set_text(text);
                    }
                }
            }
            ui_layers::place(node, x - width / 2.0, y - height / 2.0, width, height);
            // On the plate: lying on it at the readout's own point, so it
            // turns, grows and shrinks with the plate.
            if let (Some(map), true) = (&fixed_map, visible) {
                let color = node
                    .style
                    .text_color
                    .unwrap_or(UiColor::Rgb(0, 255, 77))
                    .to_rgba_f32();
                let font_size = hud_readout_font(key) * scale;
                if let Some(label) = node.as_label_mut() {
                    app.renderer.render_sets.text("hud_text").push(
                        label.text(),
                        label.font(),
                        map.text_frame(map.at(x, y), font_size),
                        color,
                    );
                }
            }
        }
        for (key, visible) in [
            ("hud/game_ui/hud_stores", shown),
            (
                "hud/game_ui/hud_stores/weapons",
                shown && !self.weapons.weapon_list.is_empty(),
            ),
        ] {
            if let Some(node) = scene.ui_node(key) {
                node.set_active(visible);
            }
        }
        // The ladder's and the heading tape's numbers - hidden until placed
        // below.
        let pitch_keys = (0..play_ui::HUD_PITCH_LABELS).map(play_ui::hud_pitch_label_key);
        let heading_keys = (0..play_ui::HUD_HEADING_LABELS).map(play_ui::hud_heading_label_key);
        for key in pitch_keys.chain(heading_keys) {
            if let Some(node) = scene.ui_node(&play_ui::in_hud(&key)) {
                node.set_active(false);
            }
        }
        if !shown {
            return;
        }

        // The side lines and the range scale - in screen pixels (the UI's,
        // scaled to the screen's real ones).
        let to_pixels = app.window.pixel_size.height as f32 / screen_height.max(1.0);
        // On a pixel's middle, so the lines that meet line up exactly.
        let snap = |value: f32| (value * to_pixels).round() + 0.5;
        let line_color = std::cell::Cell::new(COLOR);
        // F6 draws the center on the combiner glass. The video toggle instead
        // puts the whole HUD on a plate fixed to the jet, camera pose aside.
        let collimated = if let Some(map) = fixed_map {
            Some(CollimatedHud {
                map,
                visible: true,
                gpu_mask: plate_mask.clone(),
                print_scale: plate_print,
            })
        } else {
            self.hud
                .collimated_hud
                .then(|| {
                    let instance = crate::ext::drawn::get(scene, player_id)?;
                    let glass = self.flight_manager.hud_glass(player_id)?;
                    let aircraft_pos = instance.instance.transform.position;
                    let aircraft_rot = instance.instance.transform.rotation;
                    let eye = instance
                        .instance
                        .metadata
                        .cameras
                        .as_ref()
                        .and_then(|cameras| cameras.get(COCKPIT_CAMERA))
                        .map(|camera| camera.position)
                        .unwrap_or_else(|| {
                            aircraft_rot.inverse()
                                * (scene.cameras.active().camera.position().coords - aircraft_pos)
                        });
                    if glass.len() < 3 {
                        return None;
                    }
                    let center = glass.iter().copied().sum::<Vector3<f32>>() / glass.len() as f32;
                    let boresight = (center - eye).try_normalize(1e-4)?;
                    let up = (Vector3::y() - boresight * Vector3::y().dot(&boresight))
                        .try_normalize(1e-4)?;
                    let right = boresight.cross(&up);
                    let (mut r_min, mut r_max) = (f32::MAX, f32::MIN);
                    let (mut u_min, mut u_max) = (f32::MAX, f32::MIN);
                    for point in glass {
                        let along_right = (point - center).dot(&right);
                        let along_up = (point - center).dot(&up);
                        r_min = r_min.min(along_right);
                        r_max = r_max.max(along_right);
                        u_min = u_min.min(along_up);
                        u_max = u_max.max(along_up);
                    }
                    if r_max - r_min < 1e-3 || u_max - u_min < 1e-3 {
                        return None;
                    }
                    // Larger than the pane, so the glass edge — not an inner margin —
                    // is what cuts the symbols off.
                    let grow = |min: f32, max: f32| {
                        let mid = (min + max) * 0.5;
                        let half = (max - min) * 0.5 * layout.glass_fill;
                        (mid - half, mid + half)
                    };
                    let (r_min, r_max) = grow(r_min, r_max);
                    let (u_min, u_max) = grow(u_min, u_max);
                    let top = label_y - small_label.1 * 0.5;
                    let map = GlassMap {
                        aircraft_pos,
                        aircraft_rot,
                        eye,
                        center,
                        boresight,
                        right,
                        up,
                        r_min,
                        r_max,
                        u_min,
                        u_max,
                        left: left_line,
                        right_px: right_line,
                        top,
                        bottom: line_bottom,
                    };
                    let active = scene.cameras.active();
                    let mut ui_points = Vec::new();
                    let mut gpu_points = Vec::new();
                    for point in glass {
                        let world = aircraft_pos + aircraft_rot * point;
                        let Some((x, y)) =
                            project_hud_point(active, world, screen_width, screen_height)
                        else {
                            continue;
                        };
                        ui_points.push([x, y]);
                        gpu_points.push([x * to_pixels, y * to_pixels]);
                    }
                    let gpu_mask = convex_mask(&gpu_points, 0.0);
                    let visible = gpu_mask.is_some();
                    // The glass's height on screen against the layout it was drawn
                    // for. Zoom changes both the pane and this, so the line widths
                    // stay the same on the glass. Clamped: looking away stretches
                    // the projection.
                    let print_scale = if ui_points.len() >= 2 {
                        let (mut min_y, mut max_y) = (f32::MAX, f32::MIN);
                        for point in &ui_points {
                            min_y = min_y.min(point[1]);
                            max_y = max_y.max(point[1]);
                        }
                        let glass_span = (max_y - min_y).max(1.0);
                        let layout_span = (line_bottom - top).max(1.0);
                        (glass_span / layout_span).clamp(0.25, 4.0)
                    } else {
                        1.0
                    };
                    Some(CollimatedHud {
                        map,
                        visible,
                        gpu_mask,
                        print_scale,
                    })
                })
                .flatten()
        };
        if let Some(hud) = &collimated {
            app.renderer.render_sets.lines("hud_horizon_lines").mask = hud.gpu_mask.clone();
            app.renderer.render_sets.text("hud_text").mask = hud.gpu_mask.clone();
        }
        let collimating = collimated.is_some();
        let map_for_lines = collimated
            .as_ref()
            .filter(|hud| hud.visible)
            .map(|hud| hud.map);
        let print_scale = collimated
            .as_ref()
            .map(|hud| hud.print_scale)
            .unwrap_or(1.0);
        // `on_glass`: this segment is part of the center. With F6 on it is
        // drawn on the combiner glass (and left out when the glass is behind
        // the camera) instead of stuck to the screen. With the plate on, every
        // mark — readouts included — is drawn on that plate.
        let mut screen_line = |start: (f32, f32), end: (f32, f32), width: f32, on_glass: bool| {
            if collimating && (on_glass || hud_fixed) {
                if let Some(map) = map_for_lines {
                    let start = map.at(start.0, start.1);
                    let end = map.at(end.0, end.1);
                    app.renderer
                        .render_sets
                        .lines("hud_horizon_lines")
                        .dynamic
                        .push(LineInstance {
                            start: start.into(),
                            end: end.into(),
                            color: line_color.get(),
                            width: width * print_scale,
                        });
                }
                return;
            }
            app.renderer
                .render_sets
                .lines("hud_screen_lines")
                .dynamic
                .push(LineInstance {
                    start: [snap(start.0), snap(start.1), 0.0],
                    end: [snap(end.0), snap(end.1), 0.0],
                    color: line_color.get(),
                    width,
                });
        };
        // The shader runs each line half its width past its ends, so the
        // ends meet the side line in a square corner.
        for (line, inward) in [(left_line, 1.0), (right_line, -1.0)] {
            screen_line((line, line_top), (line, line_bottom), line_w, true);
            for y in [line_top, line_bottom] {
                screen_line((line, y), (line + inward * end_in, y), line_w, true);
            }
        }
        // The readouts' boxes. The radar altitude under the altitude number
        // has none, and neither does WARN.
        {
            let mut readout_box = |x: f32, y: f32, width: f32| {
                let (left, right) = (x - width * 0.5, x + width * 0.5);
                let (top, bottom) = (y - readout_height * 0.5, y + readout_height * 0.5);
                screen_line((left, top), (right, top), line_w, false);
                screen_line((right, top), (right, bottom), line_w, false);
                screen_line((right, bottom), (left, bottom), line_w, false);
                screen_line((left, bottom), (left, top), line_w, false);
            };
            readout_box(speed_x, middle_y, readout_width);
            readout_box(altitude_x, middle_y, readout_width);
        }
        let alert = [1.0, 0.25, 0.25, 0.9];
        line_color.set(COLOR);
        // Fuel bar. The top and bottom caps, and the bingo mark, share one
        // horizontal span and one stroke so their ends line up.
        if fuel_capacity > 0.0 {
            let cap = 2.0 * scale;
            let mark_left = fuel_bar_left - cap;
            let mark_right = fuel_bar_right + cap;
            screen_line(
                (fuel_bar_left, fuel_bar_top),
                (fuel_bar_left, fuel_bar_bottom),
                line_w,
                false,
            );
            screen_line(
                (fuel_bar_right, fuel_bar_top),
                (fuel_bar_right, fuel_bar_bottom),
                line_w,
                false,
            );
            screen_line(
                (mark_left, fuel_bar_top),
                (mark_right, fuel_bar_top),
                line_w,
                false,
            );
            screen_line(
                (mark_left, fuel_bar_bottom),
                (mark_right, fuel_bar_bottom),
                line_w,
                false,
            );
            // The shader runs a line half its width past each end, so the
            // fill is shortened to stay inside the caps.
            let fill_width = fuel_bar_width - 3.0 * scale;
            let half = fill_width * 0.5;
            let y_bottom = fuel_bar_bottom - half;
            let y_top = fuel_level_y + half;
            if y_bottom - y_top > 0.5 {
                line_color.set(if fuel_red { alert } else { COLOR });
                let center_x = (fuel_bar_left + fuel_bar_right) * 0.5;
                screen_line((center_x, y_bottom), (center_x, y_top), fill_width, false);
            }
            if let Some(bingo_fraction) = bingo_fraction {
                let bingo_y = fuel_bar_bottom - bingo_fraction * fuel_bar_height;
                line_color.set([1.0, 0.25, 0.25, 1.0]);
                screen_line((mark_left, bingo_y), (mark_right, bingo_y), line_w, false);
            }
            line_color.set(COLOR);
        }
        if let Some(cue) = &self.weapons.range_cue {
            // Bottom of the line is 0 m, the top the seeker's range.
            let height_of = |range: f32| {
                line_bottom - (range / cue.scale_top).clamp(0.0, 1.0) * (line_bottom - line_top)
            };
            for range in [cue.min_range, cue.max_range] {
                let y = height_of(range);
                screen_line((right_line - tick_len, y), (right_line, y), line_w, true);
            }
            // The arrow, pointing at the line from inside - closed into a
            // triangle while the target's in range.
            let y = height_of(cue.range);
            let tip = (right_line - 2.0 * scale, y);
            let (upper, lower) = (
                (tip.0 - arrow * 1.5, y - arrow),
                (tip.0 - arrow * 1.5, y + arrow),
            );
            screen_line(upper, tip, line_w, true);
            screen_line(lower, tip, line_w, true);
            if cue.in_range {
                screen_line(upper, lower, line_w, true);
            }
        }

        let active = scene.cameras.active();
        let look = active.camera.rotation_modifier * active.camera.calc_forward_direction();
        // The tape and the ladder follow the camera, unless F6 has them
        // fixed to the jet's nose. Straight up or down, no heading.
        let camera_toward = Vector3::new(look.x, 0.0, look.z).try_normalize(1e-3);
        let jet_toward = crate::ext::drawn::get(scene, player_id).and_then(|instance| {
            let forward = instance.instance.transform.rotation * Vector3::z();
            Vector3::new(forward.x, 0.0, forward.z).try_normalize(1e-3)
        });
        // The tape and the ladder follow the head, unless the HUD is pinned
        // to the jet (or F6 has already put them on the glass).
        let toward = if collimated.is_some() || hud_fixed {
            jet_toward
        } else {
            camera_toward
        };
        let Some(toward) = toward else { return };
        // The space between the side lines in fifths: the horizon's two
        // halves take the 2nd and 4th, the gap between them the 3rd. The
        // other lines are half as long, around the same gap. On the glass
        // that inset is empty pane, so the marks run out to the frame.
        let fifth = (right_line - left_line) / 5.0;
        let gap = fifth * 0.5;
        let half_span = (right_line - left_line) * 0.5;
        let (horizon_half_length, ladder_half_length) = if collimated.is_some() {
            (half_span - end_in, half_span * 0.78)
        } else {
            (gap + fifth, gap + fifth * 0.5)
        };

        // The heading tape - where the camera looks, north +Z and east -X
        // (as the F10 map has them): a tick every 5°, longer every 10° with
        // its number (or N/E/S/W) over it - but not under the heading itself,
        // bigger in the middle.
        let per_degree = horizon_half_length / HEADING_SPAN_DEG;
        let heading = (-toward.x).atan2(toward.z).to_degrees().rem_euclid(360.0);
        if let Some(label) = scene
            .ui_node("hud/hud_heading")
            .and_then(|node| node.as_label_mut())
        {
            label.set_text(&((heading.round() as i32) % 360).to_string());
        }
        let mut heading_labels = Vec::new();
        let first = ((heading - HEADING_SPAN_DEG) / 5.0).ceil() as i32;
        let last = ((heading + HEADING_SPAN_DEG) / 5.0).floor() as i32;
        for mark in first..=last {
            let degrees = mark * 5;
            let x = middle_x + (degrees as f32 - heading) * per_degree;
            let major = degrees.rem_euclid(10) == 0;
            screen_line(
                (x, ticks_bottom),
                (
                    x,
                    ticks_bottom - if major { heading_major } else { heading_minor },
                ),
                line_w,
                true,
            );
            // Its number's box clear of the heading's.
            let clear = (x - middle_x).abs() >= (heading_box.0 + small_label.0) * 0.5;
            if major && clear {
                let text = match degrees.rem_euclid(360) {
                    0 => "N".to_owned(),
                    90 => "E".to_owned(),
                    180 => "S".to_owned(),
                    270 => "W".to_owned(),
                    other => other.to_string(),
                };
                heading_labels.push((x, label_y, text));
            }
        }
        // The heading's box.
        let (left, right) = (
            middle_x - heading_box.0 * 0.5,
            middle_x + heading_box.0 * 0.5,
        );
        let (top, bottom) = (
            heading_y - heading_box.1 * 0.5,
            heading_y + heading_box.1 * 0.5,
        );
        screen_line((left, top), (right, top), line_w, true);
        screen_line((right, top), (right, bottom), line_w, true);
        screen_line((right, bottom), (left, bottom), line_w, true);
        screen_line((left, bottom), (left, top), line_w, true);

        // The heading number rides the glass with the tape, cut off by the
        // glass's edge (`hud_text.mask`). On the plate it was already drawn
        // with the other readouts.
        if !hud_fixed {
            if let Some(hud) = &collimated {
                if let Some(node) = scene.ui_node("hud/hud_heading") {
                    node.set_active(false);
                    let color = node
                        .style
                        .text_color
                        .unwrap_or(UiColor::Rgb(0, 255, 77))
                        .to_rgba_f32();
                    if let (Some(label), true) = (node.as_label_mut(), hud.visible) {
                        let frame = hud.map.text_frame(
                            hud.map.at(middle_x, heading_y),
                            hud_readout_font("hud_heading") * scale,
                        );
                        app.renderer.render_sets.text("hud_text").push(
                            label.text(),
                            label.font(),
                            frame,
                            color,
                        );
                    }
                }
            } else if let Some(node) = scene.ui_node("hud/hud_heading") {
                node.style.font_size = Some(16.0 * scale);
            }
        }

        // The horizon and the ladder. On the screen they follow the camera
        // and stop at the side lines. With F6 they are the same angles,
        // carried onto the combiner glass, and the glass is what cuts them off.
        let ladder_box = [
            (left_line, line_top),
            (right_line, line_top),
            (right_line, line_bottom),
            (left_line, line_bottom),
        ]
        .map(|(x, y)| [x * to_pixels, y * to_pixels]);
        if collimated.is_none() {
            app.renderer.render_sets.lines("hud_horizon_lines").mask =
                convex_mask(&ladder_box, 2.0 * to_pixels);
        }
        let across = Vector3::y().cross(&toward);
        let active = scene.cameras.active();
        let eye = collimated
            .as_ref()
            .map(|hud| hud.map.eye_world())
            .unwrap_or_else(|| active.camera.position().coords);
        let meters_per_pixel = collimated
            .as_ref()
            .map(|hud| hud.map.meters_per_pixel())
            .unwrap_or_else(|| {
                2.0 * DISTANCE * (active.projection.fovy.to_radians() * 0.5).tan()
                    / screen_height.max(1.0)
            });
        // The numbers: on the screen (x, y), or with F6 or the plate, on the
        // glass in the world.
        let mut labels = Vec::new();
        let mut glass_labels: Vec<(Vector3<f32>, String)> = Vec::new();
        // None of it in the Fox-2 mode, nor while the glass is behind the camera.
        let glass_hidden = collimated.as_ref().is_some_and(|hud| !hud.visible);
        let steps = if fox2 || glass_hidden {
            1..=0
        } else {
            (-90 / LADDER_STEP_DEG)..=(90 / LADDER_STEP_DEG)
        };
        for step in steps {
            let degrees = step * LADDER_STEP_DEG;
            let pitch = (degrees as f32).to_radians();
            let half_length = if step == 0 {
                horizon_half_length
            } else {
                ladder_half_length
            };
            let dir = toward * pitch.cos() + Vector3::y() * pitch.sin();
            let center = if let Some(hud) = &collimated {
                let Some(hit) = hud.map.ray_to_glass(dir) else {
                    continue;
                };
                hit
            } else {
                eye + dir * DISTANCE
            };
            // In front of the camera. The mask cuts off whatever misses the
            // box - or, with F6, the glass. Off-screen still counts then, so
            // a line can enter the glass at the edge of the view.
            let ahead = (center - active.camera.position().coords).dot(&look) > 0.0;
            let in_view = collimated.is_some()
                || app
                    .renderer
                    .camera_resources
                    .world_to_screen(
                        active,
                        Point3::from(center),
                        screen_width as u32,
                        screen_height as u32,
                    )
                    .is_some();
            if !ahead || !in_view {
                continue;
            }
            // Solid, or below the horizon in dashes (px from the middle).
            let pieces: Vec<(f32, f32)> = if step < 0 {
                std::iter::successors(Some(gap), |from| Some(from + dash + dash_gap))
                    .take_while(|from| *from < half_length)
                    .map(|from| (from, (from + dash).min(half_length)))
                    .collect()
            } else {
                vec![(gap, half_length)]
            };
            for side in [-1.0, 1.0] {
                for (from, to) in &pieces {
                    let mut start = center + across * (side * from * meters_per_pixel);
                    let mut end = center + across * (side * to * meters_per_pixel);
                    if let Some(hud) = &collimated {
                        start = hud.map.cling(start);
                        end = hud.map.cling(end);
                    }
                    // On the plate, stop the pitch line at the HUD square.
                    // The screen HUD does this with a mask; the plate's other
                    // marks share that line list, so the square can't be a mask.
                    if hud_fixed {
                        let Some((clipped_start, clipped_end)) = collimated
                            .as_ref()
                            .and_then(|hud| hud.map.clip_to_frame(start, end))
                        else {
                            continue;
                        };
                        start = clipped_start;
                        end = clipped_end;
                    }
                    let width = if collimated.is_some() {
                        line_w * print_scale
                    } else {
                        line_w
                    };
                    app.renderer
                        .render_sets
                        .lines("hud_horizon_lines")
                        .dynamic
                        .push(LineInstance {
                            start: start.into(),
                            end: end.into(),
                            color: COLOR,
                            width,
                        });
                }
                // Its number past each end - not the horizon's.
                if step != 0 {
                    let mut at =
                        center + across * (side * (half_length + label_offset) * meters_per_pixel);
                    if let Some(hud) = &collimated {
                        at = hud.map.cling(at);
                    }
                    if hud_fixed
                        && collimated.as_ref().is_some_and(|hud| {
                            let margin = small_label.0.max(small_label.1) * 0.5 * meters_per_pixel;
                            !hud.map.inside_frame(at, margin)
                        })
                    {
                        continue;
                    }
                    if collimated.is_some() {
                        glass_labels.push((at, degrees.to_string()));
                    } else if let Some(point) = app.renderer.camera_resources.world_to_screen(
                        active,
                        Point3::from(at),
                        screen_width as u32,
                        screen_height as u32,
                    ) {
                        labels.push((point.x as f32, point.y as f32, degrees.to_string()));
                    }
                }
            }
        }

        // On the glass (F6) or the plate: the ladder's numbers and the heading
        // tape's lie on it, cut off by the glass's edge (`hud_text.mask`).
        let (label_width, label_height) = small_label;
        if let Some(hud) = &collimated {
            if hud.visible {
                glass_labels.extend(
                    heading_labels
                        .into_iter()
                        .map(|(x, y, text)| (hud.map.at(x, y), text)),
                );
            }
            let color = UiColor::Rgb(0, 255, 77).to_rgba_f32();
            for (at, text) in glass_labels {
                app.renderer.render_sets.text("hud_text").push(
                    &text,
                    HUD_FONT,
                    hud.map.text_frame(at, SMALL_LABEL_FONT * scale),
                    color,
                );
            }
            return;
        }

        // On the screen: the ladder's numbers, only where they're wholly
        // inside its box, and the heading tape's.
        let inside_box = |x: f32, y: f32| {
            (left_line + label_width * 0.5..=right_line - label_width * 0.5).contains(&x)
                && (line_top + label_height * 0.5..=line_bottom - label_height * 0.5).contains(&y)
        };
        let pitch_placed = labels
            .into_iter()
            .filter(|(x, y, _)| inside_box(*x, *y))
            .take(play_ui::HUD_PITCH_LABELS)
            .enumerate()
            .map(|(index, label)| (play_ui::hud_pitch_label_key(index), label));
        let heading_placed = heading_labels
            .into_iter()
            .take(play_ui::HUD_HEADING_LABELS)
            .enumerate()
            .map(|(index, label)| (play_ui::hud_heading_label_key(index), label));
        for (key, (x, y, text)) in pitch_placed.chain(heading_placed) {
            let Some(node) = scene.ui_node(&play_ui::in_hud(&key)) else {
                continue;
            };
            node.set_active(true);
            node.style.font_size = Some(SMALL_LABEL_FONT * hud_font_scale(scale, 1.0));
            ui_layers::place(
                node,
                x - label_width / 2.0,
                y - label_height / 2.0,
                label_width,
                label_height,
            );
            if let Some(label) = node.as_label_mut() {
                label.set_text(&text);
            }
        }
    }

    /// The camera is in the player's own cockpit - the HUD is the pilot's,
    /// not the camera's.
    pub(super) fn in_own_cockpit(&self, scene: &Scene) -> bool {
        self.camera
            .in_cockpit_of(scene, self.flight_manager.player_id())
    }

    /// The HUD is a plate in the jet's frame ("HUD stays with the jet", from
    /// the cockpit), so the camera can roll and move around it - not glued
    /// to the screen.
    pub(super) fn hud_on_plate(&self, scene: &Scene) -> bool {
        GAME_SETTINGS.lock().unwrap().hud_stays_with_jet && self.in_own_cockpit(scene)
    }

    /// An arrow just inside the HUD square's edge on the plate (`map`),
    /// pointing the way `direction` (world, from the eye) goes off it - for
    /// a lock outside the HUD. Sized in HUD pixels (`scale` - the layout's)
    /// with the plate, its lines `print_scale` wide like the plate's.
    pub(super) fn draw_plate_arrow(
        app: &mut App,
        map: &GlassMap,
        direction: Vector3<f32>,
        scale: f32,
        print_scale: f32,
        color: [f32; 4],
    ) {
        // Its length, half its width at the back, and its lines (HUD px).
        const LENGTH: f32 = 16.0;
        const HALF_WIDTH: f32 = 8.0;
        const LINE_WIDTH: f32 = 1.5;
        let meters_per_pixel = map.meters_per_pixel();
        let (length, half_width) = (
            LENGTH * scale * meters_per_pixel,
            HALF_WIDTH * scale * meters_per_pixel,
        );
        let (x, y, way) = map.held_in_frame(direction, length);
        let across = nalgebra::Vector2::new(-way.y, way.x);
        let middle = nalgebra::Vector2::new(x, y);
        let tip = middle + way * (length * 0.5);
        let base = middle - way * (length * 0.5);
        let corners = [tip, base + across * half_width, base - across * half_width]
            .map(|corner| map.from_plane(corner.x, corner.y));
        for index in 0..3 {
            let (start, end) = (corners[index], corners[(index + 1) % 3]);
            app.renderer
                .render_sets
                .lines("hud_lines")
                .dynamic
                .push(LineInstance {
                    start: start.into(),
                    end: end.into(),
                    color,
                    width: LINE_WIDTH * scale * print_scale,
                });
        }
    }

    /// The helmet display (the F-16's JHMCS), for when the pilot looks away
    /// from the HUD: fixed to the screen - a small cross in its middle, the
    /// bearing in a box over it, the speed in a box left of it and the
    /// altitude in a box right of it, a box on what's locked - or, while
    /// it's out of view, an arrow out from the cross pointing the way to it.
    /// All of it or none: it's up while the pilot looks outside - blanked
    /// while the middle of the view is on the HUD's glass (the HUD's there;
    /// on a jet without a glass outline, the HUD's square on `map` - the
    /// plate), and while it's below the cockpit's sill (the jet's data.ron
    /// `cockpit_sill`: the panel, the floor, the belly and wings, the spine
    /// behind the seat). It fades over HMD_FADE_DEG at both edges. Where the
    /// head can turn isn't touched - only what shows.
    ///
    /// The bearing is where the head points, not the jet.
    ///
    /// In the Fox-2 mode (a heat-seeker selected), while it's up, the helmet
    /// is the seeker's boresight: it looks where the pilot looks (see
    /// FireControl::cue), shown by a small diamond in the middle of the view
    /// inside a circle the size of its field of view - what's in the circle
    /// is what locks. Past the missile's gimbal limit that's crossed out
    /// instead - it can't look there, and rests back on the nose. With the
    /// display down the seeker is on the nose, the HUD's diamond. Screen
    /// size in UI points.
    pub(super) fn draw_hmd(
        &self,
        scene: &mut Scene,
        app: &mut App,
        map: &GlassMap,
        screen_width: f32,
        screen_height: f32,
    ) {
        const COLOR: [f32; 3] = [0.0, 1.0, 0.3];
        // How far past the glass's edge or the sill it fades in over (degrees).
        const HMD_FADE_DEG: f32 = 4.0;
        // Sizes in UI points: the cross's arms; the lines; the readouts'
        // letters and their boxes, how far out from the middle the side ones
        // sit (a fraction of the screen's width) and how far up the
        // bearing's sits (a fraction of its height); the lock box's and the
        // seeker diamond's half sizes.
        const CROSS_ARM: f32 = 5.0;
        const LINE_WIDTH: f32 = 1.2;
        const READOUT_FONT: f32 = 12.0;
        const READOUT_BOX: (f32, f32) = (48.0, 19.0);
        const READOUT_OUT: f32 = 0.12;
        const BEARING_UP: f32 = 0.16;
        const LOCK_HALF: f32 = 9.0;
        const DIAMOND_HALF: f32 = 4.0;
        // The seeker circle's segments.
        const CIRCLE_SEGMENTS: usize = 40;
        // A lock out of view: an arrow this far out from the cross, its
        // length and half its width at the back.
        const ARROW_OUT: f32 = 30.0;
        const ARROW_LENGTH: f32 = 12.0;
        const ARROW_HALF_WIDTH: f32 = 6.0;

        // Real pixels from here on - what the screen lines and the screen text
        // are in.
        let to_pixels = app.window.pixel_size.height as f32 / screen_height.max(1.0);
        let (middle_x, middle_y) = (
            screen_width * 0.5 * to_pixels,
            screen_height * 0.5 * to_pixels,
        );
        let out = READOUT_OUT * screen_width * to_pixels;
        let up = BEARING_UP * screen_height * to_pixels;
        let (box_half_width, box_half_height) = (
            READOUT_BOX.0 * 0.5 * to_pixels,
            READOUT_BOX.1 * 0.5 * to_pixels,
        );

        let active = scene.cameras.active();
        let eye = active.camera.position().coords;
        let look =
            (active.camera.rotation_modifier * active.camera.calc_forward_direction()).normalize();

        // Off the HUD's glass: how far the middle of the view is outside its
        // outline (degrees, negative inside). Measured on a plane square to
        // the line to the glass's middle, where every point is its
        // direction from the eye (so its distances are tangents of angles).
        let player_id = self.flight_manager.player_id();
        let glass: Vec<Vector3<f32>> = match self
            .flight_manager
            .hud_glass(player_id)
            .filter(|glass| glass.len() >= 3)
        {
            Some(glass) => glass.iter().map(|point| map.on_glass(*point)).collect(),
            None => [
                (map.left, map.top),
                (map.right_px, map.top),
                (map.right_px, map.bottom),
                (map.left, map.bottom),
            ]
            .into_iter()
            .map(|(x, y)| map.at(x, y))
            .collect(),
        };
        let off_glass_deg = (|| {
            let middle = glass.iter().sum::<Vector3<f32>>() / glass.len() as f32;
            let toward = (middle - eye).try_normalize(1e-6)?;
            let across = toward
                .cross(&(map.aircraft_rot * Vector3::y()))
                .try_normalize(1e-6)?;
            let upward = across.cross(&toward);
            let on_plane = |direction: Vector3<f32>| {
                let ahead = direction.dot(&toward);
                (ahead > 1e-3).then(|| {
                    [
                        direction.dot(&across) / ahead,
                        direction.dot(&upward) / ahead,
                    ]
                })
            };
            let outline: Vec<[f32; 2]> = glass
                .iter()
                .map(|point| on_plane(point - eye))
                .collect::<Option<_>>()?;
            let edges = convex_mask(&outline, 0.0)?;
            // Looking away from the glass altogether: well off it.
            let Some([x, y]) = on_plane(look) else {
                return Some(90.0);
            };
            let inside = edges
                .iter()
                .map(|edge| edge[0] * x + edge[1] * y + edge[2])
                .fold(f32::MAX, f32::min);
            Some(-inside.atan().to_degrees())
        })()
        // No outline to go by (behind the eye): not on it.
        .unwrap_or(90.0);
        // Above the sill: how far the view is over it (degrees), in the jet's
        // frame - round from the nose (either side), and up from level.
        let local = map.aircraft_rot.inverse() * look;
        let azimuth = local.x.atan2(local.z).to_degrees();
        let elevation = local.y.clamp(-1.0, 1.0).asin().to_degrees();
        let over_sill_deg = crate::prefabs::plane::aircraft_spec::sill_elevation(
            self.flight_manager.cockpit_sill(player_id),
            azimuth,
        )
        .map_or(90.0, |sill| elevation - sill);
        let alpha = (off_glass_deg / HMD_FADE_DEG).clamp(0.0, 1.0)
            * (over_sill_deg / HMD_FADE_DEG).clamp(0.0, 1.0);
        if alpha <= 0.0 {
            return;
        }
        let line_width = LINE_WIDTH * to_pixels;

        // The Fox-2 mode: the helmet's boresight - the seeker - in the middle
        // of the view.
        let gimbal_limit = self
            .weapons
            .selected_weapon
            .as_ref()
            .and_then(|weapon| self.weapons.weapon_specs.get(weapon))
            .filter(|spec| {
                spec.category == WeaponCategory::Missile && spec.guidance.seeker == Seeker::InfraRed
            })
            .map(|spec| spec.guidance.gimbal_limit_deg);
        if let Some(limit) = gimbal_limit {
            self.weapons.hmd_cue.set(Some(look));
            let color = [COLOR[0], COLOR[1], COLOR[2], 0.9 * alpha];
            let mut boresight_line = |start: (f32, f32), end: (f32, f32)| {
                app.renderer
                    .render_sets
                    .lines("hud_screen_lines")
                    .dynamic
                    .push(LineInstance {
                        start: [start.0, start.1, 0.0],
                        end: [end.0, end.1, 0.0],
                        color,
                        width: line_width,
                    });
            };
            let nose = map.aircraft_rot * Vector3::z();
            if nose.angle(&look).to_degrees() > limit {
                // Past the gimbal limit: it can't look there.
                let reach = CROSS_ARM * 1.6 * to_pixels;
                boresight_line(
                    (middle_x - reach, middle_y - reach),
                    (middle_x + reach, middle_y + reach),
                );
                boresight_line(
                    (middle_x - reach, middle_y + reach),
                    (middle_x + reach, middle_y - reach),
                );
            } else if self.weapons.fire_control.seeker_target.is_none() {
                // Its field of view on screen, and the diamond in its middle.
                let half_fov = (active.projection.fovy.to_radians() * 0.5).tan().max(1e-4);
                let radius =
                    fire_control::SEEKER_FIELD_DEG.to_radians().tan() / half_fov * middle_y;
                let point = |index: usize| {
                    let angle = index as f32 / CIRCLE_SEGMENTS as f32 * std::f32::consts::TAU;
                    (
                        middle_x + radius * angle.cos(),
                        middle_y + radius * angle.sin(),
                    )
                };
                for index in 0..CIRCLE_SEGMENTS {
                    boresight_line(point(index), point(index + 1));
                }
                let half = DIAMOND_HALF * to_pixels;
                let corners = [
                    (middle_x, middle_y - half),
                    (middle_x + half, middle_y),
                    (middle_x, middle_y + half),
                    (middle_x - half, middle_y),
                ];
                for index in 0..4 {
                    boresight_line(corners[index], corners[(index + 1) % 4]);
                }
            }
        }

        let line_color = [COLOR[0], COLOR[1], COLOR[2], 0.9 * alpha];
        let text_color = [0.0, 1.0, 77.0 / 255.0, alpha];

        let mut lines: Vec<((f32, f32), (f32, f32))> = Vec::new();
        let rectangle = |lines: &mut Vec<((f32, f32), (f32, f32))>,
                         x: f32,
                         y: f32,
                         half_width: f32,
                         half_height: f32| {
            let corners = [
                (x - half_width, y - half_height),
                (x + half_width, y - half_height),
                (x + half_width, y + half_height),
                (x - half_width, y + half_height),
            ];
            for index in 0..4 {
                lines.push((corners[index], corners[(index + 1) % 4]));
            }
        };
        let triangle = |lines: &mut Vec<((f32, f32), (f32, f32))>,
                        middle: nalgebra::Vector2<f32>,
                        way: nalgebra::Vector2<f32>,
                        length: f32,
                        half_width: f32| {
            let across = nalgebra::Vector2::new(-way.y, way.x);
            let tip = middle + way * (length * 0.5);
            let base = middle - way * (length * 0.5);
            let corners = [tip, base + across * half_width, base - across * half_width];
            for index in 0..3 {
                lines.push((
                    (corners[index].x, corners[index].y),
                    (corners[(index + 1) % 3].x, corners[(index + 1) % 3].y),
                ));
            }
        };
        // Where a world point is on screen (real pixels), if it's on it.
        let on_screen = |point: Vector3<f32>| {
            project_hud_point(active, point, screen_width, screen_height)
                .map(|(x, y)| (x * to_pixels, y * to_pixels))
                .filter(|(x, y)| {
                    (0.0..=screen_width * to_pixels).contains(x)
                        && (0.0..=screen_height * to_pixels).contains(y)
                })
        };

        // The cross, following the view.
        let arm = CROSS_ARM * to_pixels;
        lines.push(((middle_x - arm, middle_y), (middle_x + arm, middle_y)));
        lines.push(((middle_x, middle_y - arm), (middle_x, middle_y + arm)));

        // The box on what's locked, where it is in the view.
        let locked = self
            .weapons
            .hud_symbol
            .as_ref()
            .filter(|symbol| !symbol.diamond)
            .and_then(|symbol| symbol.track.as_ref())
            .and_then(|name| crate::ext::drawn::get(scene, name))
            .map(|instance| instance.instance.transform.position);
        if let Some((x, y)) = locked.and_then(on_screen) {
            let half = LOCK_HALF * to_pixels;
            rectangle(&mut lines, x, y, half, half);
        }

        // A lock out of view: an arrow out from the cross, pointing the way
        // to turn to it (the view's own right and up; straight behind,
        // down - turn round).
        let target = self
            .weapons
            .fire_control
            .seeker_target
            .as_ref()
            .and_then(|name| crate::ext::drawn::get(scene, name))
            .map(|instance| instance.instance.transform.position);
        let (width, height) = (app.window.size.width, app.window.size.height);
        let out_of_view = target.filter(|target| {
            app.renderer
                .camera_resources
                .world_to_screen(active, Point3::from(*target), width, height)
                .is_none()
        });
        if let Some(target) = out_of_view {
            let view_up = active.camera.rotation_modifier * active.camera.up;
            if let Some(right) = look.cross(&view_up).try_normalize(1e-6) {
                let view_up = right.cross(&look);
                let to_target = target - eye;
                let way = nalgebra::Vector2::new(to_target.dot(&right), -to_target.dot(&view_up))
                    .try_normalize(1e-6)
                    .unwrap_or_else(|| nalgebra::Vector2::new(0.0, 1.0));
                let middle =
                    nalgebra::Vector2::new(middle_x, middle_y) + way * (ARROW_OUT * to_pixels);
                triangle(
                    &mut lines,
                    middle,
                    way,
                    ARROW_LENGTH * to_pixels,
                    ARROW_HALF_WIDTH * to_pixels,
                );
            }
        }

        // The bearing over the cross, the speed left of it and the altitude
        // right of it, each in its box - as the HUD reads them.
        let readouts = [
            ("hud_heading", middle_x, middle_y - up),
            ("hud_speed", middle_x - out, middle_y),
            ("hud_altitude", middle_x + out, middle_y),
        ];
        for (_, x, y) in readouts {
            rectangle(&mut lines, x, y, box_half_width, box_half_height);
        }

        for (start, end) in lines {
            app.renderer
                .render_sets
                .lines("hud_screen_lines")
                .dynamic
                .push(LineInstance {
                    start: [start.0, start.1, 0.0],
                    end: [end.0, end.1, 0.0],
                    color: line_color,
                    width: line_width,
                });
        }
        // The bearing where the head points (north +Z, east -X - as the HUD's
        // heading tape has it); straight up or down, the last one stands.
        let head_bearing = Vector3::new(look.x, 0.0, look.z)
            .try_normalize(1e-3)
            .map(|toward| {
                ((-toward.x)
                    .atan2(toward.z)
                    .to_degrees()
                    .rem_euclid(360.0)
                    .round() as i32
                    % 360)
                    .to_string()
            });
        for (key, x, y) in readouts {
            let Some(label) = scene
                .ui_node(&play_ui::in_hud(key))
                .and_then(|node| node.as_label_mut())
            else {
                continue;
            };
            let text = match (key, &head_bearing) {
                ("hud_heading", Some(bearing)) => bearing.clone(),
                _ => label.text().to_owned(),
            };
            app.renderer.render_sets.text("hmd_text").push(
                &text,
                HUD_FONT,
                TextFrame::screen(x, y, READOUT_FONT * to_pixels),
                text_color,
            );
        }
    }

    /// The seeker's symbol (see `HudSymbol`). Over the world, it sits on its
    /// point at a fixed size on screen. On the plate (`plate` - the HUD
    /// fixed to the jet, and how big it is on screen), it is where the line
    /// from the camera to the point crosses the plate - so it sits on the
    /// point whatever the head does (the plate is close: from the design eye
    /// a few cm of head offset would put it degrees off) - sized and turned
    /// with the plate, and left out once that's off the HUD's square. Not
    /// drawn behind the camera.
    pub(super) fn draw_hud_symbol(
        &self,
        scene: &Scene,
        app: &mut App,
        symbol: HudSymbol,
        plate: Option<(&GlassMap, f32)>,
    ) {
        const COLOR: [f32; 4] = [0.0, 1.0, 0.3, 0.95];
        // Its lines' width (px) - the diamond's, the box's.
        const DIAMOND_WIDTH: f32 = 4.0;
        const BOX_WIDTH: f32 = 1.2;
        let HudSymbol {
            point,
            track,
            up,
            half_size,
            diamond,
        } = symbol;
        // A tracked aircraft where it's drawn this frame, not where the
        // weapons update last saw it.
        let point = track
            .and_then(|name| crate::ext::drawn::get(scene, &name))
            .map_or(point, |instance| instance.instance.transform.position);
        let camera = scene.cameras.active().camera.position().coords;
        let scale = self.hud.hud_layout.draw_scale();
        let half_size = half_size * scale;
        let line_width = (if diamond { DIAMOND_WIDTH } else { BOX_WIDTH }) * scale;
        let (center, right, upward, half, width) = if let Some((map, print_scale)) = plate {
            let Some(direction) = (point - camera).try_normalize(1e-6) else {
                return;
            };
            let half = half_size * map.meters_per_pixel();
            let Some(center) = map
                .ray_from(camera, direction)
                .filter(|center| map.inside_frame(*center, half))
            else {
                // Off the HUD's square: a lock gets an arrow at its edge,
                // pointing the way to it.
                if !diamond {
                    Self::draw_plate_arrow(app, map, direction, scale, print_scale, COLOR);
                }
                return;
            };
            (
                center,
                map.aircraft_rot * map.right,
                map.aircraft_rot * map.up,
                half,
                line_width * print_scale,
            )
        } else {
            let active = scene.cameras.active();
            let (width, height) = (app.window.size.width, app.window.size.height);
            if app
                .renderer
                .camera_resources
                .world_to_screen(active, Point3::from(point), width, height)
                .is_none()
            {
                return;
            }
            let view = point - active.camera.position().coords;
            let distance = view.magnitude();
            let (Some(right), true) = (view.cross(&up).try_normalize(1e-6), distance > 1.0) else {
                return;
            };
            let screen_up = right.cross(&(view / distance));
            let pixels = app.window.pixel_size.height.max(1) as f32;
            let meters =
                half_size * 2.0 * distance * (active.projection.fovy.to_radians() * 0.5).tan()
                    / pixels;
            (point, right, screen_up, meters, line_width)
        };
        let corners = if diamond {
            [
                center + upward * half,
                center + right * half,
                center - upward * half,
                center - right * half,
            ]
        } else {
            [
                center + (upward + right) * half,
                center + (right - upward) * half,
                center - (upward + right) * half,
                center + (upward - right) * half,
            ]
        };
        for index in 0..4 {
            let (start, end) = (corners[index], corners[(index + 1) % 4]);
            app.renderer
                .render_sets
                .lines("hud_lines")
                .dynamic
                .push(LineInstance {
                    start: start.into(),
                    end: end.into(),
                    color: COLOR,
                    width,
                });
        }
    }

    pub(super) fn format_duration(seconds: f64) -> String {
        let duration = Duration::from_secs_f64(seconds);

        let total_millis = duration.as_millis() as u64;
        let hours = total_millis / 3_600_000; // 1 hour = 3,600,000 milliseconds
        let minutes = (total_millis % 3_600_000) / 60_000; // 1 minute = 60,000 milliseconds
        let seconds = (total_millis % 60_000) / 1_000; // 1 second = 1,000 milliseconds
        let milliseconds = total_millis % 1_000; // Remaining milliseconds
                                                 // Format as hh:mm:ss:milmilmil
        format!(
            "{:02}:{:02}:{:02}:{:03}",
            hours, minutes, seconds, milliseconds
        )
    }

    pub(super) fn ui_control(&mut self, scene: &mut Scene, app: &mut App) {
        let ui_elapsed = self
            .hud
            .last_ui_update
            .elapsed()
            .min(Duration::from_millis(100));
        if ui_elapsed >= Duration::from_secs_f32(1.0 / 120.0) {
            let ui_delta_time = ui_elapsed.as_secs_f32();

            // Reads the "player" node once up front (plain Copy values, so
            // nothing borrowed from `scene` needs to stay alive afterward)
            // instead of fetching it again at every label below - throttle
            // from its Plane behavior, everything else from the latest state
            // its physics half published.
            let player_id = self.flight_manager.player_id();
            let player = scene.content.nodes.get(player_id);
            let throttle = player
                .and_then(|node| node.get_behavior::<Plane>())
                .map(|plane| plane.controls.throttle)
                .unwrap_or(0.0);
            let (g_meter, altimeter, speedometer) = player
                .and_then(|node| node.physics_state::<AircraftState>())
                .map(|state| {
                    (
                        state.flight_data.g_meter,
                        state.flight_data.altimeter,
                        state.flight_data.speedometer,
                    )
                })
                .unwrap_or((0.0, 0.0, 0.0));
            let (fuel_kg, fuel_capacity) = player
                .and_then(|node| node.get_behavior::<Plane>())
                .map(|plane| plane.fuel())
                .unwrap_or((0.0, 0.0));

            if let Some(label) = scene
                .ui_node("hud/game_ui/data_box/framerate")
                .and_then(|n| n.as_label_mut())
            {
                label.set_text(&format!("FPS: {}", app.time.fps()));
            }

            if let Some(label) = scene
                .ui_node("hud/game_ui/data_box/g")
                .and_then(|n| n.as_label_mut())
            {
                label.set_text(&format!("G: {:.0}", g_meter));
            }

            if let Some(label) = scene
                .ui_node("hud/game_ui/data_box/timer")
                .and_then(|n| n.as_label_mut())
            {
                label.set_text(&Self::format_duration(self.game_time));
            }

            if let Some(label) = scene
                .ui_node("hud/game_ui/data_box/power")
                .and_then(|n| n.as_label_mut())
            {
                label.set_text(&format!("Power: {}%", (throttle * 100.0).round()));
            }
            if let Some(label) = scene
                .ui_node("hud/game_ui/data_box/fuel")
                .and_then(|n| n.as_label_mut())
            {
                let text = if fuel_capacity > 0.0 {
                    format!("Fuel: {:.0} / {:.0} kg", fuel_kg, fuel_capacity)
                } else {
                    "Fuel: —".to_owned()
                };
                label.set_text(&text);
            }

            if let Some(label) = scene
                .ui_node("hud/game_ui/data_box/weapons")
                .and_then(|n| n.as_label_mut())
            {
                label.set_text(&self.weapons.weapons_status);
            }
            if let Some(label) = scene
                .ui_node("hud/game_ui/data_box/countermeasures")
                .and_then(|n| n.as_label_mut())
            {
                label.set_text(&self.weapons.countermeasures_status);
            }
            play_ui::update_weapon_list(scene, app, &self.weapons.weapon_list);

            // The speed, in knots (FlightData::speedometer already is).
            if let Some(label) = scene
                .ui_node("hud/hud_speed")
                .and_then(|node| node.as_label_mut())
            {
                label.set_text(&format!("{:.0}", speedometer));
            }
            if let Some(label) = scene
                .ui_node("hud/hud_altitude")
                .and_then(|node| node.as_label_mut())
            {
                label.set_text(&format!("{:.0}", altimeter));
            }

            // F3's debug panel - see play::debug_tools.
            self.update_debug_panel(scene, app);

            // Every aircraft's name over it - see flight_manager::update_name_tags.
            self.flight_manager
                .update_name_tags(scene, app, self.pilot_view.hud_hidden_by_g);

            self.hud.last_ui_update = Instant::now();
        }
    }

    pub(super) fn map_to_range(
        x: f64,
        in_min: f64,
        in_max: f64,
        out_min: f64,
        out_max: f64,
    ) -> f64 {
        (x - in_min) * (out_max - out_min) / (in_max - in_min) + out_min
    }
}

#[cfg(test)]
mod hud_boot_tests {
    use super::*;

    #[test]
    fn hud_fades_in_then_stays_on() {
        assert_eq!(hud_boot_alpha(0.0), 0.0);
        let early = hud_boot_alpha(HUD_BOOT_FADE * 0.5);
        assert!(early > 0.0 && early < 1.0, "{early}");
        assert_eq!(hud_boot_alpha(hud_boot_length()), 1.0);
        assert_eq!(hud_boot_alpha(hud_boot_length() + 2.0), 1.0);
    }
}

#[cfg(test)]
mod bank_tests {
    use super::*;

    #[test]
    fn bank_is_measured_against_the_horizon() {
        let level = UnitQuaternion::identity();
        assert!(bank_angle(&level).unwrap().abs() < 1e-3);
        // Rolled about the nose (+Z): +30 lifts the left wing (+X)... check
        // by which way up tips: up toward -X is right wing down.
        let right_down = UnitQuaternion::from_axis_angle(&Vector3::z_axis(), 30f32.to_radians());
        let up = right_down * Vector3::y();
        let expected = if up.x < 0.0 { 30.0 } else { -30.0 };
        assert!((bank_angle(&right_down).unwrap() - expected).abs() < 1e-3);
        // Pitched up 40 degrees, then rolled: still the same bank.
        let pitched =
            UnitQuaternion::from_axis_angle(&Vector3::x_axis(), -40f32.to_radians()) * right_down;
        assert!(
            (bank_angle(&pitched).unwrap() - expected).abs() < 1e-2,
            "{:?}",
            bank_angle(&pitched)
        );
        // Upside down.
        let inverted = UnitQuaternion::from_axis_angle(&Vector3::z_axis(), 180f32.to_radians());
        assert!((bank_angle(&inverted).unwrap().abs() - 180.0).abs() < 1e-3);
        // Straight up: no bank.
        let vertical = UnitQuaternion::from_axis_angle(&Vector3::x_axis(), -90f32.to_radians());
        assert_eq!(bank_angle(&vertical), None);
    }
}
