//! The ocean lab - a test bench for the sea: what it costs to draw, and what
//! each setting behind that cost changes. One sea, the runway island (for
//! shore foam), a free camera, and a debug window that's always up:
//! - the frame rate and time - vsync starts off, so it's the real one;
//! - the mesh: the layout the game uses (main.rs's OCEAN_TIERS) or the old,
//!   denser one, which altitude tier, frustum culling, what's being drawn;
//! - the waves: the wind, how many geometry and detail waves;
//! - per pixel: how many waves one pixel may sum, how small a wave may get
//!   on screen before it's dropped;
//! - the pipeline: alpha blending or none, the depth copy (shore foam);
//! - debug views: how many waves each pixel summed, the mesh's cells.
//!
//! Frustum culling: the engine draws a mesh whole, so here the sea's mesh is
//! cut into SECTORS wedges around the camera, each its own node, and only
//! the wedges the camera's view can reach are drawn (`sector_visible`).
//!
//! Open it by setting game::ext::splash_screen's NEXT_SCENE to "ocean_lab".
//! Camera: hold the right mouse button to look around; WASD to move,
//! Space / Left Ctrl up and down, Left Shift faster.

use std::collections::{HashMap, HashSet};
use std::f32::consts::{FRAC_PI_2, PI, TAU};

use nalgebra::{Matrix4, Vector3, Vector4};

use pankarta_software::asset_manager::ModelVertex;
use pankarta_software::prelude::*;
use pankarta_software::rendering::frame::FramePlan;
use pankarta_software::scene_manager::node::properties::Material;

use crate::game::camera::camera::looking_along;
use crate::game::ext::mesh_data::{self, WaterRing, WaterTier};
use crate::game::ocean::{Ocean, OceanDebugView, OceanQuality, OceanSettings, MAX_WAVES, OCEAN_SHADER};
use crate::game::scenes::follow_test::scene::sun_node;

/// How many wedges the sea's mesh is cut into for culling.
const SECTORS: usize = 16;
const CAMERA: &str = "lab_camera";
/// Where the camera starts: out at sea south of the island, looking at it.
const START: Vector3<f32> = Vector3::new(0.0, 40.0, -14_000.0);
/// The source the pipeline variants are made from.
const OCEAN_WGSL: &str = "assets/shaders/ocean.wgsl";
/// The one line of ocean.wgsl that reads the depth copy - see its
/// `scene_depth`. The variants without the copy return the far plane.
const DEPTH_READ: &str = "return textureLoad(t_scene_depth, pixel, 0).x;";
/// ocean.wgsl's GEOMETRY_MIN_CELLS: a ring moves the waves at least this
/// many of its cells long.
const GEOMETRY_MIN_CELLS: f32 = 3.0;

/// The layout the game used before (2 m cells, 256 across every ring,
/// doubling out to 32 km, then two flat rings) - to compare against.
const ORIGINAL_TIERS: &[WaterTier] = &[
    WaterTier { min_altitude: f32::NEG_INFINITY, rings: &[
        WaterRing { cell: 2.0, outer_half_extent: 256.0 },
        WaterRing { cell: 4.0, outer_half_extent: 512.0 },
        WaterRing { cell: 8.0, outer_half_extent: 1_024.0 },
        WaterRing { cell: 16.0, outer_half_extent: 2_048.0 },
        WaterRing { cell: 32.0, outer_half_extent: 4_096.0 },
        WaterRing { cell: 64.0, outer_half_extent: 8_192.0 },
        WaterRing { cell: 128.0, outer_half_extent: 16_384.0 },
        WaterRing { cell: 256.0, outer_half_extent: 32_768.0 },
        WaterRing { cell: 1_024.0, outer_half_extent: 131_072.0 },
        WaterRing { cell: 16_384.0, outer_half_extent: 2_097_152.0 },
    ] },
    WaterTier { min_altitude: 1_000.0, rings: &[
        WaterRing { cell: 8.0, outer_half_extent: 1_024.0 },
        WaterRing { cell: 16.0, outer_half_extent: 2_048.0 },
        WaterRing { cell: 32.0, outer_half_extent: 4_096.0 },
        WaterRing { cell: 64.0, outer_half_extent: 8_192.0 },
        WaterRing { cell: 128.0, outer_half_extent: 16_384.0 },
        WaterRing { cell: 256.0, outer_half_extent: 32_768.0 },
        WaterRing { cell: 1_024.0, outer_half_extent: 131_072.0 },
        WaterRing { cell: 16_384.0, outer_half_extent: 2_097_152.0 },
    ] },
    WaterTier { min_altitude: 4_000.0, rings: &[
        WaterRing { cell: 32.0, outer_half_extent: 4_096.0 },
        WaterRing { cell: 64.0, outer_half_extent: 8_192.0 },
        WaterRing { cell: 128.0, outer_half_extent: 16_384.0 },
        WaterRing { cell: 256.0, outer_half_extent: 32_768.0 },
        WaterRing { cell: 1_024.0, outer_half_extent: 131_072.0 },
        WaterRing { cell: 16_384.0, outer_half_extent: 2_097_152.0 },
    ] },
];

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Layout {
    /// main.rs's OCEAN_TIERS - what the game draws.
    Compact,
    /// ORIGINAL_TIERS.
    Original,
}

