use std::time::{Duration, Instant};

/// The frame clock - `app.time`.
///
/// `time_scale` speeds the game up or slows it down: 1 is normal, 0.5 half
/// speed, 0 paused. It scales `delta_time` (what scenes, node behaviors,
/// particles and animations move by) and the physics alike; menus that must
/// keep moving while the game is paused use `unscaled_delta_time`.
pub struct Timing {
    /// Seconds since the last frame, times `time_scale`.
    pub delta_time: f32,
    /// Seconds since the last frame, as they really passed.
    pub unscaled_delta_time: f32,
    pub time_scale: f32,
    /// Scaled seconds since the app started, wrapped every 10,000 so its
    /// precision doesn't degrade over a long session - for shader animation
    /// (the light buffer's `time`), whose waves are periodic so the wrap
    /// doesn't show.
    shader_time: f32,
    last_frame: Instant,
    frame_count: u32,
    last_fps_update: Instant,
    fps: f32,
}

impl Timing {
    pub fn new() -> Self {
        Self {
            delta_time: 0.0,
            unscaled_delta_time: 0.0,
            time_scale: 1.0,
            shader_time: 0.0,
            last_frame: Instant::now(),
            frame_count: 0,
            last_fps_update: Instant::now(),
            fps: 0.0,
        }
    }

    pub fn update(&mut self) {
        let now = Instant::now();
        self.unscaled_delta_time = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.delta_time = self.unscaled_delta_time * self.time_scale.max(0.0);
        self.shader_time = (self.shader_time + self.delta_time) % 10_000.0;
        self.calculate_framerate();
    }

    /// Whether `time_scale` has the game stopped.
    pub fn is_paused(&self) -> bool {
        self.time_scale <= 0.0
    }

    /// See `shader_time`.
    pub fn shader_time(&self) -> f32 {
        self.shader_time
    }

    pub fn fps(&self) -> f32 {
        self.fps
    }

    fn calculate_framerate(&mut self) {
        self.frame_count += 1;

        // Update FPS every second
        if self.last_fps_update.elapsed() >= Duration::from_secs(1) {
            self.fps = self.frame_count as f32;
            self.frame_count = 0;
            self.last_fps_update = Instant::now();
        }
    }
}
