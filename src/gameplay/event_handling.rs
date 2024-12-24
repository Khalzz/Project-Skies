use std::{collections::HashMap, time::Duration};
use serde::Deserialize;

use crate::audio::audio::{self, Audio};
use crate::app::App;

#[derive(Debug, Deserialize)]
struct AudioFile {
    file_name: String
}

#[derive(Debug, Deserialize)]
enum EventType {
    PlayAudio(AudioFile)
}

#[derive(Debug, Deserialize)]
struct Event {
    event_type: EventType,
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


    pub fn handle_events(&mut self, seconds: f64, app: &mut App) {  
        let duration = Duration::from_secs_f64(seconds).as_millis();
        
        for (millis, event) in &mut self.event_list {
            if duration > (*millis).into() && !event.activated {
                event.activated = true;
                match &event.event_type {
                    EventType::PlayAudio(audio_file) => {
                        app.audio.play_audio(audio_file.file_name.clone());
                        
                    },
                    _ => {}
                }
            }
            
        }
    }
}