impl Layout {
    fn tiers(self) -> &'static [WaterTier] {
        match self {
            Layout::Compact => crate::OCEAN_TIERS,
            Layout::Original => ORIGINAL_TIERS,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Layout::Compact => "compact",
            Layout::Original => "original",
        }
    }
}

/// One wedge of one tier's mesh, registered as its own custom mesh.
struct SectorMesh {
    name: String,
    triangles: usize,
    vertices: usize,
    /// Convex hull (mesh XZ) of everywhere its vertices can be - where they
    /// sit and where the ring morph can slide them - for culling.
    hull: Vec<[f32; 2]>,
}

struct TierMeshes {
    sectors: Vec<SectorMesh>,
    triangles: usize,
    vertices: usize,
}

/// What the last frame drew.
#[derive(Clone, Copy, Default)]
struct Stats {
    tier: usize,
    triangles: usize,
    vertices: usize,
    sectors: usize,
    tier_triangles: usize,
    tier_vertices: usize,
    altitude: f32,
}

pub struct GameLogic {
    ocean: Ocean,
    /// What the wave and per-pixel controls edit - put on `ocean` when they
    /// change.
    settings: OceanSettings,
    layout: Layout,
    /// None: by the camera's altitude, like the game.
    forced_tier: Option<usize>,
    culling: bool,
    /// Alpha blending, like ocean.ron (the game's) - off is `blend: None`.
    alpha_blend: bool,
    /// The frame plan's CopyDepth and the shore foam that reads it.
    depth_copy: bool,
    debug_view: OceanDebugView,
    vsync: bool,
    /// Every layout built so far, per tier - the old one only once picked.
    meshes: HashMap<Layout, Vec<TierMeshes>>,
    /// ocean.wgsl's source for the pipeline variants, or why it couldn't be
    /// read.
    wgsl: Result<String, String>,
    /// The pipeline variants registered so far (see `shader_name`).
    variants: HashSet<String>,
    problem: Option<String>,
    yaw: f32,
    pitch: f32,
    fly_speed: f32,
    /// Frame time, smoothed, ms.
    frame_ms: f32,
    stats: Stats,
}

impl GameLogic {
    pub fn new(scene: &mut Scene, app: &mut App) -> Self {
        app.renderer.set_environment(Environment::ProceduralSky { sun: true });
        let mut meshes = HashMap::new();
        meshes.insert(Layout::Compact, build_layout(app, Layout::Compact));

        let (yaw, pitch) = (FRAC_PI_2, -4.0f32.to_radians());
        let at = |position: Vector3<f32>, scale: f32| Transform3D { position, rotation: UnitQuaternion::identity(), scale: Vector3::new(scale, scale, scale) };
        let mut nodes = vec![
            sun_node([0.7, 0.7, 0.8]),
            // Same island and far landmass as play/follow_test - something
            // solid in the water, for the shore foam.
            Node::new("island").with_property(at(Vector3::new(0.0, 100.0, 0.0), 60.0)).with_property(Model::file("models/Runway/Runway.glb").double_sided(DoubleSidedMeshes::All)),
            Node::new("ground").with_property(at(Vector3::new(0.0, 0.0, 50_000.0), 30_000.0)).with_property(Model::file("models/ground/ground.glb")),
            Node::new(CAMERA)
                .with_property(Transform3D { position: START, rotation: looking_along(forward_of(yaw, pitch)), ..Default::default() })
                .with_property(CameraLens::new(70.0)),
        ];
        // The sea, one node per wedge - moved under the camera every frame.
        for (index, sector) in meshes[&Layout::Compact][0].sectors.iter().enumerate() {
            nodes.push(Node::new(sector_node(index))
                .with_property(at(Vector3::zeros(), 1.0))
                .with_property(Mesh::custom(sector.name.clone()).material(Ocean::material()).casts_shadows(false)));
        }
        for node in nodes {
            if let Err(error) = scene.spawn_node(app, node) {
                eprintln!("{error}");
            }
        }

        let settings = OceanSettings::default();
        Self {
            ocean: Ocean::new(settings.clone()),
            settings,
            layout: Layout::Compact,
            forced_tier: None,
            culling: true,
            alpha_blend: true,
            depth_copy: true,
            debug_view: OceanDebugView::Normal,
            vsync: false,
            meshes,
            wgsl: std::fs::read_to_string(OCEAN_WGSL).map_err(|error| format!("couldn't read {OCEAN_WGSL}: {error}")),
            variants: HashSet::new(),
            problem: None,
            yaw,
            pitch,
            fly_speed: 60.0,
            frame_ms: 0.0,
            stats: Stats::default(),
        }
    }

    /// The shader the sea's nodes draw with: the game's own for its
    /// pipeline (alpha blend + depth copy), else a variant of it registered
    /// from ocean.wgsl's source with the other settings.
    fn shader_name(&mut self, app: &mut App) -> String {
        if self.alpha_blend && self.depth_copy {
            return OCEAN_SHADER.to_owned();
        }
        let name = format!("ocean_lab/{}_{}", if self.alpha_blend { "alpha" } else { "opaque" }, if self.depth_copy { "depth" } else { "no_depth" });
        if !self.variants.contains(&name) {
            match variant_source(&self.wgsl, self.depth_copy) {
                Ok(wgsl) => {
                    app.renderer.shaders.add(&name, &wgsl, Some(&variant_settings(self.alpha_blend, self.depth_copy)));
                    self.variants.insert(name.clone());
                }
                Err(error) => {
                    // Back to the game's own pipeline.
                    self.problem = Some(error);
                    self.alpha_blend = true;
                    self.depth_copy = true;
                    app.renderer.frame_plan = frame_plan(true);
                    return OCEAN_SHADER.to_owned();
                }
            }
        }
        name
    }

