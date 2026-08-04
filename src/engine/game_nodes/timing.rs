use std::time::{Duration, Instant};


pub struct Timing {
    pub delta_time: f32,
    last_frame: Instant,
    frame_count: u32,
    last_fps_update: Instant,
    fps: f32
}

impl Timing {
    pub fn new() -> Self {
        Self {
            delta_time: 0.0,
            last_frame: Instant::now(),
            frame_count: 0,
            last_fps_update: Instant::now(),
            fps: 0.0
        }
    }

    pub fn update(&mut self) {
        self.calculate_delta_time();
        self.calculate_framerate();
    }

    fn calculate_delta_time(&mut self) {
        let current_time = Instant::now();
        self.delta_time = current_time.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = current_time;
    }

    fn calculate_framerate(&mut self) -> f32 {
        self.frame_count += 1;

        // Update FPS every second
        if self.last_fps_update.elapsed() >= Duration::from_secs(1) {
            self.fps = self.frame_count as f32;
            self.frame_count = 0; // Reset frame count for the next second
            self.last_fps_update = Instant::now(); // Update the last FPS update time
        }
        
        self.fps
    }

    pub fn get_fps(&mut self) -> f32 {
        self.fps
    }
}