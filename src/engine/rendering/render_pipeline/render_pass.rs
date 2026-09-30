use std::collections::HashSet;

use wgpu::RenderPassDepthStencilAttachment;

use nalgebra::Vector3;

use crate::app::App;
use crate::engine::input::input;
use crate::engine::primitive::manual_vertex::ManualVertex;
use crate::engine::rendering::models::model::DrawModel;
use crate::game::game_settings::{COCKPIT_VIEW_PITCH_MAX, GAME_SETTINGS, HEAD_G_REACTION_MAX};
use crate::game::scenes::play::camera::camera::Camera;
use crate::game::scenes::play::plane::aero_spec::{AeroSpec, SurfaceKind};
use crate::game::scenes::play::plane::aircraft_spec::AircraftSpec;
use crate::game::scenes::play::plane::physics::rolling_rate::{max_roll_rate_deg_s, RollRateParams};
use crate::game::scenes::play::plane::messages::AircraftState;
use crate::game::scenes::play::plane::plane::Plane;
use crate::game::scenes::play::plane::aircraft_physics::KINEMATIC_ROLL;
use crate::game::scenes::play::plane::physics::roll_flcs::ROLL_FLCS;
use crate::game::scenes::play::plane::physics::yaw_flcs::YAW_FLCS;
use crate::game::scenes::play::plane::physics::f16_pitch::F16PitchModel;
use crate::game::scenes::play::plane::physics::wings::wing_manager::aileron_for_side;

/// One F7 "Flight Rates" sample (see `App::aero_debug_trail`) - recorded
/// every frame the overlay is up, by `Plane::record_aero_debug_trail`.
#[derive(Clone, Copy)]
pub struct AeroSample {
    pub speed_kt: f32,
    /// FlightData::mach - airspeed over sea-level speed of sound.
    pub mach: f32,
    /// Angle of attack, deg (+ = nose above the flight path).
    pub aoa_deg: f32,
    pub altitude_m: f32,
    /// Body rates, deg/s - roll + = rolling right, pitch + = nose up, yaw
    /// as FlightData::yaw_rate.
    pub roll_rate: f32,
    pub pitch_rate: f32,
    pub yaw_rate: f32,
    /// Vertical speed, m/s (+ = climbing).
    pub climb_rate: f32,
    /// Stick/pedals, -1..1 - aileron (+ = right), pitch (+ = pull), rudder.
    pub aileron: f32,
    pub pitch_stick: f32,
    pub rudder: f32,
    /// FlightData::up_alignment - how much of gravity acts along the jet's
    /// own up axis (1 = level flight).
    pub up_alignment: f32,
}

/// The F7 "Flight Rates" window's tabs - one measured value each, plotted
/// against airspeed, with a theoretical curve when one exists
/// (`chart_max`).
#[derive(Clone, Copy, PartialEq)]
pub enum RateChart {
    Roll,
    Pitch,
    Yaw,
    Climb,
}

impl RateChart {
    pub const ALL: [RateChart; 4] = [RateChart::Roll, RateChart::Pitch, RateChart::Yaw, RateChart::Climb];

    fn label(self) -> &'static str {
        match self {
            RateChart::Roll => "Roll",
            RateChart::Pitch => "Pitch",
            RateChart::Yaw => "Yaw",
            RateChart::Climb => "Climb",
        }
    }

    /// The chart's fixed y axis for this rate - wide enough for the F-16.
    fn y_range(self) -> (f64, f64) {
        match self {
            RateChart::Roll => (0.0, 450.0),
            RateChart::Pitch => (0.0, 40.0),
            RateChart::Yaw => (0.0, 40.0),
            RateChart::Climb => (-300.0, 300.0),
        }
    }

    fn unit(self) -> &'static str {
        match self {
            RateChart::Climb => "m/s",
            _ => "deg/s",
        }
    }

    /// The measured value plotted. Roll/pitch/yaw as magnitudes - the charts
    /// are "how fast can it go at this speed", not which way; climb signed.
    fn measured(self, sample: &AeroSample) -> f32 {
        match self {
            RateChart::Roll => sample.roll_rate.abs(),
            RateChart::Pitch => sample.pitch_rate.abs(),
            RateChart::Yaw => sample.yaw_rate.abs(),
            RateChart::Climb => sample.climb_rate,
        }
    }

    /// The control that commands this rate, -1..1 - None for climb.
    fn stick(self, sample: &AeroSample) -> Option<f32> {
        match self {
            RateChart::Roll => Some(sample.aileron),
            RateChart::Pitch => Some(sample.pitch_stick),
            RateChart::Yaw => Some(sample.rudder),
            RateChart::Climb => None,
        }
    }

    /// The theoretical max at full control, or None if this rate has no
    /// chart yet. Add a new chart here (e.g. a pitch-rate curve) and the
    /// window plots it and the Copy button compares against it.
    fn chart_max(self, speed_kt: f32, aoa_deg: f32, altitude_m: f32) -> Option<f32> {
        match self {
            RateChart::Roll => Some(max_roll_rate_deg_s(speed_kt * 0.514_444, altitude_m, aoa_deg.abs(), &RollRateParams::default())),
            // Nose-up max - the pitch model has no AoA input.
            RateChart::Pitch => Some(f16_pitch_model().max_pitch_rate_deg_s(speed_kt as f64 * 0.514_444, altitude_m as f64) as f32),
            RateChart::Yaw | RateChart::Climb => None,
        }
    }

    /// What the chart says this sample should read: its max at the sample's
    /// own speed/AoA/altitude, times how much stick was in.
    fn charted(self, sample: &AeroSample) -> Option<f32> {
        // Pitch: the model's own stick -> rate, so pushing is measured
        // against its (weaker) nose-down max, not the nose-up one.
        if self == RateChart::Pitch {
            let tas_ms = sample.speed_kt as f64 * 0.514_444;
            return Some(f16_pitch_model().commanded_pitch_rate_deg_s(sample.pitch_stick as f64, tas_ms, sample.altitude_m as f64).abs() as f32);
        }
        let max = self.chart_max(sample.speed_kt, sample.aoa_deg, sample.altitude_m)?;
        Some(match self.stick(sample) {
            Some(stick) => max * stick.abs(),
            None => max,
        })
    }

    /// The pitch chart with gravity put back in. The pitch model is lift
    /// only (n*g/V - see physics::f16_pitch), but the jet's real pitch rate
    /// also has gravity bending its flight path: g * up_alignment / V, nose
    /// down. So in a level pull the real rate should read that much under
    /// the chart - ~7.5 deg/s at 145 kt, ~2.6 at 420 kt. None for every
    /// other rate. Meant for full stick: the model maps a centered stick to
    /// no lift, where the fly-by-wire holds 1 g.
    fn gravity_adjusted(self, sample: &AeroSample) -> Option<f32> {
        if self != RateChart::Pitch {
            return None;
        }
        let tas_ms = sample.speed_kt as f64 * 0.514_444;
        if tas_ms < 1.0 {
            return None;
        }
        let lift_only = f16_pitch_model().commanded_pitch_rate_deg_s(sample.pitch_stick as f64, tas_ms, sample.altitude_m as f64);
        Some((lift_only - gravity_pitch_rate_deg_s(tas_ms, sample.up_alignment as f64)).abs() as f32)
    }

    /// The gravity-adjusted chart as a line: full nose-up stick, level,
    /// sea level. Floored at 0 - below that the jet can't hold level.
    fn gravity_curve(self, mach: f32) -> Option<f32> {
        if self != RateChart::Pitch {
            return None;
        }
        let tas_ms = mach as f64 * 340.29;
        if tas_ms < 1.0 {
            return Some(0.0);
        }
        let lift_only = f16_pitch_model().max_pitch_rate_deg_s(tas_ms, 0.0);
        Some((lift_only - gravity_pitch_rate_deg_s(tas_ms, 1.0)).max(0.0) as f32)
    }

    /// `samples` (oldest first) as a text table - up to COPY_ROWS of them,
    /// spread across the whole trail - each with the chart's value next to
    /// the measured one when there's a chart.
    fn copy_text(self, samples: &[AeroSample]) -> String {
        const COPY_ROWS: usize = 40;
        let unit = self.unit();
        let has_chart = samples.iter().any(|s| self.charted(s).is_some());
        let has_stick = self.stick(&ZERO_SAMPLE).is_some();
        let stick_header = if has_stick { " stick |" } else { "" };

        let mut text = format!("{} rate - chart vs real (F7 Flight Rates), {} samples\n", self.label(), samples.len().min(COPY_ROWS));
        // Only pitch has a gravity-adjusted chart (see gravity_adjusted).
        let has_gravity = self == RateChart::Pitch;
        if has_chart {
            text.push_str("chart = its max at that speed/AoA (sea-level-corrected by altitude) x stick\n");
            if has_gravity {
                text.push_str("chart-g = the chart with gravity put back in (g * up / V, nose down) - compare at full stick\n");
                text.push_str(&format!("mach | speed kt | AoA deg |{stick_header} real {unit} | chart {unit} | real-chart | chart-g {unit} | real-(chart-g)\n"));
            } else {
                text.push_str(&format!("mach | speed kt | AoA deg |{stick_header} real {unit} | chart {unit} | real-chart\n"));
            }
        } else {
            text.push_str("(no chart for this rate yet - real values only)\n");
            text.push_str(&format!("mach | speed kt | AoA deg |{stick_header} real {unit}\n"));
        }

        let step = (samples.len() / COPY_ROWS).max(1);
        for sample in samples.iter().rev().step_by(step).take(COPY_ROWS).collect::<Vec<_>>().into_iter().rev() {
            let real = self.measured(sample);
            let stick = self.stick(sample).map(|s| format!(" {:+.2} |", s)).unwrap_or_default();
            let row = format!("{:4.2} | {:8.0} | {:7.1} |{} {:8.1}", sample.mach, sample.speed_kt, sample.aoa_deg, stick, real);
            match self.charted(sample) {
                Some(chart) => match self.gravity_adjusted(sample) {
                    Some(with_gravity) => text.push_str(&format!("{row} | {:8.1} | {:+8.1} | {:8.1} | {:+8.1}\n", chart, real - chart, with_gravity, real - with_gravity)),
                    None => text.push_str(&format!("{row} | {:8.1} | {:+8.1}\n", chart, real - chart)),
                },
                None => text.push_str(&format!("{row}\n")),
            }
        }
        text
    }
}

