use std::{fs::File, io::BufReader, time::Duration};

use fs_extra::file;
use rodio::{source::SineWave, Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use sdl2::mixer::{self, InitFlag, Sdl2MixerContext, AUDIO_S16LSB, DEFAULT_CHANNELS};

// To add: a way to load all audio once the game starts

pub struct Audio {
    stream: OutputStream,
    stream_handle: OutputStreamHandle,
    sink: Sink
    // mixer_context: Sdl2MixerContext,
}

impl Audio {
    pub fn new() -> Self {
        let (stream, stream_handle) = OutputStream::try_default().unwrap();
        let sink = Sink::try_new(&stream_handle).unwrap();

        Self {
            stream,
            stream_handle,
            sink
            
        }
    }

    pub fn update(game_time: f64) {
        
    }

    pub fn play_audio(&mut self, mut file_string: String) {
        file_string += "/audio.ogg";

        let file = File::open(file_string);

        match file {
            Ok(result_file) => {
                let buffer = BufReader::new(result_file);

                match Decoder::new(buffer) {
                    Ok(source) => {
                        self.sink.append(source);
                        self.sink.play();
                    },
                    Err(err) => eprintln!("Error decoding the buffer: {}", err),
                }

            },
            Err(err) => eprintln!("Error Opening the file: {}", err),
        }
        
    }
}