    fn set_camera(&mut self, scene: &mut Scene, position: Option<Vector3<f32>>, pitch_deg: f32) {
        self.pitch = pitch_deg.to_radians();
        let rotation = looking_along(forward_of(self.yaw, self.pitch));
        if let Some(transform) = scene.get_node_mut(CAMERA).and_then(|node| node.transform_3d_mut()) {
            if let Some(position) = position {
                transform.position = position;
            }
            transform.rotation = rotation;
        }
    }

    /// Everything onto `quality`: its waves and per-pixel budget, its shore
    /// foam (and depth copy) - and what every preset shares: the game's
    /// mesh, culling on, no blending (the water's opaque, so it looks the
    /// same and skips reading the screen).
    fn apply_quality(&mut self, quality: OceanQuality, app: &mut App) {
        let mut settings = self.settings.clone();
        quality.apply(&mut settings);
        self.ocean.set_settings(settings.clone());
        self.settings = settings;
        self.layout = Layout::Compact;
        self.culling = true;
        self.alpha_blend = false;
        self.depth_copy = quality.shore_foam();
        app.renderer.frame_plan = frame_plan(self.depth_copy);
    }

    /// The preset everything's on now - none once a control is moved off it.
    fn current_quality(&self) -> Option<OceanQuality> {
        OceanQuality::ALL.into_iter().find(|quality| {
            quality.matches(&self.settings) && self.culling && !self.alpha_blend && self.depth_copy == quality.shore_foam() && self.layout == Layout::Compact
        })
    }

    /// Everything the lab is set to and measuring, as text - what "Copy
    /// settings" puts on the clipboard.
    fn report(&self, scene: &Scene, app: &App) -> String {
        let stats = self.stats;
        let camera = scene.get_node(CAMERA).and_then(|node| node.transform_3d()).map(|transform| transform.position).unwrap_or(START);
        let quality = self.current_quality().map_or("Custom", |quality| quality.name());
        format!(
            "Ocean lab\n\
             frame: {:.0} fps, {:.2} ms (vsync {})\n\
             screen: {} x {} px\n\
             camera: ({:.0}, {:.0}, {:.0}), {:.0} m above the sea, yaw {:.0} deg, pitch {:.0} deg\n\
             preset: {quality}\n\
             mesh: {:?} layout, tier {} ({}), culling {}\n\
             drawn: {} triangles, {} vertices, {}/{SECTORS} wedges (whole tier {} triangles)\n\
             pipeline: alpha blend {}, depth copy + shore foam {}\n\
             view: {:?}\n\
             shortest wave drawn at this height: {:.1} m\n\
             settings: {:#?}",
            app.time.fps(), self.frame_ms, if self.vsync { "on" } else { "off" },
            app.window.pixel_size.width, app.window.pixel_size.height,
            camera.x, camera.y, camera.z, stats.altitude, self.yaw.to_degrees(), self.pitch.to_degrees(),
            self.layout, stats.tier, if self.forced_tier.is_some() { "fixed" } else { "auto" }, if self.culling { "on" } else { "off" },
            thousands(stats.triangles), thousands(stats.vertices), stats.sectors, thousands(stats.tier_triangles),
            if self.alpha_blend { "on" } else { "off" }, if self.depth_copy { "on" } else { "off" },
            self.debug_view,
            self.ocean.shortest_wave_at(camera.y),
            self.settings,
        )
    }

