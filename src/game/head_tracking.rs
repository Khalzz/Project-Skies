//! Live head tracker. Settings picks TrackIR or OpenTrack (see
//! `GameSettings::head_tracking`); this starts that method and remembers
//! whether a frame has actually arrived. The cockpit camera reads the pose.
//!
//! `head-tracking-rs`'s `status()` stays false until `poll()` has returned a
//! pose, so a method that merely started is not active. On the first start
//! each available method is listened to for [`RETRY_WINDOW`]: the first one
//! that delivers a frame is selected. After that, a closed tracker stays
//! inactive until the player picks it and presses Retry, which listens again
//! for the same window and stops at the first frame.
//!
//! One static, same reason as `GAME_SETTINGS`: the main menu and the pause
//! menu each build their own settings tree, and neither should tear the
//! tracker down and start it again just because the UI was rebuilt.

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use head_tracking_rs::{HeadPose, HeadTracker, HeadTrackerMethod, DEFAULT_PORT};

use crate::app::App;
use crate::game::game_settings::{HeadTrackingSource, GAME_SETTINGS};

/// The id from head-tracking-rs's own example. OpenTrack accepts any number.
/// The real TrackIR software only accepts the id NaturalPoint assigns to a
/// game; a mismatch comes back as not connected, with that error.
const TRACKIR_PROFILE_ID: u16 = 1001;

/// How long a TrackIR frame stays "live". The client reports a frame only
/// when its signature changes, so a still head still counts while TrackIR
/// keeps transmitting. After this, mouse look in the cockpit comes back.
const TRACKIR_RECEIVING_WINDOW: Duration = Duration::from_millis(500);

/// How long startup, and the Retry button, keep calling `poll` while waiting
/// for the first frame. A frame ends the wait early.
const RETRY_WINDOW: Duration = Duration::from_secs(3);

/// Tracker pitch and roll, degrees, either way. Applied to every method in
/// `tracked_pose`, so TrackIR and OpenTrack stop at the same place. Yaw is
/// left as the tracker sent it.
const MAX_TRACK_PITCH_DEG: f32 = 90.0;
const MAX_TRACK_ROLL_DEG: f32 = 45.0;

/// Startup tries these in order and keeps the first one that sends a frame.
#[cfg(windows)]
const PROBE_ORDER: [HeadTrackingSource; 2] = [HeadTrackingSource::TrackIr, HeadTrackingSource::OpenTrack];
#[cfg(not(windows))]
const PROBE_ORDER: [HeadTrackingSource; 1] = [HeadTrackingSource::OpenTrack];

/// What the listener is doing. `Idle` keeps polling only after a frame has
/// already arrived for the selected method.
#[derive(Clone, Copy)]
enum Phase {
    Idle,
    /// Automatic scan at startup. `index` is into [`PROBE_ORDER`].
    Probe { index: usize, until: Instant },
    /// The Retry button, for whichever method is selected.
    Retry { until: Instant },
}

struct Runtime {
    /// The startup probe has been kicked off. Later settings builds only read
    /// the dot - they do not scan again.
    started: bool,
    phase: Phase,
    tracker: Option<HeadTracker>,
    /// Which method `tracker` was started as. `None` when nothing is running.
    running: Option<HeadTrackingSource>,
    /// Empty when the last start succeeded. `HeadTrackerError`'s message otherwise.
    error: String,
    /// The newest frame. Kept when `poll` has nothing new, so the head
    /// doesn't snap back between packets. Cleared when the method restarts.
    latest: Option<HeadPose>,
    /// When the newest frame arrived. TrackIR mouse-look suppression uses
    /// this, so a successful start with no frames leaves the mouse alone.
    last_frame: Option<Instant>,
    /// The dot, the method name, or the Retry label needs rewriting.
    changed: bool,
    /// 3, 2, or 1 while a Retry listen is counting down. `None` when the
    /// button reads "Retry" and can be clicked.
    shown_countdown: Option<u8>,
}

static RUNTIME: Mutex<Runtime> = Mutex::new(Runtime {
    started: false,
    phase: Phase::Idle,
    tracker: None,
    running: None,
    error: String::new(),
    latest: None,
    last_frame: None,
    changed: false,
    shown_countdown: None,
});

pub struct HeadTrackingStatus {
    pub connected: bool,
    pub error: String,
}

/// Whether the running method has delivered a frame. Not connected with an
/// empty error means nothing has been tried yet.
pub fn status() -> HeadTrackingStatus {
    let runtime = RUNTIME.lock().unwrap();
    HeadTrackingStatus {
        connected: received(&runtime),
        error: runtime.error.clone(),
    }
}

/// Whether `source` is the method running now and a frame has arrived.
pub fn source_connected(source: HeadTrackingSource) -> bool {
    let runtime = RUNTIME.lock().unwrap();
    runtime.running == Some(source) && received(&runtime)
}