/// Sea-level speed of sound in knots - the same 340.29 m/s FlightData::mach
/// divides by, so Mach and knots convert the same way here as there.
const KNOTS_PER_MACH: f32 = 340.29 * 1.943_84;

/// The pitch-rate chart (physics::f16_pitch) - built once, since its lift
/// curve is set up on construction.
fn f16_pitch_model() -> &'static F16PitchModel {
    static MODEL: std::sync::OnceLock<F16PitchModel> = std::sync::OnceLock::new();
    MODEL.get_or_init(F16PitchModel::default)
}

/// How fast gravity turns the flight path nose-down in the pitch plane,
/// deg/s: g * up_alignment / V.
fn gravity_pitch_rate_deg_s(tas_ms: f64, up_alignment: f64) -> f64 {
    (9.806_65 * up_alignment / tas_ms.max(1.0)).to_degrees()
}

/// The Flight Rates chart's fixed x axis: Mach 0 to this.
const RATE_CHART_MAX_MACH: f64 = 2.0;

const ZERO_SAMPLE: AeroSample = AeroSample { speed_kt: 0.0, mach: 0.0, aoa_deg: 0.0, altitude_m: 0.0, roll_rate: 0.0, pitch_rate: 0.0, yaw_rate: 0.0, climb_rate: 0.0, aileron: 0.0, pitch_stick: 0.0, rudder: 0.0, up_alignment: 1.0 };

/// Which of the F7/F8 debug overlay's windows are open (see
/// `App::debug_windows`) - all of them to begin with.
#[derive(Clone)]
pub struct DebugWindows {
    pub camera_editor: bool,
    pub debug_info: bool,
    pub stick_input: bool,
    pub g_forces: bool,
    pub pitch_rate: bool,
    pub roll_authority: bool,
    pub wing_lift: bool,
    pub wing_surfaces: bool,
    /// The "Flight Rates" window's open tab.
    pub rate_chart: RateChart,
}

impl Default for DebugWindows {
    fn default() -> Self {
        Self { camera_editor: true, debug_info: true, stick_input: true, g_forces: true, pitch_rate: true, roll_authority: true, wing_lift: true, wing_surfaces: true, rate_chart: RateChart::Roll }
    }
}

impl DebugWindows {
    /// Every window's toggle, labelled, in the panel's order.
    fn entries(&mut self) -> [(&'static str, &mut bool); 8] {
        [
            ("Camera Editor", &mut self.camera_editor),
            ("Debug Info", &mut self.debug_info),
            ("Stick Input", &mut self.stick_input),
            ("G Forces", &mut self.g_forces),
            ("Pitch Rate", &mut self.pitch_rate),
            ("Flight Rates", &mut self.roll_authority),
            ("Wing Lift Forces", &mut self.wing_lift),
            ("Wing Surfaces", &mut self.wing_surfaces),
        ]
    }
}

fn color_attachment(view: &wgpu::TextureView, load: wgpu::LoadOp<wgpu::Color>) -> Option<wgpu::RenderPassColorAttachment> {
    Some(wgpu::RenderPassColorAttachment {
        view,
        resolve_target: None,
        ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
    })
}

fn depth_attachment(view: &wgpu::TextureView, load: wgpu::LoadOp<f32>) -> Option<RenderPassDepthStencilAttachment> {
    Some(RenderPassDepthStencilAttachment {
        view,
        depth_ops: Some(wgpu::Operations { load, store: wgpu::StoreOp::Store }),
        stencil_ops: None,
    })
}

impl App {
    // Distinct model refs currently in use, optionally skipping one instance key
    // (e.g. "sun", which has no drawable model of its own).
    fn distinct_model_refs(&self, exclude_key: Option<&str>) -> HashSet<String> {
        let Some(content) = self.scene_manager.content() else { return HashSet::new() };
        content.renderizable_instances.iter()
            .filter(|(key, _)| exclude_key != Some(key.as_str()))
            .map(|(_, renderizable)| renderizable.model_ref.clone())
            .collect()
    }

    // Renders into BlurRender's scene_color (an offscreen copy of the swapchain),
    // not the swapchain view itself - the UI pass needs something already-rendered
    // it can sample from for background_blur, and you can't read a texture that's
    // also the render target currently being written to. BlurRender::render blits
    // scene_color onto the real swapchain view right before the UI pass runs, so
    // this indirection is invisible for every scene not using background_blur.
    fn render_opaque_pass(&mut self, encoder: &mut wgpu::CommandEncoder) {
        let view = &self.renderer.blur.scene_color.view;
        // No real camera to render from (see SceneCameras::has_active_camera)
        // - skip every model/skybox draw below, but still just clear to
        // clear_color rather than hardcoding black here: that value is
        // already correct for every phase this can happen in (the active
        // scene's own SceneEnvironment, or - only before any scene has ever
        // reset - run_splash_screen's own configured background, see
        // self.clear_color's own doc comment) - hardcoding black would
        // silently override any of those instead of leaving them alone.
        // App::sync_no_camera_message puts the actual "Add a camera to the
        // scene" text up via the separate UI pass, which still runs normally
        // on top of whatever this clears to (and suppresses itself during
        // splash/loading - see its own comment).
        let has_camera = self.scene_manager.cameras().is_some_and(|c| c.has_active_camera());
        let clear_color = self.scene_manager.environment().map_or(self.clear_color, |e| e.clear_color);
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Opaque Render Pass"),
            color_attachments: &[color_attachment(view, wgpu::LoadOp::Clear(clear_color))],
            // Reversed-Z: clear to 0.0 ("infinitely far") instead of 1.0.
            depth_stencil_attachment: depth_attachment(&self.renderer.depth_render.texture.view, wgpu::LoadOp::Clear(0.0)),
            occlusion_query_set: None,
            timestamp_writes: None,
        });

        if !has_camera {
            return;
        }

        let skybox = self.scene_manager.environment().and_then(|e| e.skybox.as_ref()).or(self.skybox.as_ref());
        if let Some(skybox) = skybox {
            skybox.render(&mut render_pass, &self.camera_resources.bind_group);
        }