    fn controls(&mut self, ui: &mut egui::Ui, scene: &mut Scene, app: &mut App) {
        let stats = self.stats;
        let pixels = (app.window.pixel_size.width, app.window.pixel_size.height);
        ui.label(egui::RichText::new(format!("{:.0} fps  ·  {:.2} ms", app.time.fps(), self.frame_ms)).size(20.0).strong());
        if ui.button("Copy settings").on_hover_text("Everything set here, the camera and what it's drawing - as text, to paste").clicked() {
            let report = self.report(scene, app);
            ui.ctx().copy_text(report);
        }
        ui.label(format!("Drawing at {} x {} pixels ({:.1} M)", pixels.0, pixels.1, (pixels.0 * pixels.1) as f32 / 1e6));
        if ui.checkbox(&mut self.vsync, "Vsync (off = measure the real frame time)").changed() {
            app.renderer.set_vsync(self.vsync);
        }
        ui.label(format!("Camera {:.0} m above the sea", stats.altitude));
        ui.small("Right mouse: look  ·  WASD: move  ·  Space / Ctrl: up / down  ·  Shift: x10");
        ui.add(egui::Slider::new(&mut self.fly_speed, 5.0..=1_000.0).logarithmic(true).text("fly speed m/s"));
        ui.horizontal_wrapped(|ui| {
            let mut preset = None;
            if ui.button("10 m").clicked() { preset = Some((10.0, -3.0)); }
            if ui.button("150 m").clicked() { preset = Some((150.0, -8.0)); }
            if ui.button("1.5 km").clicked() { preset = Some((1_500.0, -15.0)); }
            if ui.button("6 km").clicked() { preset = Some((6_000.0, -20.0)); }
            if ui.button("Look down").clicked() {
                self.set_camera(scene, None, -89.0);
            }
            if ui.button("Start").clicked() {
                self.yaw = FRAC_PI_2;
                self.set_camera(scene, Some(START), -4.0);
            }
            if let Some((height, pitch)) = preset {
                let position = scene.get_node(CAMERA).and_then(|node| node.transform_3d()).map(|transform| transform.position).unwrap_or(START);
                let sea = self.ocean.sea_level();
                self.set_camera(scene, Some(Vector3::new(position.x, sea + height, position.z)), pitch);
            }
        });

        ui.separator();
        ui.heading("Quality preset");
        // Which preset everything's on now - none once a control below is
        // moved off it.
        let current = self.current_quality();
        let mut picked = None;
        ui.horizontal_wrapped(|ui| {
            for quality in OceanQuality::ALL {
                if ui.selectable_label(current == Some(quality), quality.name()).on_hover_text(quality.description()).clicked() {
                    picked = Some(quality);
                }
            }
        });
        ui.small(match current {
            Some(quality) => quality.description(),
            None => "Custom - the controls below are off every preset",
        });
        if let Some(quality) = picked {
            self.apply_quality(quality, app);
        }

        ui.separator();
        ui.heading("Mesh");
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.layout, Layout::Compact, "Compact (game)");
            ui.radio_value(&mut self.layout, Layout::Original, "Original");
        });
        if !self.meshes.contains_key(&self.layout) {
            let built = build_layout(app, self.layout);
            self.meshes.insert(self.layout, built);
        }
        let tiers = self.layout.tiers();
        egui::ComboBox::from_label("Altitude tier")
            .selected_text(match self.forced_tier { None => format!("Auto (tier {})", stats.tier), Some(tier) => format!("Tier {tier}") })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut self.forced_tier, None, "Auto - by altitude");
                for (index, tier) in tiers.iter().enumerate() {
                    let from = if tier.min_altitude.is_finite() { format!("from {:.0} m", tier.min_altitude) } else { "sea level".to_owned() };
                    ui.selectable_value(&mut self.forced_tier, Some(index), format!("Tier {index} ({from})"));
                }
            });
        ui.checkbox(&mut self.culling, format!("Frustum culling ({SECTORS} wedges)"));
        ui.label(format!(
            "Drawn: {} triangles, {} vertices ({}/{SECTORS} wedges)\nWhole tier: {} triangles, {} vertices",
            thousands(stats.triangles), thousands(stats.vertices), stats.sectors, thousands(stats.tier_triangles), thousands(stats.tier_vertices),
        ));
        let wavelengths: Vec<f32> = self.ocean.geometry_wavelengths().collect();
        egui::CollapsingHeader::new("This tier's rings").default_open(false).show(ui, |ui| {
            egui::Grid::new("rings").striped(true).show(ui, |ui| {
                ui.strong("cell");
                ui.strong("out to");
                ui.strong("triangles");
                ui.strong("waves it moves");
                ui.end_row();
                let mut inner = 0.0f32;
                for ring in tiers[stats.tier.min(tiers.len() - 1)].rings {
                    let (half, inside) = ((ring.outer_half_extent / ring.cell).round(), (inner / ring.cell).round());
                    let triangles = 2.0 * ((2.0 * half).powi(2) - (2.0 * inside).powi(2));
                    let moved = wavelengths.iter().filter(|&&wavelength| wavelength > GEOMETRY_MIN_CELLS * ring.cell).count();
                    ui.label(format!("{} m", ring.cell));
                    ui.label(distance(ring.outer_half_extent));
                    ui.label(thousands(triangles as usize));
                    ui.label(if moved == 0 { "flat".to_owned() } else { format!("{moved} of {}", wavelengths.len()) });
                    ui.end_row();
                    inner = ring.outer_half_extent;
                }
            });
        });

        ui.separator();
        ui.heading("Waves");
        let mut settings = self.settings.clone();
        ui.add(egui::Slider::new(&mut settings.wind_speed, 2.0..=20.0).text("wind m/s"));
        ui.add(egui::Slider::new(&mut settings.geometry_waves, 0..=MAX_WAVES).text("geometry waves"));
        ui.checkbox(&mut settings.flat, "Flat water (no waves at all)");
        ui.checkbox(&mut settings.textured_ripples, "Textured ripples (the ripple map instead of the detail waves)");
        ui.add_enabled_ui(settings.textured_ripples, |ui| {
            ui.horizontal(|ui| {
                ui.label("ripple layers");
                ui.radio_value(&mut settings.ripple_layers, 1, "1");
                ui.radio_value(&mut settings.ripple_layers, 2, "2");
                ui.checkbox(&mut settings.ripple_stochastic, "stochastic tiling (3 reads a layer, no repeat)");
            });
            ui.add(egui::Slider::new(&mut settings.detail_strength, 0.0..=1.5).text("ripple strength"));
            ui.add(egui::Slider::new(&mut settings.ripple_scale, 0.5..=10.0).text("ripple size (x)"));
            ui.add(egui::Slider::new(&mut settings.far_variation, 0.0..=1.5).text("far sea variation"));
            ui.add(egui::Slider::new(&mut settings.far_start, 0.0..=2_000.0).text("far look from (m)"));
            ui.add(egui::Slider::new(&mut settings.far_end, 10.0..=5_000.0).logarithmic(true).text("far look fully at (m)"));
        });
        ui.add(egui::Slider::new(&mut settings.depth_tint, 0.0..=2.0).text("depth color patches"));
        ui.add_enabled(!settings.textured_ripples, egui::Slider::new(&mut settings.detail_waves, 0..=MAX_WAVES).text("detail waves"));
        settings.detail_waves = settings.detail_waves.min(MAX_WAVES - settings.geometry_waves);
        ui.horizontal(|ui| {
            if ui.button("Old counts (40 + 32)").clicked() {
                settings.geometry_waves = 40;
                settings.detail_waves = 32;
            }
            if ui.button("Game defaults").clicked() {
                let defaults = OceanSettings::default();
                settings.geometry_waves = defaults.geometry_waves;
                settings.detail_waves = defaults.detail_waves;
                settings.wind_speed = defaults.wind_speed;
            }
        });

        ui.separator();
        ui.heading("Per pixel");
        ui.add(egui::Slider::new(&mut settings.pixel_wave_cap, 0..=MAX_WAVES).text("max waves a pixel sums"));
        ui.add(egui::Slider::new(&mut settings.pixel_full_pixels, 2.0..=32.0).text("min wave size on screen (px)"));
        ui.add(egui::Slider::new(&mut settings.altitude_detail, 0.0..=0.1).text("detail lost with altitude"));
        let shortest = self.ocean.shortest_wave_at(stats.altitude + self.ocean.sea_level());
        ui.small(if shortest > 0.0 { format!("At this height, waves shorter than {shortest:.1} m are dropped.") } else { "At this height, nothing is dropped for altitude.".to_owned() });
        ui.small("A pixel sums the longest waves first and stops at the first one too short to see there, or at the max.");
        if settings.wind_speed != self.settings.wind_speed
            || settings.geometry_waves != self.settings.geometry_waves
            || settings.detail_waves != self.settings.detail_waves
            || settings.pixel_wave_cap != self.settings.pixel_wave_cap
            || settings.pixel_full_pixels != self.settings.pixel_full_pixels
            || settings.textured_ripples != self.settings.textured_ripples
            || settings.flat != self.settings.flat
            || settings.altitude_detail != self.settings.altitude_detail
            || settings.ripple_layers != self.settings.ripple_layers
            || settings.ripple_stochastic != self.settings.ripple_stochastic
            || settings.far_variation != self.settings.far_variation
            || settings.ripple_scale != self.settings.ripple_scale
            || settings.detail_strength != self.settings.detail_strength
            || settings.far_start != self.settings.far_start
            || settings.far_end != self.settings.far_end
            || settings.depth_tint != self.settings.depth_tint
        {
            self.ocean.set_settings(settings.clone());
            self.settings = settings;
        }

        ui.separator();
        ui.heading("Pipeline");
        ui.checkbox(&mut self.alpha_blend, "Alpha blending (ocean.ron's blend: Alpha)");
        if ui.checkbox(&mut self.depth_copy, "Depth copy + shore foam").changed() {
            app.renderer.frame_plan = frame_plan(self.depth_copy);
        }
        if let Some(problem) = &self.problem {
            ui.colored_label(egui::Color32::LIGHT_RED, problem);
        }

        ui.separator();
        ui.heading("View");
        let before = self.debug_view;
        ui.horizontal(|ui| {
            ui.radio_value(&mut self.debug_view, OceanDebugView::Normal, "Normal");
            ui.radio_value(&mut self.debug_view, OceanDebugView::WaveCost, "Waves per pixel");
            ui.radio_value(&mut self.debug_view, OceanDebugView::MeshCells, "Mesh cells");
        });
        if self.debug_view != before {
            self.ocean.set_debug_view(self.debug_view);
        }
        match self.debug_view {
            OceanDebugView::WaveCost => { ui.small(format!("Blue: few waves  ·  green: ~20  ·  yellow: ~50  ·  red: {MAX_WAVES}")); }
            OceanDebugView::MeshCells => { ui.small("Each ring tinted by its cell size, cell edges dark. The mesh follows the camera."); }
            OceanDebugView::Normal => {}
        }
    }
}