/// What the Retry button should read. "3", "2", "1" for the whole wait after
/// a click - the button ignores further clicks until it is "Retry" again.
/// The number is the time left, not whether the tracker has started.
pub fn retry_caption() -> &'static str {
    let runtime = RUNTIME.lock().unwrap();
    let Phase::Retry { until } = runtime.phase else { return "Retry" };
    match countdown_digit(until) {
        2 => "2",
        1 => "1",
        _ => "3",
    }
}

/// `true` once since the last take, when the dot, the method name, or the
/// Retry label changed. Clears the flag.
pub fn take_changed() -> bool {
    let mut runtime = RUNTIME.lock().unwrap();
    let changed = runtime.changed;
    runtime.changed = false;
    changed
}

/// Starts the startup scan the first time settings is built. Opening the
/// pause menu later rebuilds that UI and must not scan again.
pub fn ensure_started(app: &App) {
    let started = RUNTIME.lock().unwrap().started;
    if started {
        return;
    }
    let mut runtime = RUNTIME.lock().unwrap();
    runtime.started = true;
    start_probe_at(&mut runtime, app, 0);
}

/// The player picked another method. Stops a scan or a retry so a late frame
/// from the previous method cannot select itself. Does not start the new one
/// - that waits for Retry, unless this method is already the live one.
pub fn selection_changed() {
    let mut runtime = RUNTIME.lock().unwrap();
    if !matches!(runtime.phase, Phase::Idle) {
        runtime.phase = Phase::Idle;
    }
    runtime.shown_countdown = None;
    runtime.changed = true;
}

/// Listens to the method selected in settings for [`RETRY_WINDOW`]. The first
/// frame marks it active and ends the wait; the window ending without one
/// leaves it inactive.
pub fn retry(app: &App) {
    if matches!(RUNTIME.lock().unwrap().phase, Phase::Retry { .. }) {
        return;
    }
    let source = GAME_SETTINGS.lock().unwrap().head_tracking;
    let mut runtime = RUNTIME.lock().unwrap();
    let until = Instant::now() + RETRY_WINDOW;
    // The countdown runs even when the program is not there to connect to,
    // so the button always leaves "Retry" for 3, 2, 1.
    if !start_listening(&mut runtime, app, source, Phase::Retry { until }) {
        runtime.phase = Phase::Retry { until };
    }
    runtime.shown_countdown = Some(3);
    runtime.changed = true;
}

/// Advances a startup scan or a retry, and keeps the newest frame once the
/// selected method is active. Call once a frame.
pub fn poll(app: &App) {
    ensure_started(app);
    let selected = GAME_SETTINGS.lock().unwrap().head_tracking;
    let switch_to = {
        let mut runtime = RUNTIME.lock().unwrap();
        drive(&mut runtime, app, selected)
    };
    if let Some(source) = switch_to {
        GAME_SETTINGS.lock().unwrap().head_tracking = source;
        RUNTIME.lock().unwrap().changed = true;
    }
}

/// TrackIR is the selected method, it has delivered a frame, and one has
/// arrived within [`TRACKIR_RECEIVING_WINDOW`]. OpenTrack frames, and a
/// TrackIR start that has not delivered a pose, are false.
pub fn trackir_receiving() -> bool {
    let selected = GAME_SETTINGS.lock().unwrap().head_tracking;
    if selected != HeadTrackingSource::TrackIr {
        return false;
    }
    let runtime = RUNTIME.lock().unwrap();
    runtime.running == Some(HeadTrackingSource::TrackIr)
        && received(&runtime)
        && runtime.last_frame.is_some_and(|at| at.elapsed() < TRACKIR_RECEIVING_WINDOW)
}

/// The pose the cockpit should be using. Zeros until a frame has arrived
/// since the method last started, and while the selected method is not the
/// one that frame came from. Rotations are degrees, position centimetres,
/// with the crate's own signs: right, up, and toward the screen are positive.
/// Pitch is held to ±[`MAX_TRACK_PITCH_DEG`] and roll to ±[`MAX_TRACK_ROLL_DEG`].
pub fn tracked_pose() -> HeadPose {
    let selected = GAME_SETTINGS.lock().unwrap().head_tracking;
    let runtime = RUNTIME.lock().unwrap();
    if runtime.running != Some(selected) || !received(&runtime) {
        return HeadPose::default();
    }
    let mut pose = runtime.latest.unwrap_or_default();
    pose.pitch = pose.pitch.clamp(-MAX_TRACK_PITCH_DEG, MAX_TRACK_PITCH_DEG);
    pose.roll = pose.roll.clamp(-MAX_TRACK_ROLL_DEG, MAX_TRACK_ROLL_DEG);
    pose
}