        // draw_model_instanced_from_list sets the pipeline itself, per mesh
        // (single- vs double-sided), so no set_pipeline here.
        for model_ref in self.distinct_model_refs(Some("sun")) {
            // Water-shaded models draw in their own later pass instead (see
            // render_water_pass) - they need a snapshot of this pass's own
            // depth output to sample from, so they can't be part of this
            // pass themselves.
            if self.water_shaded_models.contains(&model_ref) {
                continue;
            }
            if let Some(model_data) = self.game_models.get(&model_ref) {
                render_pass.set_vertex_buffer(1, model_data.instance_buffer.slice(..));
                render_pass.draw_model_instanced_from_list(
                    &model_data.model, 0..model_data.instance_count as u32,
                    &self.camera_resources.bind_group, &self.light.rendering_data.bind_group,
                    &"opaque".to_string(),
                    &self.render_pipeline, Some(&self.render_pipeline_double_sided), None,
                );
            }
        }
    }

    // Runs after render_opaque_pass (needs its depth output already snapshotted,
    // see DepthRender::snapshot_for_water) and before render_transparent_pass -
    // its own render pass rather than folded into render_opaque_pass's loop,
    // since a water-shaded model needs to sample a copy of the depth buffer
    // render_opaque_pass just finished writing, and wgpu doesn't allow reading a
    // texture that's also this same pass's own depth-stencil attachment (same
    // "can't read what you're writing" constraint BlurRender's scene_color
    // already works around for color). Draws with raw wgpu calls instead of the
    // shared draw_model_instanced_from_list/draw_mesh_instanced helpers other
    // models use, since those unconditionally bind each mesh's own *material* at
    // group 0 - here group 0 is the depth snapshot instead (see
    // WaterRenderData's own doc comment), constant for the whole pass rather
    // than rebound per mesh.
    fn render_water_pass(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if self.water_shaded_models.is_empty() {
            return;
        }
        if !self.scene_manager.cameras().is_some_and(|c| c.has_active_camera()) {
            return;
        }

        // Has to happen before begin_render_pass (copy_texture_to_texture isn't
        // valid mid-pass) and before this pass writes any depth of its own.
        self.renderer.depth_render.snapshot_for_water(encoder);

        let view = &self.renderer.blur.scene_color.view;
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Water Render Pass"),
            color_attachments: &[color_attachment(view, wgpu::LoadOp::Load)],
            depth_stencil_attachment: depth_attachment(&self.renderer.depth_render.texture.view, wgpu::LoadOp::Load),
            occlusion_query_set: None,
            timestamp_writes: None,
        });

        render_pass.set_pipeline(&self.water.render_pipeline);
        render_pass.set_bind_group(0, self.water.bind_group(), &[]);
        render_pass.set_bind_group(1, &self.camera_resources.bind_group, &[]);
        render_pass.set_bind_group(3, &self.light.rendering_data.bind_group, &[]);

        // Ocean culling (see WaterRenderData::visible_ranges): the camera's
        // view volume in world space, and fresh counters for the F3 panel.
        let camera_position = self.scene_manager.cameras().map(|cameras| cameras.active().camera.position().coords).unwrap_or_else(nalgebra::Vector3::zeros);
        let frustum = crate::engine::rendering::render_pipeline::water_renderer::WaterRenderData::world_frustum(&nalgebra::Matrix4::from(self.camera_resources.uniform.view_proj), camera_position);
        self.water.begin_culling_stats();

        for model_ref in self.distinct_model_refs(Some("sun")) {
            if !self.water_shaded_models.contains(&model_ref) {
                continue;
            }
            // Where this model's (single) instance sits in the world - tile
            // boxes are relative to it. With several instances there's no one
            // place to cull against, so those draw whole.
            let mut instances = self.scene_manager.content().into_iter()
                .flat_map(|content| content.renderizable_instances.values())
                .filter(|instance| instance.model_ref == model_ref);
            let mesh_position = match (instances.next(), instances.next()) {
                (Some(instance), None) => Some(instance.instance.transform.position),
                _ => None,
            };
            // See App::water_debug_hidden_models's own doc comment.
            if self.water_debug_view && self.water_debug_hidden_models.contains(&model_ref) {
                continue;
            }
            let Some(model_data) = self.game_models.get(&model_ref) else { continue };
            let Some(meshes) = model_data.model.mesh_lists.get("opaque") else { continue };

            render_pass.set_vertex_buffer(1, model_data.instance_buffer.slice(..));
            for mesh in meshes.values() {
                // Ocean meshes come in altitude tiers - only the current
                // one draws (see WaterRenderData::tier).
                if crate::resources::water_tier_of_mesh(&mesh.name).is_some_and(|tier| tier != self.water.tier()) {
                    continue;
                }
                render_pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                render_pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                render_pass.set_bind_group(2, &mesh.transform_bind_group, &[]);
                let visible = mesh_position.and_then(|position| self.water.visible_ranges(&mesh.name, frustum, position));
                match visible {
                    Some(ranges) => {
                        for range in ranges {
                            render_pass.draw_indexed(range, 0, 0..model_data.instance_count as u32);
                        }
                    }
                    None => render_pass.draw_indexed(0..mesh.num_elements, 0, 0..model_data.instance_count as u32),
                }
            }
        }
    }

    // Same scene_color target as render_opaque_pass above, and for the same reason.
    fn render_transparent_pass(&mut self, encoder: &mut wgpu::CommandEncoder) {
        // render_opaque_pass already cleared to black and drew nothing - see
        // its own comment on has_active_camera - nothing for this pass to add.
        if !self.scene_manager.cameras().is_some_and(|c| c.has_active_camera()) {
            return;
        }

        let view = &self.renderer.blur.scene_color.view;
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Transparent Render Pass"),
            color_attachments: &[color_attachment(view, wgpu::LoadOp::Load)],
            depth_stencil_attachment: depth_attachment(&self.renderer.depth_render.texture.view, wgpu::LoadOp::Load),
            occlusion_query_set: None,
            timestamp_writes: None,
        });

        // Pipeline is set per mesh inside draw_model_instanced_from_list.
        for model_ref in self.distinct_model_refs(None) {
            if let Some(model_data) = self.game_models.get(&model_ref) {
                render_pass.set_vertex_buffer(1, model_data.instance_buffer.slice(..));
                render_pass.draw_model_instanced_from_list(
                    &model_data.model, 0..model_data.instance_count as u32,
                    &self.camera_resources.bind_group, &self.light.rendering_data.bind_group,
                    &"transparent".to_string(),
                    &self.render_pipeline_transparent, None, Some(&self.render_pipeline_transparent_front_cull),
                );
            }
        }
    }

    // Particles and trails (see ParticleRenderer) - after everything solid,
    // the water and transparent models, so they blend over all of it. The
    // update (compute) pass runs first, then the scene's depth - now
    // including the water - is snapshotted again into foam_depth_copy for
    // their soft fading (the water pass is done reading its own copy by then).
    fn render_particle_pass(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if !self.particles.has_work() || !self.scene_manager.cameras().is_some_and(|c| c.has_active_camera()) {
            return;
        }
        self.particles.simulate(encoder);
        self.renderer.depth_render.snapshot_for_water(encoder);

        let view = &self.renderer.blur.scene_color.view;
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Particle Render Pass"),
            color_attachments: &[color_attachment(view, wgpu::LoadOp::Load)],
            depth_stencil_attachment: depth_attachment(&self.renderer.depth_render.texture.view, wgpu::LoadOp::Load),
            occlusion_query_set: None,
            timestamp_writes: None,
        });
        self.particles.draw(&mut render_pass, &self.camera_resources.bind_group);
    }

    // Physics debug lines and UI/text share this pass since they're both drawn on top,
    // without a depth buffer, gated on there being UI geometry to draw at all.
    fn render_ui_pass(&mut self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        if self.ui.ui_rendering.num_indices == 0 && self.ui.image_draws.is_empty() {
            return;
        }

        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("UI Render Pass"),
            color_attachments: &[color_attachment(view, wgpu::LoadOp::Load)],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes: None,
        });

        render_pass.set_pipeline(&self.render_physics.render_pipeline);
        render_pass.set_bind_group(0, &self.render_physics.bind_group, &[]);
        render_pass.set_bind_group(1, &self.camera_resources.bind_group, &[]);

        if !self.show_depth_map && self.render_physics.visible {
            self.render_physics_debug_lines(&mut render_pass);
        }

        // Main-pass quads, then main-pass text, THEN always_on_top's quads and
        // text (see UiRendering::main_index_count / TextRendering::
        // text_renderer_on_top's own doc comments) - text is a wholly separate
        // draw pass from these quads (glyphon, not the ui_pipeline below), so
        // without this split every node's text ended up in one shared final
        // pass with no z-order relative to a quad drawn after it: a modal's
        // opaque background quad could correctly land above a panel behind it,
        // while that same panel's *text* still rendered on top of the modal
        // regardless, since all text - modal's and panel's alike - drew after
        // all quads unconditionally.
        let main_index_count = self.ui.ui_rendering.main_index_count;
        if main_index_count > 0 {
            render_pass.set_pipeline(&self.ui.ui_pipeline);
            render_pass.set_bind_group(0, &self.ui.blur_bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.ui.ui_rendering.vertex_buffer.slice(..));
            render_pass.set_index_buffer(self.ui.ui_rendering.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            render_pass.draw_indexed(0..main_index_count, 0, 0..1);
        }

        // One draw call per distinct image - see Ui::build_image_draws. Not
        // itself split into a main/on-top pair (nothing always_on_top uses
        // Image content today), so it renders as one batch here, same relative
        // position (after the main pass's quads, before the on-top pass) it
        // always has.
        if !self.ui.image_draws.is_empty() {
            render_pass.set_pipeline(&self.ui.image_pipeline);
            for draw in &self.ui.image_draws {
                if let Some(image) = self.ui.images.get(&draw.path) {
                    render_pass.set_bind_group(0, &image.bind_group, &[]);
                    render_pass.set_vertex_buffer(0, draw.vertex_buffer.slice(..));
                    render_pass.set_index_buffer(draw.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                    render_pass.draw_indexed(0..draw.num_indices, 0, 0..1);
                }
            }
        }

        // Render text (text renderer handles empty content gracefully)
        self.ui.text.text_renderer.render(&self.ui.text.text_atlas, &self.renderer.glyphon.viewport, &mut render_pass).unwrap();

        if self.ui.ui_rendering.num_indices > main_index_count {
            render_pass.set_pipeline(&self.ui.ui_pipeline);
            render_pass.set_bind_group(0, &self.ui.blur_bind_group, &[]);
            render_pass.set_vertex_buffer(0, self.ui.ui_rendering.vertex_buffer.slice(..));
            render_pass.set_index_buffer(self.ui.ui_rendering.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
            render_pass.draw_indexed(main_index_count..self.ui.ui_rendering.num_indices, 0, 0..1);
        }

        self.ui.text.text_renderer_on_top.render(&self.ui.text.text_atlas, &self.renderer.glyphon.viewport, &mut render_pass).unwrap();
    }

    // F7 (see App::show_aero_debug_overlay's own doc comment) - two live
    // egui windows (see engine::rendering::egui_overlay for why it's egui
    // rendered into this same wgpu frame rather than a separate native
    // window): the roll-authority-gain chart (rolling_rate's theoretical
    // curve against the player's own recent trail), plus a second "Debug
    // Info" window with everything else Plane::flight_data tracks (speed,
    // altitude, mach, g, AoA, turn rates, ...) - this used to only be
    // readable from the in-HUD text labels (still there, unaffected) or the
    // old F3 debug_panel (FPS/position only) - this consolidates the same
    // numbers into one place alongside the chart they're for tuning
    // against. Input is deliberately minimal (pointer position + primary
    // button + scroll) - just enough for egui_plot's own pan/zoom/hover,
    // not full egui interactivity (no keyboard/text - this is a read-only
    // debug view).
    fn render_aero_debug_overlay_pass(&mut self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        if !self.debug_overlay_visible() {
            return;
        }

        // screen_rect/pointer input have to be in LOGICAL (window, not
        // drawable) pixels - the same space input::mouse_x()/mouse_y()
        // already report in (see that fn's own doc comment) - while the
        // actual render target (`view`) is sized in PHYSICAL/drawable
        // pixels. Mixing these up is what confines the whole overlay into
        // one corner of the window on a HiDPI/Retina display instead of
        // filling it - see EguiOverlay::render's own doc comment on
        // pixels_per_point for why both are needed.
        let logical_size = [self.window_manager.size.width, self.window_manager.size.height];
        let physical_size = [self.window_manager.pixel_size.width, self.window_manager.pixel_size.height];
        let pixels_per_point = physical_size[0] as f32 / logical_size[0] as f32;
        // Both right-aligned against the actual (logical) screen width -
        // "Debug Info" top-right, chart bottom-right beneath it, each
        // independently right-aligned to its own estimated window width
        // (300 for Debug Info's narrow text column, 580 for the chart's
        // wider plot) rather than a fixed offset from each other, so both
        // actually hug the real right edge on any screen size. 420 for the
        // chart's Y assumes Debug Info's own window (title bar + ~13 rows +
        // 2 separators + padding) is roughly that tall - not exact, just
        // enough clearance that they don't start out overlapping.
        let debug_info_default_x = (logical_size[0] as f32 - 300.0).max(20.0);
        let aero_chart_default_x = (logical_size[0] as f32 - 580.0).max(20.0);
        let pointer_pos = egui::pos2(input::mouse_x() as f32, input::mouse_y() as f32);
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(logical_size[0] as f32, logical_size[1] as f32))),
            events: vec![
                egui::Event::PointerMoved(pointer_pos),
                egui::Event::PointerButton {
                    pos: pointer_pos,
                    button: egui::PointerButton::Primary,
                    pressed: input::mouse_left_button_down(),
                    modifiers: egui::Modifiers::default(),
                },
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: egui::vec2(0.0, input::mouse_scroll_y()),
                    modifiers: egui::Modifiers::default(),
                },
            ],
            ..Default::default()
        };

        // The F7 "Flight Rates" window's samples - copied out for the
        // closure below (it can't borrow `self`).
        let aero_samples: Vec<AeroSample> = self.aero_debug_trail.iter().copied().collect();
        // See App::wing_lift_trail's own doc comment - two separate line
        // series sharing one x (sample index), gathered here same as
        // trail/curve above for the same borrow-checker reason.
        let main_wing_lift_trail: Vec<[f64; 2]> = self.wing_lift_trail.iter()
            .map(|(index, main_lift_y, _elevator_lift_y)| [*index as f64, *main_lift_y as f64])
            .collect();
        let elevator_wing_lift_trail: Vec<[f64; 2]> = self.wing_lift_trail.iter()
            .map(|(index, _main_lift_y, elevator_lift_y)| [*index as f64, *elevator_lift_y as f64])
            .collect();

        // Everything else this debug view shows - gathered up front into
        // plain locals (same reason curve/trail are, above) since the
        // build_ui closure below can't also borrow `self`. `flight_data` is
        // the plane's published `AircraftState` struct, already Copy-friendly f32s, so this is just
        // cheap field reads, not a real per-frame cost.
        let fps = self.time.get_fps();
        let player_position = self.scene_manager.content()
            .and_then(|content| content.renderizable_instances.get("player"))
            .map(|instance| instance.instance.transform.position);
        let flight_data = self.scene_manager.content()
            .and_then(|content| content.nodes.get("player"))
            .and_then(|node| Some((node.get_behavior::<Plane>()?, &node.physics_state::<AircraftState>()?.flight_data)))
            .map(|(plane, f)| (plane.controls.throttle, f.speedometer, f.altimeter, f.mach, f.g_meter, f.aoa_x, f.aoa_y, f.aoa, f.roll_rate, f.pitch_rate, f.yaw_rate, f.stall, plane.controls.aileron, plane.controls.elevator, plane.controls.rudder, plane.controls.trim.roll, plane.controls.trim.pitch, plane.controls.trim.yaw));

        // G Forces window: (lateral_g, longitudinal_g, body_g) - see
        // FlightData's own.
        let g_forces = self.scene_manager.content()
            .and_then(|content| content.nodes.get("player"))
            .and_then(|node| node.physics_state::<AircraftState>())
            .map(|state| (state.flight_data.lateral_g, state.flight_data.longitudinal_g, state.flight_data.body_g));
        // Pitch Rate window: x = seconds relative to the newest sample (so
        // "now" is always 0 at the right edge), y negated so nose-up plots
        // upward - FlightData::pitch_rate is the rate about the body's +X,
        // and +X is the LEFT wing (see Camera's cockpit look: positive yaw
        // turns toward +X, which is the mouse moving left), so its own
        // positive is nose DOWN.
        let pitch_now_t = self.pitch_rate_trail.back().map(|(t, _)| *t).unwrap_or(0.0);
        let pitch_trail: Vec<[f64; 2]> = self.pitch_rate_trail.iter()
            .map(|(t, rate)| [(t - pitch_now_t) as f64, -*rate as f64])
            .collect();

        // Play camera (the "camera" node's Camera behavior), read into plain
        // Copy locals for the build_ui closure. name / base offset / editor
        // offset / whether the editor is currently applied.
        let camera_info: Option<(&'static str, Vector3<f32>, Vector3<f32>, bool, f32)> = self.scene_manager.content()
            .and_then(|content| content.nodes.get("camera"))
            .and_then(|node| node.get_behavior::<Camera>())
            .map(|cam| (cam.state_name(), cam.debug_base_offset, cam.debug_offset, cam.debug_mode_active, cam.cockpit_target_fov));
        // The windows' open/closed toggles - a copy the closure below can
        // change (panel buttons, each window's close button), written back
        // after render().
        let mut windows = self.debug_windows.clone();
        let mouse_free = self.debug_mouse_free;

        // Filled by the Camera Editor window's buttons; applied after render().
        let mut camera_offset_delta = Vector3::<f32>::zeros();
        let mut camera_set_active: Option<bool> = None;
        let mut camera_reset_offset = false;
        // Set by the Camera Editor's "Copy" button - put on the system
        // clipboard after render() (egui's own copy needs platform output
        // this overlay doesn't handle).
        let mut clipboard_text: Option<String> = None;
        // How far the editor's held nudge buttons move the camera this frame
        // (units/s - same speed as F5's own keys, see Camera::update).
        let nudge_step = 2.0 * self.time.delta_time;

        self.egui_overlay.render(
            &self.renderer.device,
            &self.renderer.queue,
            encoder,
            view,
            physical_size,
            pixels_per_point,
            raw_input,
            |ctx| {
                // Pinned to the right edge: opens/closes every other window.
                egui::Window::new("Debug Windows")
                    .anchor(egui::Align2::RIGHT_CENTER, egui::vec2(-10.0, 0.0))
                    .collapsible(false)
                    .resizable(false)
                    .show(ctx, |ui| {
                        ui.label(if mouse_free { "F8: mouse free (click away)" } else { "F8: free the mouse to click" });
                        ui.separator();
                        for (name, open) in windows.entries() {
                            ui.toggle_value(open, name);
                        }
                        ui.separator();
                        ui.horizontal(|ui| {
                            if ui.button("Show all").clicked() {
                                windows.entries().into_iter().for_each(|(_, open)| *open = true);
                            }
                            if ui.button("Hide all").clicked() {
                                windows.entries().into_iter().for_each(|(_, open)| *open = false);
                            }
                        });
                    });

                egui::Window::new("Camera Editor (F7)")
                    .open(&mut windows.camera_editor)
                    .default_pos(egui::pos2(20.0, 20.0))
                    .show(ctx, |ui| {
                        let Some((cam_name, base, editor, active, cockpit_fov)) = camera_info else {
                            ui.label("(no play camera)");
                            return;
                        };
                        // Nudges move while held, in the plane's local frame
                        // (+Z forward, +Y up, +X left - matches Camera's own
                        // `target.rotation * debug_offset`).
                        let held = |response: egui::Response| response.is_pointer_button_down_on();

                        let resulting = base + editor;
                        ui.label(format!("Camera: {}", cam_name));
                        ui.label(format!("Base (state) offset: ({:+.2}, {:+.2}, {:+.2})", base.x, base.y, base.z));
                        // Drag a value left/right to slide it smoothly.
                        ui.horizontal(|ui| {
                            ui.label("Editor nudge:");
                            let mut nudge = editor;
                            let mut changed = false;
                            for (axis, value) in ["X", "Y", "Z"].into_iter().zip(nudge.iter_mut()) {
                                changed |= ui.add(egui::DragValue::new(value).speed(0.01).fixed_decimals(2).prefix(format!("{axis} "))).changed();
                            }
                            if changed {
                                camera_offset_delta += nudge - editor;
                            }
                        });
                        ui.label(format!("Resulting offset:    ({:+.2}, {:+.2}, {:+.2})", resulting.x, resulting.y, resulting.z));
                        ui.separator();

                        ui.horizontal(|ui| {
                            ui.add_space(40.0);
                            if held(ui.button("  Up  ")) { camera_offset_delta.y += nudge_step; }
                        });
                        ui.horizontal(|ui| {
                            if held(ui.button(" Left ")) { camera_offset_delta.x += nudge_step; }
                            if held(ui.button("Right ")) { camera_offset_delta.x -= nudge_step; }
                        });
                        ui.horizontal(|ui| {
                            ui.add_space(40.0);
                            if held(ui.button(" Down ")) { camera_offset_delta.y -= nudge_step; }
                        });
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if held(ui.button("Forward")) { camera_offset_delta.z += nudge_step; }
                            if held(ui.button(" Back  ")) { camera_offset_delta.z -= nudge_step; }
                        });
                        ui.separator();

                        let mut active_mut = active;
                        if ui.checkbox(&mut active_mut, "Editor active (also F5)").changed() {
                            camera_set_active = Some(active_mut);
                        }
                        if ui.button("Reset nudge").clicked() {
                            camera_reset_offset = true;
                        }
                        ui.add_space(2.0);
                        ui.label(format!("position: Vector3::new({:.3}, {:.3}, {:.3})", resulting.x, resulting.y, resulting.z));

                        // The cockpit camera's own settings - the same values
                        // Settings > Video shows (GAME_SETTINGS), so either
                        // place changes both.
                        ui.separator();
                        ui.label("Cockpit camera");
                        let mut settings = GAME_SETTINGS.lock().unwrap();
                        ui.add(egui::Slider::new(&mut settings.head_g_reaction, 0.0..=HEAD_G_REACTION_MAX)
                            .text("Head G reaction")
                            .custom_formatter(|value, _| format!("{:.0}%", value * 100.0)));
                        ui.add(egui::Slider::new(&mut settings.cockpit_view_pitch_deg, -COCKPIT_VIEW_PITCH_MAX..=COCKPIT_VIEW_PITCH_MAX)
                            .text("View angle (+ = up)")
                            .fixed_decimals(1)
                            .suffix(" deg"));

                        // Everything above, as text to paste back in and
                        // bake into the first-person camera's defaults.
                        ui.separator();
                        if ui.button("Copy camera values").clicked() {
                            clipboard_text = Some(format!(
                                "First-person camera values (Camera Editor):\n\
                                 camera state: {cam_name}\n\
                                 position: Vector3::new({:.3}, {:.3}, {:.3})\n\
                                 cockpit fov: {:.1}\n\
                                 head g reaction: {:.2} ({:.0}%)\n\
                                 view angle: {:+.1} deg (+ = up)",
                                resulting.x, resulting.y, resulting.z,
                                cockpit_fov,
                                settings.head_g_reaction, settings.head_g_reaction * 100.0,
                                settings.cockpit_view_pitch_deg,
                            ));
                        }
                        if cam_name != "Cockpit" {
                            ui.label("(position is the current camera's - switch to Cockpit first)");
                        }
                    });

                egui::Window::new("Debug Info (F7)")
                    .open(&mut windows.debug_info)
                    .default_pos(egui::pos2(debug_info_default_x, 20.0))
                    .show(ctx, |ui| {
                        ui.label(format!("FPS: {:.0}", fps));
                        if let Some(pos) = player_position {
                            ui.label(format!("Position: ({:.1}, {:.1}, {:.1})", pos.x, pos.y, pos.z));
                        } else {
                            ui.label("Position: (no player)");
                        }
                        ui.separator();
                        if let Some((throttle, speedometer, altimeter, mach, g_meter, aoa_x, aoa_y, aoa, roll_rate, pitch_rate, yaw_rate, stall, _aileron, _elevator, _rudder, _trim_roll, _trim_pitch, _trim_yaw)) = flight_data {
                            ui.label(format!("Throttle: {:.0}%", throttle * 100.0));
                            ui.label(format!("Speed: {:.0} kt", speedometer));
                            ui.label(format!("Altitude: {:.0}", altimeter));
                            ui.label(format!("Mach: {:.2}", mach));
                            ui.label(format!("G: {:.1}", g_meter));
                            ui.label(format!("Stall: {}", stall));
                            ui.separator();
                            // Plain "deg" rather than the "°" glyph - egui's
                            // own default embedded font doesn't guarantee
                            // that character renders (showed up blank/
                            // missing in testing), ASCII always will.
                            ui.label(format!("AoA X: {:.1} deg", aoa_x));
                            ui.label(format!("AoA Y: {:.1} deg", aoa_y));
                            ui.label(format!("AoA: {:.1} deg", aoa));
                            ui.separator();
                            ui.label(format!("Roll rate: {:.1} deg/s", roll_rate));
                            ui.label(format!("Pitch rate: {:.1} deg/s", pitch_rate));
                            ui.label(format!("Yaw rate: {:.1} deg/s", yaw_rate));
                        } else {
                            ui.label("Flight data: (no player)");
                        }
                    });

                // Stick position indicator - X = aileron (roll), Y = elevator
                // (pitch), rudder shown separately as a bar (no natural 2nd
                // axis to pair it with here). Drawn with egui's own raw
                // Painter rather than egui_plot - this isn't really a chart
                // (no axes/data series), just a circle + a dot, which
                // egui_plot has no particularly good primitive for.
                egui::Window::new("Stick Input (F7)")
                    .open(&mut windows.stick_input)
                    .default_pos(egui::pos2(debug_info_default_x, 420.0))
                    .show(ctx, |ui| {
                        if let Some((.., aileron, elevator, rudder, trim_roll, trim_pitch, trim_yaw)) = flight_data {
                            ui.label("X = aileron, Y = elevator. Circle = full deflection.");
                            ui.label("Red = commanded stick. Yellow ring = trim only.");
                            let size = egui::vec2(200.0, 200.0);
                            let (response, painter) = ui.allocate_painter(size, egui::Sense::hover());
                            let rect = response.rect;
                            let center = rect.center();
                            let radius = rect.width().min(rect.height()) * 0.5 - 4.0;

                            painter.circle_stroke(center, radius, egui::Stroke::new(1.5, egui::Color32::GRAY));
                            painter.line_segment([egui::pos2(center.x - radius, center.y), egui::pos2(center.x + radius, center.y)], egui::Stroke::new(1.0, egui::Color32::DARK_GRAY));
                            painter.line_segment([egui::pos2(center.x, center.y - radius), egui::pos2(center.x, center.y + radius)], egui::Stroke::new(1.0, egui::Color32::DARK_GRAY));

                            // Screen Y grows downward, so elevator is negated
                            // here to make "nose up" (positive elevator, by
                            // this game's own convention) plot upward on
                            // screen, matching how a real stick display reads
                            // - flip this back if elevator's own sign turns
                            // out to mean the opposite in this game.
                            let stick_pos = egui::pos2(
                                center.x + aileron.clamp(-1.0, 1.0) * radius,
                                center.y - elevator.clamp(-1.0, 1.0) * radius,
                            );
                            painter.circle_filled(stick_pos, 6.0, egui::Color32::from_rgb(255, 80, 80));

                            // Trim-only marker - where "elevator"/"aileron"
                            // above would sit from trim alone, ignoring
                            // whatever the pilot's stick is doing right now.
                            // Plotted directly from trim_roll/trim_pitch, not
                            // from `elevator`/`aileron` (those already have
                            // pitch trim folded in as of this session - see
                            // Plane::update's own comment - so re-deriving
                            // trim's contribution from them isn't possible
                            // once the stick is off-center; trim_roll/
                            // trim_pitch are the raw Trim values instead).
                            // Same sign-of-elevator caveat as the stick dot
                            // above.
                            let trim_pos = egui::pos2(
                                center.x + trim_roll.clamp(-1.0, 1.0) * radius,
                                center.y - trim_pitch.clamp(-1.0, 1.0) * radius,
                            );
                            painter.circle_stroke(trim_pos, 6.0, egui::Stroke::new(2.0, egui::Color32::from_rgb(255, 220, 60)));

                            ui.add_space(4.0);
                            ui.label(format!("Aileron: {:.2}  Elevator: {:.2}  Rudder: {:.2}", aileron, elevator, rudder));
                            ui.label(format!("Trim roll: {:.2}  Trim pitch: {:.2}", trim_roll, trim_pitch));

                            // Yaw trim has no natural 2nd axis to share the
                            // circle with (same reason rudder itself isn't
                            // plotted there either - see this block's own
                            // top comment), so it gets its own bar here
                            // instead. `flight_data` is a plain snapshot
                            // gathered before this closure (can't borrow
                            // `self` again inside it - see where it's built,
                            // above), not a live handle into Plane's own
                            // Trim, so this can't actually be dragged to set
                            // yaw trim the way a normal Slider would suggest
                            // - disabled to read as the display-only gauge
                            // it is, matching everything else in this
                            // window. Say the word if you want it wired up
                            // to actually set trim.yaw instead.
                            ui.add_space(4.0);
                            let mut trim_yaw_display = trim_yaw;
                            ui.add_enabled(false, egui::Slider::new(&mut trim_yaw_display, -1.0..=1.0).text("Yaw trim"));
                        } else {
                            ui.label("(no player)");
                        }
                    });

                // Circle: the car-style G meter (FlightData::lateral_g/
                // longitudinal_g - same values the cockpit camera's head
                // follows) - X = side, Y = forward/back. The dot is the way
                // the pilot is PUSHED - the opposite of the acceleration:
                // speeding up pushes it back, braking forward, turning left
                // pushes it right. Bar: felt seat-axis G (1 = level flight).
                egui::Window::new("G Forces (F7)")
                    .open(&mut windows.g_forces)
                    .default_pos(egui::pos2(20.0, 360.0))
                    .show(ctx, |ui| {
                        // Circle edge / bar ends, in G.
                        const CIRCLE_MAX_G: f32 = 2.0;
                        const BAR_MIN_G: f32 = -3.0;
                        const BAR_MAX_G: f32 = 9.0;

                        let Some((lateral_g, longitudinal_g, body_g)) = g_forces else {
                            ui.label("(no player)");
                            return;
                        };
                        // Body +X is the left wing (see pitch_trail above),
                        // so this is + toward the right.
                        let right_g = -lateral_g;
                        let vertical_g = body_g.y;

                        // The push on the pilot - opposite the acceleration.
                        let push_forward = -longitudinal_g;
                        let push_right = -right_g;

                        ui.label(format!("Circle = +-{:.0} G, rings every 1 G. Dot = push on the pilot.", CIRCLE_MAX_G));
                        let (response, painter) = ui.allocate_painter(egui::vec2(260.0, 220.0), egui::Sense::hover());
                        let rect = response.rect;
                        let text_color = ui.visuals().text_color();
                        let font = egui::FontId::proportional(11.0);
                        let grid = egui::Stroke::new(1.0, egui::Color32::DARK_GRAY);
                        let red = egui::Color32::from_rgb(255, 80, 80);

                        // Circle, on the left of the drawing area.
                        let center = egui::pos2(rect.left() + 110.0, rect.center().y);
                        let radius = 100.0;
                        let per_g = radius / CIRCLE_MAX_G;
                        painter.circle_stroke(center, radius, egui::Stroke::new(1.5, egui::Color32::GRAY));
                        for ring in 1..CIRCLE_MAX_G as i32 {
                            painter.circle_stroke(center, ring as f32 * per_g, grid);
                        }
                        painter.line_segment([egui::pos2(center.x - radius, center.y), egui::pos2(center.x + radius, center.y)], grid);
                        painter.line_segment([egui::pos2(center.x, center.y - radius), egui::pos2(center.x, center.y + radius)], grid);
                        painter.text(egui::pos2(center.x, center.y - radius + 2.0), egui::Align2::CENTER_TOP, "FWD", font.clone(), egui::Color32::GRAY);
                        painter.text(egui::pos2(center.x, center.y + radius - 2.0), egui::Align2::CENTER_BOTTOM, "BACK", font.clone(), egui::Color32::GRAY);
                        painter.text(egui::pos2(center.x - radius + 4.0, center.y - 2.0), egui::Align2::LEFT_BOTTOM, "L", font.clone(), egui::Color32::GRAY);
                        painter.text(egui::pos2(center.x + radius - 4.0, center.y - 2.0), egui::Align2::RIGHT_BOTTOM, "R", font.clone(), egui::Color32::GRAY);

                        // Screen Y grows downward, so forward is negated.
                        // Pinned to the edge (hollow) when past the circle.
                        let offset = egui::vec2(push_right, -push_forward) * per_g;
                        let past_edge = offset.length() > radius;
                        let offset = if past_edge { offset * (radius / offset.length()) } else { offset };
                        let dot = center + offset;
                        painter.line_segment([center, dot], egui::Stroke::new(1.0, red));
                        if past_edge {
                            painter.circle_stroke(dot, 6.0, egui::Stroke::new(2.0, red));
                        } else {
                            painter.circle_filled(dot, 6.0, red);
                        }

                        // Seat-axis bar, on the right.
                        let bar = egui::Rect::from_min_max(
                            egui::pos2(rect.left() + 232.0, center.y - radius),
                            egui::pos2(rect.left() + 252.0, center.y + radius),
                        );
                        let bar_y = |g: f32| bar.bottom() - (g.clamp(BAR_MIN_G, BAR_MAX_G) - BAR_MIN_G) / (BAR_MAX_G - BAR_MIN_G) * bar.height();
                        painter.rect_stroke(bar, 2.0, egui::Stroke::new(1.5, egui::Color32::GRAY), egui::StrokeKind::Inside);
                        let bar_color = if vertical_g > 7.0 || vertical_g < -1.0 { red } else { egui::Color32::from_rgb(80, 200, 120) };
                        // Filled from 0 G to the value, so negative G hangs
                        // below the zero line.
                        let (fill_top, fill_bottom) = (bar_y(vertical_g).min(bar_y(0.0)), bar_y(vertical_g).max(bar_y(0.0)));
                        painter.rect_filled(egui::Rect::from_min_max(egui::pos2(bar.left() + 2.0, fill_top), egui::pos2(bar.right() - 2.0, fill_bottom)), 1.0, bar_color);
                        for (g, label) in [(BAR_MIN_G, "-3"), (0.0, "0"), (1.0, "1"), (BAR_MAX_G, "9")] {
                            let y = bar_y(g);
                            painter.line_segment([egui::pos2(bar.left() - 4.0, y), egui::pos2(bar.left(), y)], egui::Stroke::new(1.0, egui::Color32::GRAY));
                            painter.text(egui::pos2(bar.left() - 6.0, y), egui::Align2::RIGHT_CENTER, label, font.clone(), text_color);
                        }

                        ui.add_space(4.0);
                        ui.label(format!("Pushed forward: {:+.2} G   Pushed right: {:+.2} G", push_forward, push_right));
                        ui.label(format!("Up/down (seat): {:+.2} G", vertical_g));
                        ui.label("Accelerating pushes you back (-), braking forward (+).");
                    });

                // Top-down view of the main wings and their hinged control
                // surfaces, to scale - see AeroSpec/ControlSurfaceSpec.
                // Nose up. Surfaces shaded by the stick's deflection
                // (orange = trailing edge down, blue = up - a positive
                // deflection is trailing edge UP, see ControlSurfaceSpec).
                egui::Window::new("Wing Surfaces (F7)")
                    .open(&mut windows.wing_surfaces)
                    .default_pos(egui::pos2(300.0, 360.0))
                    .show(ctx, |ui| {
                        let mut kinematic = KINEMATIC_ROLL.load(std::sync::atomic::Ordering::Relaxed);
                        if ui.checkbox(&mut kinematic, "Kinematic roll (off = physics roll)").changed() {
                            KINEMATIC_ROLL.store(kinematic, std::sync::atomic::Ordering::Relaxed);
                        }
                        let mut roll_flcs = ROLL_FLCS.load(std::sync::atomic::Ordering::Relaxed);
                        if ui.checkbox(&mut roll_flcs, "Roll fly-by-wire (off = raw airframe, stick = surface travel)").changed() {
                            ROLL_FLCS.store(roll_flcs, std::sync::atomic::Ordering::Relaxed);
                        }
                        let mut yaw_flcs = YAW_FLCS.load(std::sync::atomic::Ordering::Relaxed);
                        if ui.checkbox(&mut yaw_flcs, "Yaw fly-by-wire (off = raw airframe, pedals = rudder travel)").changed() {
                            YAW_FLCS.store(yaw_flcs, std::sync::atomic::Ordering::Relaxed);
                        }
                        ui.separator();

                        let aileron = flight_data.map(|(.., aileron, _, _, _, _, _)| aileron).unwrap_or(0.0);
                        // The F-16's wings, from its data.ron - read once, not every frame.
                        static F16_AERO: std::sync::OnceLock<Option<AeroSpec>> = std::sync::OnceLock::new();
                        let aero = F16_AERO.get_or_init(|| AircraftSpec::load("f16").map(|spec| spec.aero).map_err(|e| eprintln!("{e}")).ok());
                        // Top-down view: horizontal surfaces only (a fin stands upright).
                        let wings: Vec<_> = aero.iter().flat_map(|aero| aero.wings.iter().cloned()).filter(|w| w.chord > 0.0 && w.normal.x.abs() > 0.5).collect();
                        // Fuselage half-width between the wing roots (m).
                        const ROOT_GAP: f32 = 0.6;
                        let widest = wings.iter().map(|w| ROOT_GAP + w.wing_area / w.chord).fold(0.0, f32::max);
                        let deepest = wings.iter().map(|w| w.chord).fold(0.0, f32::max);
                        let (response, painter) = ui.allocate_painter(egui::vec2(480.0, 200.0), egui::Sense::hover());
                        let rect = response.rect.shrink(10.0);
                        let scale = (rect.width() * 0.5 / widest.max(0.1)).min(rect.height() / deepest.max(0.1));
                        let center_x = rect.center().x;
                        let leading_y = rect.center().y - deepest * scale * 0.5;
                        let font = egui::FontId::proportional(11.0);

                        for wing in &wings {
                            let half_span = wing.wing_area / wing.chord;
                            // Body +X is the left wing - drawn on the left.
                            let side = if wing.pressure_center.x >= 0.0 { -1.0 } else { 1.0 };
                            let span_x = |fraction: f32| center_x + side * (ROOT_GAP + fraction * half_span) * scale;
                            let outline = egui::Rect::from_two_pos(
                                egui::pos2(span_x(0.0), leading_y),
                                egui::pos2(span_x(1.0), leading_y + wing.chord * scale),
                            );
                            painter.rect_filled(outline, 2.0, egui::Color32::from_gray(45));
                            painter.rect_stroke(outline, 2.0, egui::Stroke::new(1.0, egui::Color32::GRAY), egui::StrokeKind::Inside);

                            for surface in &wing.surfaces {
                                let input = match surface.kind {
                                    SurfaceKind::Aileron => aileron_for_side(wing.pressure_center.x, aileron),
                                    SurfaceKind::Flap => 0.0,
                                    SurfaceKind::Rudder => 0.0,
                                };
                                let deflection = input * surface.max_deflection_deg;
                                let strength = (deflection.abs() / surface.max_deflection_deg.max(1.0)).clamp(0.0, 1.0);
                                let fill = if deflection <= 0.0 {
                                    egui::Color32::from_rgb(80 + (175.0 * strength) as u8, 80 + (60.0 * strength) as u8, 80)
                                } else {
                                    egui::Color32::from_rgb(80, 80 + (60.0 * strength) as u8, 80 + (175.0 * strength) as u8)
                                };
                                let trailing_y = leading_y + wing.chord * scale;
                                let strip = egui::Rect::from_two_pos(
                                    egui::pos2(span_x(surface.span_start), trailing_y - surface.chord_ratio * wing.chord * scale),
                                    egui::pos2(span_x(surface.span_end), trailing_y),
                                );
                                painter.rect_filled(strip, 1.0, fill);
                                painter.rect_stroke(strip, 1.0, egui::Stroke::new(1.0, egui::Color32::WHITE), egui::StrokeKind::Inside);
                                painter.text(egui::pos2(strip.center().x, trailing_y + 4.0), egui::Align2::CENTER_TOP,
                                    format!("{} {:.1} deg", if deflection > 0.0 { "up" } else if deflection < 0.0 { "down" } else { "" }, deflection.abs()), font.clone(), ui.visuals().text_color());
                            }
                        }
                        painter.text(egui::pos2(center_x, leading_y - 4.0), egui::Align2::CENTER_BOTTOM, "NOSE", font.clone(), egui::Color32::GRAY);

                        ui.add_space(14.0);
                        for wing in &wings {
                            for surface in &wing.surfaces {
                                let share = surface.span_end - surface.span_start;
                                ui.label(format!(
                                    "{}: {:?}, span {:.0}-{:.0}% ({:.2} m2 of wing), {:.0}% chord, +-{:.1} deg",
                                    surface.label, surface.kind, surface.span_start * 100.0, surface.span_end * 100.0,
                                    wing.wing_area * share, surface.chord_ratio * 100.0, surface.max_deflection_deg,
                                ));
                            }
                        }
                        ui.label("Deflection shown is the raw stick - with roll fly-by-wire on, the surfaces move by what it commands instead.");
                    });

                egui::Window::new("Pitch Rate (F7)")
                    .open(&mut windows.pitch_rate)
                    .default_pos(egui::pos2(20.0, 700.0))
                    .show(ctx, |ui| {
                        let Some(current) = pitch_trail.last().map(|p| p[1]) else {
                            ui.label("(no samples yet)");
                            return;
                        };
                        let (min, max) = pitch_trail.iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| (lo.min(p[1]), hi.max(p[1])));
                        ui.label(egui::RichText::new(format!("{:+.1} deg/s", current)).size(20.0).strong());
                        ui.label(format!("+ = nose up. Last 10 s: min {:+.1}, max {:+.1} deg/s", min, max));
                        egui_plot::Plot::new("pitch_rate_live_plot")
                            .height(220.0)
                            .width(520.0)
                            .x_axis_label("Seconds ago")
                            .y_axis_label("Pitch rate (deg/s)")
                            .x_axis_formatter(|mark, _range| format!("{:.0}", mark.value))
                            .y_axis_formatter(|mark, _range| format!("{:.0}", mark.value))
                            // Always the full 10 s window, and at least
                            // +-10 deg/s so level flight isn't all noise.
                            .include_x(-10.0)
                            .include_x(0.0)
                            .include_y(-10.0)
                            .include_y(10.0)
                            .allow_drag(false)
                            .allow_zoom(false)
                            .allow_scroll(false)
                            .show(ui, |plot_ui| {
                                plot_ui.hline(egui_plot::HLine::new("zero", 0.0).color(egui::Color32::DARK_GRAY));
                                plot_ui.line(egui_plot::Line::new("Pitch rate", egui_plot::PlotPoints::from(pitch_trail.clone())).color(egui::Color32::from_rgb(90, 170, 255)).width(1.5));
                            });
                    });

                // Positioned clearly right of "Debug Info" (which sits at
                // x=20 above) rather than the other way around, per request.
                // One tab per rate, each plotted against airspeed: the
                // recent flight as dots, and the theoretical curve (AoA 0,
                // sea level, full control) as a line when that rate has one
                // - see RateChart.
                let rate_chart = &mut windows.rate_chart;
                egui::Window::new("Flight Rates (F7)")
                    .open(&mut windows.roll_authority)
                    .default_pos(egui::pos2(aero_chart_default_x, 420.0))
                    .show(ctx, |ui| {
                        ui.horizontal(|ui| {
                            for chart in RateChart::ALL {
                                ui.selectable_value(rate_chart, chart, chart.label());
                            }
                        });
                        let chart = *rate_chart;
                        // Mach 0..2 in steps of 0.005.
                        let curve: Option<Vec<[f64; 2]>> = (0..=(RATE_CHART_MAX_MACH * 200.0) as u32)
                            .map(|step| {
                                let mach = step as f32 / 200.0;
                                chart.chart_max(mach * KNOTS_PER_MACH, 0.0, 0.0).map(|max| [mach as f64, max as f64])
                            })
                            .collect();
                        let gravity_curve: Option<Vec<[f64; 2]>> = (0..=(RATE_CHART_MAX_MACH * 200.0) as u32)
                            .map(|step| {
                                let mach = step as f32 / 200.0;
                                chart.gravity_curve(mach).map(|rate| [mach as f64, rate as f64])
                            })
                            .collect();
                        let dots: Vec<[f64; 2]> = aero_samples.iter().map(|s| [s.mach as f64, chart.measured(s) as f64]).collect();

                        ui.label(match &curve {
                            Some(_) => format!("Line = theoretical max {} rate (AoA 0, sea level, full control). Dots = recent flight (measured).", chart.label().to_lowercase()),
                            None => format!("Dots = recent flight (measured {} rate). No chart for this rate yet.", chart.label().to_lowercase()),
                        });
                        ui.horizontal(|ui| {
                            if ui.button("Copy last values (chart vs real)").clicked() {
                                clipboard_text = Some(chart.copy_text(&aero_samples));
                            }
                            if let Some(last) = aero_samples.last() {
                                ui.label(format!("now: {:.1} {}", chart.measured(last), chart.unit()));
                                if let Some(charted) = chart.charted(last) {
                                    ui.label(format!("chart: {:.1}", charted));
                                }
                                if let Some(with_gravity) = chart.gravity_adjusted(last) {
                                    ui.label(format!("chart-g: {:.1}", with_gravity));
                                }
                            }
                        });
                        // Fixed axes - Mach 0..2 across, the rate's own
                        // range up - and no drag/zoom, so the chart holds
                        // still while the dots move over it.
                        let (y_min, y_max) = chart.y_range();
                        let plot = egui_plot::Plot::new(format!("rate_plot_{}", chart.label()))
                            .height(320.0)
                            .width(520.0)
                            .default_x_bounds(0.0, RATE_CHART_MAX_MACH)
                            .default_y_bounds(y_min, y_max)
                            .allow_drag(false)
                            .allow_zoom(false)
                            .allow_scroll(false)
                            .allow_boxed_zoom(false)
                            .allow_double_click_reset(false)
                            .x_axis_label("Mach")
                            .y_axis_label(format!("{} rate ({})", chart.label(), chart.unit()))
                            .x_axis_formatter(|mark, _range| format!("{:.1}", mark.value))
                            .y_axis_formatter(|mark, _range| format!("{:.0}", mark.value))
                            // Fixed spacing rather than egui_plot's own
                            // automatic tick-spacing algorithm on either
                            // axis - that algorithm picks spacing based on
                            // how much pixel width/height is actually
                            // available, and appears to have been collapsing
                            // to just the endpoint ticks when the plot ended
                            // up smaller than expected. This guarantees
                            // ticks at 0/100/.../800 knots and
                            // 0/25/50/.../deg/s regardless of that (25 rather
                            // than a round 50 so RollRateParams::default's
                            // own 260 deg/s peak lands on a tick, not between
                            // two).
                            .x_grid_spacer(egui_plot::uniform_grid_spacer(|_input| [0.1, 0.5, 1.0]))
                            .legend(egui_plot::Legend::default());
                        // Roll keeps its fixed 25 deg/s ticks (see above);
                        // the others use egui_plot's own.
                        let plot = if chart == RateChart::Roll {
                            plot.y_grid_spacer(egui_plot::uniform_grid_spacer(|_input| [25.0, 25.0, 25.0]))
                        } else {
                            plot
                        };
                        plot.show(ui, |plot_ui| {
                            if let Some(curve) = &curve {
                                plot_ui.line(egui_plot::Line::new("Theoretical (AoA 0)", egui_plot::PlotPoints::from(curve.clone())));
                            }
                            if let Some(gravity_curve) = gravity_curve {
                                plot_ui.line(egui_plot::Line::new("Chart - gravity (level flight)", egui_plot::PlotPoints::from(gravity_curve)).style(egui_plot::LineStyle::dashed_loose()));
                            }
                            plot_ui.points(egui_plot::Points::new("Recent flight", egui_plot::PlotPoints::from(dots)).radius(2.0));
                        });
                    });

                // Live wing lift force - see App::wing_lift_trail's own doc
                // comment. Positioned below "Aero Debug" (same x, height
                // 420+320+~40 padding below that one's own y=420 start).
                egui::Window::new("Wing Lift Forces (F7)")
                    .open(&mut windows.wing_lift)
                    .default_pos(egui::pos2(aero_chart_default_x, 780.0))
                    .show(ctx, |ui| {
                        ui.label("Vertical (Y) lift force, Left wing (main) and Right elevator wing, most recent samples.");
                        egui_plot::Plot::new("wing_lift_live_plot")
                            .height(240.0)
                            .width(520.0)
                            .x_axis_label("Sample")
                            .y_axis_label("Lift force Y (N)")
                            .x_axis_formatter(|mark, _range| format!("{:.0}", mark.value))
                            .y_axis_formatter(|mark, _range| format!("{:.0}", mark.value))
                            .legend(egui_plot::Legend::default())
                            .show(ui, |plot_ui| {
                                plot_ui.line(egui_plot::Line::new("Main wing (Left)", egui_plot::PlotPoints::from(main_wing_lift_trail.clone())));
                                plot_ui.line(egui_plot::Line::new("Elevator wing (Right)", egui_plot::PlotPoints::from(elevator_wing_lift_trail.clone())));
                            });
                    });
            },
        );

        self.debug_windows = windows;

        if let Some(text) = clipboard_text {
            match self.window_manager.context.video().and_then(|video| video.clipboard().set_clipboard_text(&text)) {
                Ok(()) => println!("Camera values copied to clipboard:\n{text}"),
                Err(error) => println!("Couldn't copy camera values ({error}):\n{text}"),
            }
        }

        // Apply the Camera Editor window's button presses (couldn't touch
        // `self` from inside the build_ui closure above).
        if camera_offset_delta != Vector3::zeros() || camera_set_active.is_some() || camera_reset_offset {
            if let Some(cam) = self.scene_manager.content_mut()
                .and_then(|content| content.nodes.get_mut("camera"))
                .and_then(|node| node.get_behavior_mut::<Camera>())
            {
                if camera_reset_offset {
                    cam.debug_offset = Vector3::zeros();
                }
                cam.debug_offset += camera_offset_delta;
                if let Some(v) = camera_set_active {
                    cam.debug_mode_active = v;
                }
                // A nudge is meaningless unless the editor offset is actually
                // being applied - turn it on.
                if camera_offset_delta != Vector3::zeros() {
                    cam.debug_mode_active = true;
                }
            }
        }
    }

    // Debug lines come from the physics thread in absolute world coordinates,
    // so make them camera-relative here to match camera.view_proj.
    fn render_physics_debug_lines<'rp>(&mut self, render_pass: &mut wgpu::RenderPass<'rp>) {
        let camera_position = self.scene_manager.cameras().map(|c| c.active().camera.position()).unwrap_or_else(nalgebra::Point3::origin);
        let vertices: Vec<ManualVertex> = self.render_physics.renderizable_lines.iter()
            .flat_map(|line| line.to_vec())
            .map(|mut vertex| {
                vertex.position[0] -= camera_position.x;
                vertex.position[1] -= camera_position.y;
                vertex.position[2] -= camera_position.z;
                vertex
            })
            .collect();

        if vertices.is_empty() {
            return;
        }

        self.render_physics.vertex_buffer = self.renderer.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Updated ManualVertex Buffer"),
            size: (vertices.len() * std::mem::size_of::<ManualVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        self.render_physics.vertex_buffer.slice(..).get_mapped_range_mut().copy_from_slice(bytemuck::cast_slice(&vertices));
        self.render_physics.vertex_buffer.unmap();

        // Each line has two vertices
        let mut indices = Vec::new();
        for i in 0..self.render_physics.renderizable_lines.len() {
            let base_index = (i * 2) as u16;
            indices.push(base_index);
            indices.push(base_index + 1);
        }

        if indices.is_empty() {
            return;
        }

        self.render_physics.index_buffer = self.renderer.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Index Buffer"),
            size: (indices.len() * std::mem::size_of::<u16>()) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });
        self.render_physics.index_buffer.slice(..).get_mapped_range_mut().copy_from_slice(bytemuck::cast_slice(&indices));
        self.render_physics.index_buffer.unmap();

        render_pass.set_vertex_buffer(0, self.render_physics.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.render_physics.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        render_pass.draw_indexed(0..(indices.len() as u32), 0, 0..1);
    }

    pub(crate) fn render_scene_passes(&mut self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        self.render_opaque_pass(encoder);
        self.render_water_pass(encoder);
        self.render_transparent_pass(encoder);
        self.render_particle_pass(encoder);
        // Blits scene_color onto the real swapchain view, so the scene still looks
        // normal everywhere - see BlurRender::render. Has to run after the 3D
        // passes above (nothing to blit yet otherwise) and before the UI pass
        // below, which samples scene_color directly (at whatever per-node radius,
        // see text_shader.wgsl) for any node with background_blur set.
        self.renderer.blur.write_effects(&self.renderer.queue);
        self.renderer.blur.render(encoder, view);
        self.render_ui_pass(encoder, view);
        // Drawn last so it's always on top of the game's own HUD too.
        self.render_aero_debug_overlay_pass(encoder, view);
    }
}