impl SceneBehaviour for GameLogic {
    fn on_spawn(&mut self, scene: &mut Scene, app: &mut App) {
        scene.cameras.select_camera(CAMERA);
        app.renderer.frame_plan = frame_plan(self.depth_copy);
        app.renderer.set_vsync(self.vsync);
        app.window.set_cursor_locked(false);
        app.debug_ui.visible = true;
    }

    fn update(&mut self, scene: &mut Scene, app: &mut App, _ctx: &mut FrameContext) {
        let dt = app.time.unscaled_delta_time;
        self.frame_ms = if self.frame_ms <= 0.0 { dt * 1000.0 } else { self.frame_ms + (dt * 1000.0 - self.frame_ms) * 0.05 };

        let ui = &app.debug_ui.ctx;
        let ui_busy = app.debug_ui.visible && (ui.is_pointer_over_area() || ui.wants_keyboard_input() || ui.is_using_pointer());
        if app.input.mouse_right_button_down() && !ui_busy {
            let sensitivity = app.input.mouse_sensitivity();
            self.yaw += (app.input.mouse_rel_x() as f32 * sensitivity.0).to_radians();
            self.pitch = (self.pitch - (app.input.mouse_rel_y() as f32 * sensitivity.1).to_radians()).clamp(-89f32.to_radians(), 89f32.to_radians());
            self.yaw = (self.yaw + PI).rem_euclid(TAU) - PI;
        }
        let forward = forward_of(self.yaw, self.pitch);
        let up = Vector3::y();
        let right = forward.cross(&up).normalize();
        let mut direction = Vector3::zeros();
        if !app.debug_ui.ctx.wants_keyboard_input() {
            for (action, along) in [("debug_cam_forward", forward), ("debug_cam_back", -forward), ("debug_cam_right", right), ("debug_cam_left", -right), ("debug_cam_up", up), ("debug_cam_down", -up)] {
                if app.input.is_action_pressed(action) {
                    direction += along;
                }
            }
        }
        let speed = self.fly_speed * if app.input.is_action_pressed("debug_cam_sprint") { 10.0 } else { 1.0 };
        if let Some(transform) = scene.get_node_mut(CAMERA).and_then(|node| node.transform_3d_mut()) {
            transform.rotation = looking_along(forward);
            if let Some(direction) = direction.try_normalize(1e-6) {
                transform.position += direction * speed * dt;
            }
        }
    }