/// Whole seconds still to show, counting 3 then 2 then 1 across
/// [`RETRY_WINDOW`]. A fraction under 3.0 still reads as 3.
fn countdown_digit(until: Instant) -> u8 {
    let left = until.saturating_duration_since(Instant::now());
    let elapsed = RETRY_WINDOW.saturating_sub(left);
    3u8.saturating_sub(elapsed.as_secs().min(2) as u8)
}

fn received(runtime: &Runtime) -> bool {
    runtime.tracker.as_ref().is_some_and(|tracker| tracker.status())
}

fn take_frame(runtime: &mut Runtime) {
    let Some(tracker) = runtime.tracker.as_mut() else { return };
    if let Some(pose) = tracker.poll() {
        runtime.latest = Some(pose);
        runtime.last_frame = Some(Instant::now());
    }
}

/// `Some(source)` when the startup scan should select `source`.
fn drive(runtime: &mut Runtime, app: &App, selected: HeadTrackingSource) -> Option<HeadTrackingSource> {
    match runtime.phase {
        Phase::Idle => {
            if runtime.running == Some(selected) && received(runtime) {
                take_frame(runtime);
            }
            None
        }
        Phase::Retry { until } => {
            take_frame(runtime);
            let digit = countdown_digit(until);
            if runtime.shown_countdown != Some(digit) {
                runtime.shown_countdown = Some(digit);
                runtime.changed = true;
            }
            // The button stays on 3, 2, 1 for the whole window, even if a
            // frame already arrived and the dot turned green.
            if Instant::now() >= until {
                runtime.phase = Phase::Idle;
                runtime.shown_countdown = None;
                runtime.changed = true;
            }
            None
        }
        Phase::Probe { index, until } => {
            take_frame(runtime);
            if received(runtime) {
                runtime.phase = Phase::Idle;
                runtime.changed = true;
                return Some(PROBE_ORDER[index]);
            }
            if Instant::now() >= until {
                start_probe_at(runtime, app, index + 1);
            }
            None
        }
    }
}

fn start_probe_at(runtime: &mut Runtime, app: &App, index: usize) {
    for (index, source) in PROBE_ORDER.iter().copied().enumerate().skip(index) {
        let until = Instant::now() + RETRY_WINDOW;
        if start_listening(runtime, app, source, Phase::Probe { index, until }) {
            return;
        }
    }
    runtime.phase = Phase::Idle;
    runtime.changed = true;
}

/// Starts `source` and enters `phase`. False when that method cannot start;
/// the tracker is left stopped.
fn start_listening(runtime: &mut Runtime, app: &App, source: HeadTrackingSource, phase: Phase) -> bool {
    runtime.latest = None;
    runtime.last_frame = None;
    let method = match method(app, source) {
        Ok(method) => method,
        Err(message) => {
            runtime.tracker = None;
            runtime.running = None;
            runtime.error = message;
            return false;
        }
    };
    let current = runtime.tracker.take();
    match install(current, method) {
        Ok(tracker) => {
            runtime.tracker = Some(tracker);
            runtime.running = Some(source);
            runtime.error.clear();
            runtime.phase = phase;
            runtime.changed = true;
            true
        }
        Err(error) => {
            runtime.tracker = None;
            runtime.running = None;
            runtime.error = error.to_string();
            false
        }
    }
}

fn install(current: Option<HeadTracker>, method: HeadTrackerMethod) -> Result<HeadTracker, head_tracking_rs::HeadTrackerError> {
    match current {
        Some(tracker) => tracker.set_origin_method(method),
        None => HeadTracker::new(method),
    }
}

fn method(app: &App, source: HeadTrackingSource) -> Result<HeadTrackerMethod, String> {
    match source {
        HeadTrackingSource::OpenTrack => Ok(HeadTrackerMethod::OpenTrack {
            port: DEFAULT_PORT,
            address: IpAddr::V4(Ipv4Addr::LOCALHOST),
        }),
        HeadTrackingSource::TrackIr => trackir_method(app),
    }
}

#[cfg(windows)]
fn trackir_method(app: &App) -> Result<HeadTrackerMethod, String> {
    Ok(HeadTrackerMethod::TrackIR {
        id: TRACKIR_PROFILE_ID,
        axes: head_tracking_rs::AxisSelector::All,
        window_handle: game_window_handle(app),
    })
}

#[cfg(not(windows))]
fn trackir_method(_app: &App) -> Result<HeadTrackerMethod, String> {
    Err("TrackIR is only available on Windows".to_owned())
}

/// The game window's HWND. `None` if it cannot be read - TrackIR then falls
/// back to the console window, which the real software may reject.
#[cfg(windows)]
fn game_window_handle(app: &App) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    let handle = app.window_manager.canvas.window().window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(win32) => Some(win32.hwnd.get()),
        _ => None,
    }
}
