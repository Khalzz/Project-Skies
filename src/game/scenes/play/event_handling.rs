use std::{collections::HashMap, time::Duration};
use serde::Deserialize;

use crate::engine::audio::audio::{self, Audio};
use crate::app::App;
use crate::engine::audio::subtitles::{Subtitle, SubtitleData};
use crate::engine::scene_manager::scene::Scene;
use super::animation_tracks::{self, CameraTrack, Object3DTrack, UiTrack};

#[derive(Debug, Deserialize)]
pub struct AudioFile {
    file_name: String,
    #[serde(default)] // this will set activated as false without the need of being setted in the ron
    timer: f64,
    #[serde(default)] // this will set activated as false without the need of being setted in the ron
    played: Vec<u64>
}

#[derive(Debug, Deserialize)]
enum EventType {
    PlayAudio(AudioFile)
}

#[derive(Debug, Deserialize)]
pub struct Event {
    event_type: EventType,
    #[serde(default)] // this will set activated as false without the need of being setted in the ron
    activated: bool
}

/// A `[start_time, end_time)` window (ms, same convention as every other
/// track's own start/end) during which the player has no input at all -
/// flight controls, and specifically the pause menu itself (see
/// `GameLogic::update`'s own use of `EventSystem::is_input_locked`) - only a
/// "hold ESC to skip" affordance still works. Deliberately its own thing
/// rather than reusing `is_cinematic_camera_active` (a `CameraTrack::Shot`'s
/// own window): a scripted sequence built purely from `object_3d_tracks` (no
/// camera work at all) still wants input locked out for its duration, and
/// conversely a `Shot` doesn't *have* to lock input just because it's driving
/// the camera - these are two independently-authorable concepts that happen
/// to often overlap.
#[derive(Debug, Deserialize)]
pub struct InputLock {
    #[serde(default)]
    pub start_time: u64,
    pub end_time: u64,
}

#[derive(Debug, Deserialize)]
pub struct EventSystem {
    pub event_list: HashMap<u64, Event>,
    // Keyframed parameter animation - see animation_tracks.rs. Kept as separate
    // lists (rather than folded into `event_list`) since a track is continuous
    // ("what's the value right now") while an Event is a one-shot trigger that
    // latches once fired - see `handle_events` vs `apply_tracks`.
    #[serde(default)]
    pub object_3d_tracks: Vec<Object3DTrack>,
    #[serde(default)]
    pub ui_tracks: Vec<UiTrack>,
    #[serde(default)]
    pub camera_tracks: Vec<CameraTrack>,
    #[serde(default)]
    pub input_locks: Vec<InputLock>,
}

impl EventSystem {
    pub fn new(file_path: &Option<String>) -> Result<EventSystem, String> {
        println!("{}", file_path.to_owned().unwrap_or("None".to_string()));

        match file_path {
            Some(path) => {
                match std::fs::read_to_string(path.to_owned() + "/level_planning.ron") {
                    Ok(ron_result_string) => {
                        match ron::from_str::<EventSystem>(&ron_result_string) {
                            Ok(mut event_system) => {
                                animation_tracks::normalize_all(
                                    &mut event_system.object_3d_tracks,
                                    &mut event_system.ui_tracks,
                                    &mut event_system.camera_tracks,
                                );
                                Ok(event_system)
                            },
                            Err(error) => Err(format!("Something went wrong structuring the event: {}", error)),
                        }
                    }
                    Err(err) => Err(format!("Something went wrong reading the file: {}", err)),
                }
            },
            None => Err("There is no scene openned yet".to_string())
        }
    }

    /// Applies every `object_3d_tracks`/`ui_tracks`/`camera_tracks` entry for the
    /// current scene time - see `animation_tracks` for the keyframe/lerp model.
    /// Stateless (unlike `handle_events`' `activated` latch): every frame just
    /// recomputes "what should this target's value be right now," so scrubbing
    /// `seconds` backward (e.g. a paused/rewound scene) works with no extra logic.
    pub fn apply_tracks(&self, seconds: f64, scene: &mut Scene, app: &mut App) {
        let game_time_ms = Duration::from_secs_f64(seconds).as_millis() as u64;
        animation_tracks::apply_all(&self.object_3d_tracks, &self.ui_tracks, &self.camera_tracks, game_time_ms, scene, app);
    }

    /// Whether a `CameraTrack::LookAt` cinematic shot is currently driving the
    /// camera - `GameLogic::update` uses this to lock player flight controls off
    /// for the same window the shot runs in, rather than duplicating that window
    /// as a separate hardcoded value in Rust.
    pub fn is_cinematic_camera_active(&self, seconds: f64) -> bool {
        let game_time_ms = Duration::from_secs_f64(seconds).as_millis() as u64;
        animation_tracks::any_cinematic_active(&self.camera_tracks, game_time_ms)
    }

    /// The end time (ms) of whichever `input_locks` window currently contains
    /// `seconds`, if any - `None` means input isn't locked right now. Returns
    /// the *latest* end among any that overlap, in the unusual case more than
    /// one does, so a "hold ESC to skip" (see `GameLogic::update`) always
    /// jumps past all of them at once rather than into a gap between two.
    pub fn input_lock_end(&self, seconds: f64) -> Option<u64> {
        let game_time_ms = Duration::from_secs_f64(seconds).as_millis() as u64;
        self.input_locks.iter()
            .filter(|lock| game_time_ms >= lock.start_time && game_time_ms < lock.end_time)
            .map(|lock| lock.end_time)
            .max()
    }


    pub fn handle_events(&mut self, seconds: f64, app: &mut App, subtitle_system: &mut Subtitle) {  
        let duration = Duration::from_secs_f64(seconds).as_millis();
        
        for (millis, event) in &mut self.event_list {
            if duration > (*millis).into() {
                match &mut event.event_type {
                    EventType::PlayAudio(audio_file) => {
                        EventSystem::handle_play_audio(event.activated, app, audio_file, subtitle_system);
                    },
                    _ => {

                    }
                }
                event.activated = true;
            }
            
        }
    }

    pub fn handle_play_audio(activated: bool, app: &mut App, audio_file: &mut AudioFile, subtitle_system: &mut Subtitle) {
        if !activated {
            // run once for each element
            app.audio.play_audio(audio_file.file_name.clone());
        } else {
            // run from now untill the end
            audio_file.timer += app.time.delta_time as f64;
            let duration = Duration::from_secs_f64(audio_file.timer).as_millis();

            match std::fs::read_to_string(audio_file.file_name.clone() + "/subtitles.ron") {
                Ok(ron_result_string) => {
                    match ron::from_str::<SubtitleData>(&ron_result_string) {
                        Ok(subtitles) => {
                            for (time, entry) in subtitles.subtitles {
                                if !audio_file.played.contains(&(time as u64)) && duration > time.into() {
                                    subtitle_system.add_text(&entry.text, entry.duration, app);
                                    audio_file.played.push(time as u64);
                                }
                            }
                        },
                        Err(error) => {
                            eprintln!("Something went wrong structuring the event: {}", error)
                        },
                    };
                }
                Err(err) => {
                    eprintln!("Something went wrong opening the file: {}", err)
                },
            };
        }
    }
}