    fn after_cameras(&mut self, scene: &mut Scene, app: &mut App) {
        let shader = self.shader_name(app);
        let active = scene.cameras.active();
        let camera = active.camera.position().coords;
        let planes = side_planes(&(active.projection.calc_matrix() * active.camera.calc_matrix()));

        let tiers = self.layout.tiers();
        let tier = self.forced_tier.unwrap_or_else(|| mesh_data::ocean_tier_for(tiers, camera.y)).min(tiers.len() - 1);
        let Some(meshes) = self.meshes.get(&self.layout).and_then(|tiers| tiers.get(tier)) else { return };
        // Everywhere the waves can put the surface, around the camera.
        let (sideways, vertical) = self.ocean.wave_reach();
        let sea = self.ocean.sea_level();
        let heights = (sea - vertical - 1.0 - camera.y, sea + vertical + 1.0 - camera.y);
        let mut stats = Stats { tier, tier_triangles: meshes.triangles, tier_vertices: meshes.vertices, altitude: camera.y - sea, ..Default::default() };

        for (index, sector) in meshes.sectors.iter().enumerate() {
            let visible = !self.culling || sector_visible(&sector.hull, &planes, heights, sideways + 1.0);
            if visible {
                stats.sectors += 1;
                stats.triangles += sector.triangles;
                stats.vertices += sector.vertices;
            }
            let Some(node) = scene.get_node_mut(&sector_node(index)) else { continue };
            // Under the camera (its XZ) - the hulls are around the mesh's
            // center, so they're camera-relative too.
            if let Some(transform) = node.transform_3d_mut() {
                transform.position.x = camera.x;
                transform.position.z = camera.z;
            }
            let Some(mesh) = node.get_property_mut::<Mesh>() else { continue };
            if !matches!(&mesh.shape, MeshShape::Custom(name) if *name == sector.name) {
                mesh.shape = MeshShape::Custom(sector.name.clone());
            }
            mesh.visible = visible;
            if mesh.material.as_ref().map(|material| material.shader.as_str()) != Some(shader.as_str()) {
                mesh.material = Some(Material::shader(shader.clone()));
            }
        }
        self.stats = stats;

        let names: Vec<String> = (0..SECTORS).map(sector_node).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        self.ocean.update_nodes(scene, &names, app.time.delta_time);
    }

    fn debug_ui(&mut self, ctx: &egui::Context, scene: &mut Scene, app: &mut App) {
        egui::Window::new("Ocean lab")
            .default_pos(egui::pos2(16.0, 16.0))
            .default_width(360.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.controls(ui, scene, app));
            });
    }
}

fn sector_node(index: usize) -> String {
    format!("lab_sea_{index}")
}

/// The way a camera at `yaw`/`pitch` (radians - see the engine's
/// Camera::yaw) faces.
fn forward_of(yaw: f32, pitch: f32) -> Vector3<f32> {
    Vector3::new(yaw.cos() * pitch.cos(), pitch.sin(), yaw.sin() * pitch.cos())
}

/// The frame plan with the depth copy (the game's - see
/// frame_plans::standard) or without.
fn frame_plan(depth_copy: bool) -> FramePlan {
    if depth_copy {
        crate::game::frame_plans::standard()
    } else {
        FramePlan::engine_default().insert_after_scene("opaque", |steps| steps.scene("water"))
    }
}

/// ocean.wgsl, reading the depth copy or not (its `scene_depth` returning
/// the far plane - no shore foam).
fn variant_source(wgsl: &Result<String, String>, depth_copy: bool) -> Result<String, String> {
    let wgsl = wgsl.as_ref().map_err(Clone::clone)?;
    if depth_copy {
        return Ok(wgsl.clone());
    }
    if !wgsl.contains(DEPTH_READ) {
        return Err(format!("ocean.wgsl has no `{DEPTH_READ}` to take out - see its scene_depth"));
    }
    Ok(wgsl.replace(DEPTH_READ, "return 0.0;"))
}

/// ocean.ron with the lab's blend and depth read.
fn variant_settings(alpha_blend: bool, depth_copy: bool) -> String {
    format!(
        "(kind: Raw, pass: \"water\", blend: {}, cull: Back, depth_write: true, reads: [{}], textures: {{ \"ripples\": \"{}\" }})",
        if alpha_blend { "Alpha" } else { "None" },
        if depth_copy { "Depth" } else { "" },
        crate::game::ocean::RIPPLE_MAP,
    )
}

