use std::{collections::HashMap, time::Duration};
use serde::Deserialize;

use crate::audio::audio::{self, Audio};
use crate::app::App;
use crate::audio::subtitles::{Subtitle, SubtitleData};

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

#[derive(Debug, Deserialize)]
pub struct EventSystem {
    pub event_list: HashMap<u64, Event>
}

impl EventSystem {
    pub fn new(file_path: &Option<String>) -> Result<EventSystem, String> {
        match file_path {
            Some(path) => {
                match std::fs::read_to_string(path.to_owned() + "/level_planning.ron") {
                    Ok(ron_result_string) => {
                        match ron::from_str::<EventSystem>(&ron_result_string) {
                            Ok(event_system) => Ok(event_system),
                            Err(error) => Err(format!("Something went wrong structuring the event: {}", error)),
                        }
                    }
                    Err(err) => Err(format!("Something went wrong reading the file: {}", err)),
                }
            },
            None => Err("There is no scene openned yet".to_string())
        }
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
                            for (time, subtitle) in subtitles.subtitles {
                                if !audio_file.played.contains(&(time as u64)) && duration > time.into() {
                                    subtitle_system.add_text(&subtitle, app);
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