/// Every tier of `layout`, each cut into SECTORS wedges registered as custom
/// meshes ("OceanLab/<layout>#tier<n>/sector<m>").
fn build_layout(app: &mut App, layout: Layout) -> Vec<TierMeshes> {
    layout.tiers().iter().enumerate().map(|(tier_index, tier)| {
        let (vertices, indices) = mesh_data::build_water_rings_mesh(tier.rings);
        let (triangles, vertex_count) = (indices.len() / 3, vertices.len());
        let sectors = split_into_sectors(&vertices, &indices).into_iter().enumerate().map(|(index, (vertices, indices))| {
            let name = format!("OceanLab/{}#tier{tier_index}/sector{index}", layout.key());
            let hull = convex_hull(vertices.iter().flat_map(reach_points));
            let mesh = SectorMesh { name: name.clone(), triangles: indices.len() / 3, vertices: vertices.len(), hull };
            // Already there when the lab is opened again - the same mesh.
            let _ = app.assets.add_mesh(name, vertices, indices);
            mesh
        }).collect();
        TierMeshes { sectors, triangles, vertices: vertex_count }
    }).collect()
}

/// Where `vertex` can be drawn (mesh XZ): where it sits, and fully slid onto
/// the next ring's grid by the ring morph (ocean.wgsl's vs_main) - it's
/// always between the two.
fn reach_points(vertex: &ModelVertex) -> [[f32; 2]; 2] {
    let (x, z, next_cell) = (vertex.position[0], vertex.position[2], vertex.normal[0]);
    [[x, z], [(x / next_cell).floor() * next_cell, (z / next_cell).floor() * next_cell]]
}

/// Which wedge around the center the point (x, z) is in.
fn sector_of(x: f32, z: f32) -> usize {
    let angle = z.atan2(x) + PI;
    ((angle / TAU * SECTORS as f32) as usize).min(SECTORS - 1)
}

/// The mesh cut into SECTORS wedges around its center - each triangle into
/// the one its middle is in, with the vertices it uses.
fn split_into_sectors(vertices: &[ModelVertex], indices: &[u32]) -> Vec<(Vec<ModelVertex>, Vec<u32>)> {
    let mut sectors: Vec<(Vec<ModelVertex>, Vec<u32>, HashMap<u32, u32>)> = (0..SECTORS).map(|_| (Vec::new(), Vec::new(), HashMap::new())).collect();
    for triangle in indices.chunks_exact(3) {
        let (x, z) = triangle.iter().fold((0.0, 0.0), |(x, z), &index| (x + vertices[index as usize].position[0], z + vertices[index as usize].position[2]));
        let (sector_vertices, sector_indices, remap) = &mut sectors[sector_of(x, z)];
        for &index in triangle {
            let local = *remap.entry(index).or_insert_with(|| {
                sector_vertices.push(vertices[index as usize]);
                (sector_vertices.len() - 1) as u32
            });
            sector_indices.push(local);
        }
    }
    sectors.into_iter().map(|(vertices, indices, _)| (vertices, indices)).collect()
}

/// The convex hull of `points` (monotone chain, in f64 - the far rings'
/// points are millions of meters out).
fn convex_hull(points: impl Iterator<Item = [f32; 2]>) -> Vec<[f32; 2]> {
    let mut points: Vec<[f64; 2]> = points.map(|[x, z]| [x as f64, z as f64]).collect();
    points.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    points.dedup();
    if points.len() < 3 {
        return points.iter().map(|[x, z]| [*x as f32, *z as f32]).collect();
    }
    let cross = |o: [f64; 2], a: [f64; 2], b: [f64; 2]| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    let mut hull: Vec<[f64; 2]> = Vec::with_capacity(points.len() * 2);
    for &point in &points {
        while hull.len() >= 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], point) <= 0.0 {
            hull.pop();
        }
        hull.push(point);
    }
    let lower = hull.len() + 1;
    for &point in points.iter().rev().skip(1) {
        while hull.len() >= lower && cross(hull[hull.len() - 2], hull[hull.len() - 1], point) <= 0.0 {
            hull.pop();
        }
        hull.push(point);
    }
    hull.pop();
    hull.iter().map(|[x, z]| [*x as f32, *z as f32]).collect()
}

/// The view's left, right, bottom and top planes, from a camera-relative
/// view-projection matrix - (normal, offset), inside where n.p + w >= 0.
fn side_planes(view_proj: &Matrix4<f32>) -> [Vector4<f32>; 4] {
    let row = |index: usize| -> Vector4<f32> { view_proj.row(index).transpose() };
    let (x, y, w) = (row(0), row(1), row(3));
    [w + x, w - x, w + y, w - y]
}

/// Whether any of a wedge can be on screen: its hull (camera-relative XZ),
/// anywhere from `heights.0` to `heights.1` (camera-relative Y), each point
/// also up to `margin` sideways (the waves' push). Hidden only when all of
/// it is outside one of the planes - so never wrongly, sometimes drawn when
/// it needn't be.
fn sector_visible(hull: &[[f32; 2]], planes: &[Vector4<f32>; 4], heights: (f32, f32), margin: f32) -> bool {
    planes.iter().all(|plane| {
        let across = hull.iter().map(|point| plane.x * point[0] + plane.z * point[1]).fold(f32::NEG_INFINITY, f32::max);
        let up = (plane.y * heights.0).max(plane.y * heights.1);
        across + up + plane.w + margin * (plane.x * plane.x + plane.z * plane.z).sqrt() >= 0.0
    })
}

fn thousands(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

fn distance(meters: f32) -> String {
    if meters >= 1_000.0 { format!("{:.0} km", meters / 1_000.0) } else { format!("{meters:.0} m") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validate(source: &str) {
        let module = wgpu::naga::front::wgsl::parse_str(source).unwrap_or_else(|error| panic!("{}", error.emit_to_string(source)));
        wgpu::naga::valid::Validator::new(wgpu::naga::valid::ValidationFlags::all(), wgpu::naga::valid::Capabilities::all())
            .validate(&module)
            .unwrap_or_else(|error| panic!("{error:?}"));
    }

    #[test]
    fn the_ocean_shader_and_its_lab_variants_are_valid() {
        let wgsl = std::fs::read_to_string(OCEAN_WGSL).map_err(|error| error.to_string());
        validate(&variant_source(&wgsl, true).unwrap());
        validate(&variant_source(&wgsl, false).unwrap());
    }

    #[test]
    fn sectors_keep_every_triangle() {
        for tier in crate::OCEAN_TIERS {
            let (vertices, indices) = mesh_data::build_water_rings_mesh(tier.rings);
            let sectors = split_into_sectors(&vertices, &indices);
            assert_eq!(sectors.iter().map(|(_, indices)| indices.len()).sum::<usize>(), indices.len());
        }
    }

    #[test]
    fn a_wedge_behind_the_camera_is_culled_and_one_ahead_is_not() {
        // Looking along +X from 50 m up, level.
        let camera = pankarta_software::rendering::camera::camera::Camera::new(Transform3D::default());
        let projection = pankarta_software::rendering::camera::projection::Projection::new(1920, 1080, 70.0, 0.1, 4_000_000.0);
        let planes = side_planes(&(projection.calc_matrix() * camera.calc_matrix()));
        let ahead = convex_hull([[0.0, 0.0], [10_000.0, -1_000.0], [10_000.0, 1_000.0]].into_iter());
        let behind = convex_hull([[-1.0, 0.0], [-10_000.0, -1_000.0], [-10_000.0, 1_000.0]].into_iter());
        assert!(sector_visible(&ahead, &planes, (-51.0, -49.0), 1.0));
        assert!(!sector_visible(&behind, &planes, (-51.0, -49.0), 1.0));
    }

    /// ocean.wgsl's ring morph on the CPU: where vs_main draws `vertex`
    /// (mesh XZ, before the waves), and how far it's morphed.
    fn morph(vertex: &ModelVertex) -> ([f64; 2], f64) {
        const RING_MORPH_START: f64 = 0.72;
        let (x, z) = (vertex.position[0] as f64, vertex.position[2] as f64);
        let (outer, next) = (vertex.tex_coords[1] as f64, vertex.normal[0] as f64);
        let anchor = [(x / next).floor() * next, (z / next).floor() * next];
        let snap = [x - anchor[0], z - anchor[1]];
        let t = ((anchor[0].hypot(anchor[1]) / outer - RING_MORPH_START) / (1.0 - RING_MORPH_START)).clamp(0.0, 1.0);
        let amount = t * t * (3.0 - 2.0 * t);
        ([x - snap[0] * amount, z - snap[1] * amount], amount)
    }

    #[test]
    fn the_ring_morph_never_folds_a_triangle_and_rings_meet() {
        for tiers in [crate::OCEAN_TIERS, ORIGINAL_TIERS] {
            for tier in tiers {
                let (vertices, indices) = mesh_data::build_water_rings_mesh(tier.rings);
                for triangle in indices.chunks_exact(3) {
                    let triangle: [u32; 3] = triangle.try_into().unwrap();
                    let area = |points: [[f64; 2]; 3]| (points[1][0] - points[0][0]) * (points[2][1] - points[0][1]) - (points[1][1] - points[0][1]) * (points[2][0] - points[0][0]);
                    let at = |index: u32| vertices[index as usize].position;
                    let before = area(triangle.map(|index| [at(index)[0] as f64, at(index)[2] as f64]));
                    let after = area(triangle.map(|index| morph(&vertices[index as usize]).0));
                    assert!(after * before.signum() >= -1e-6 * before.abs(), "a triangle folds over: {:?}", triangle.map(at));
                }
                // A ring's outer edge is fully morphed, the next one's inner
                // edge not at all - the two meet vertex for vertex.
                let mut inner = 0.0f32;
                for ring in tier.rings {
                    for vertex in vertices.iter().filter(|vertex| vertex.tex_coords[0] == ring.cell) {
                        let edge = vertex.position[0].abs().max(vertex.position[2].abs());
                        let (_, amount) = morph(vertex);
                        if edge == ring.outer_half_extent {
                            assert_eq!(amount, 1.0, "outer edge of the {} m ring isn't fully morphed", ring.cell);
                        }
                        if inner > 0.0 && edge == inner {
                            assert_eq!(amount, 0.0, "inner edge of the {} m ring is morphed", ring.cell);
                        }
                    }
                    inner = ring.outer_half_extent;
                }
            }
        }
    }

    #[test]
    fn the_compact_layout_is_much_lighter() {
        let triangles = |tiers: &[WaterTier]| mesh_data::build_water_rings_mesh(tiers[0].rings).1.len() / 3;
        let (compact, original) = (triangles(crate::OCEAN_TIERS), triangles(ORIGINAL_TIERS));
        println!("tier 0: {compact} triangles (was {original})");
        assert!(compact * 5 < original);
    }
}
