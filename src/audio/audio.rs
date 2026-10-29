//! The sounds playing now - `app.audio`. Started with `app.play_sound(path)`:
//!
//!     app.play_sound("voice/intro.ogg")
//!         .volume(0.8)
//!         .at(1.5, |app| show_subtitle(app, "We have to move."))
//!         .on_finish(|app| app.scene_manager.open_scene("level_1"));
//!
//!     let music = app.play_sound("music/theme.ogg").looping().id();
//!     app.audio.stop(music);
//!
//! Every sound plays on its own, at the same time as the others. Its
//! callbacks run on the main thread, with the whole `App`, the frame they're
//! due (see `App::run`).

use rodio::{OutputStream, OutputStreamHandle, Sink, Source};

use crate::app::App;
use crate::asset_manager::sounds::Clip;

/// Something to run when a sound gets to a point - given the whole app.
pub type SoundCallback = Box<dyn FnOnce(&mut App)>;

/// Which sound - from `Sound::id`, for `Audio::stop` and the like.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SoundId(u64);

/// One sound playing.
struct Playing {
    id: SoundId,
    sink: Sink,
    /// What it plays - kept to queue again for `Sound::looping`.
    clip: Clip,
    /// `at`'s callbacks, soonest first - each run once.
    cues: Vec<(f32, SoundCallback)>,
    on_finish: Option<SoundCallback>,
}

pub struct Audio {
    /// Where sound goes - None without an audio device: then nothing plays,
    /// and nothing else is any different.
    output: Option<(OutputStream, OutputStreamHandle)>,
    playing: Vec<Playing>,
    next_id: u64,
}

impl Audio {
    pub fn new() -> Self {
        let output = match OutputStream::try_default() {
            Ok(output) => Some(output),
            Err(error) => {
                eprintln!("No audio device ({error}) - the game runs without sound");
                None
            }
        };
        Self { output, playing: Vec::new(), next_id: 1 }
    }

    /// Starts `clip` - see `App::play_sound`, which finds it by path.
    pub fn play(&mut self, clip: Clip) -> Sound<'_> {
        let id = self.new_id();
        let Some((_, handle)) = &self.output else { return Sound { id, playing: None } };
        let sink = match Sink::try_new(handle) {
            Ok(sink) => sink,
            Err(error) => {
                eprintln!("A sound couldn't start: {error}");
                return Sound { id, playing: None };
            }
        };
        sink.append(clip.clone());
        self.playing.push(Playing { id, sink, clip, cues: Vec::new(), on_finish: None });
        Sound { id, playing: self.playing.last_mut() }
    }

    /// A sound that couldn't start - its settings do nothing.
    pub(crate) fn nothing(&mut self) -> Sound<'_> {
        Sound { id: self.new_id(), playing: None }
    }

    /// Stops `sound` - its callbacks don't run (`on_finish` is for a sound
    /// played to its end).
    pub fn stop(&mut self, sound: SoundId) {
        self.playing.retain(|playing| playing.id != sound);
    }

    /// Stops every sound, without their callbacks.
    pub fn stop_all(&mut self) {
        self.playing.clear();
    }

    pub fn pause(&mut self, sound: SoundId) {
        if let Some(playing) = self.find(sound) {
            playing.sink.pause();
        }
    }

    pub fn resume(&mut self, sound: SoundId) {
        if let Some(playing) = self.find(sound) {
            playing.sink.play();
        }
    }

    /// 1 is the sound as it is, 0 silent.
    pub fn set_volume(&mut self, sound: SoundId, volume: f32) {
        if let Some(playing) = self.find(sound) {
            playing.sink.set_volume(volume.max(0.0));
        }
    }

    /// Whether `sound` is still going - paused counts.
    pub fn is_playing(&self, sound: SoundId) -> bool {
        self.playing.iter().any(|playing| playing.id == sound)
    }

    /// The callbacks due now, in order - each sound's `at`s it has got to,
    /// then its `on_finish` once it's played to the end (and any `at` past
    /// its end, before that). Finished sounds are let go of.
    pub(crate) fn take_due(&mut self) -> Vec<SoundCallback> {
        let mut due = Vec::new();
        self.playing.retain_mut(|playing| {
            let finished = playing.sink.empty();
            let position = playing.sink.get_pos().as_secs_f32();
            while playing.cues.first().is_some_and(|(time, _)| finished || *time <= position) {
                due.push(playing.cues.remove(0).1);
            }
            if finished {
                due.extend(playing.on_finish.take());
            }
            !finished
        });
        due
    }

    fn find(&mut self, sound: SoundId) -> Option<&mut Playing> {
        self.playing.iter_mut().find(|playing| playing.id == sound)
    }

    fn new_id(&mut self) -> SoundId {
        let id = SoundId(self.next_id);
        self.next_id += 1;
        id
    }
}

/// A sound just started - set it up by chaining (see the module doc). Its
/// settings do nothing when it couldn't start (no file, no audio device).
pub struct Sound<'a> {
    id: SoundId,
    playing: Option<&'a mut Playing>,
}

impl Sound<'_> {
    /// 1 is the sound as it is, 0 silent.
    pub fn volume(self, volume: f32) -> Self {
        if let Some(playing) = &self.playing {
            playing.sink.set_volume(volume.max(0.0));
        }
        self
    }

    /// Plays over and over, until stopped - so it never finishes.
    pub fn looping(self) -> Self {
        if let Some(playing) = &self.playing {
            // The first play is already queued: after it, the same sound
            // without end.
            playing.sink.append(playing.clip.clone().repeat_infinite());
        }
        self
    }

    /// Runs `callback` once, when the sound gets `seconds` in.
    pub fn at(mut self, seconds: f32, callback: impl FnOnce(&mut App) + 'static) -> Self {
        if let Some(playing) = &mut self.playing {
            let index = playing.cues.partition_point(|(time, _)| *time <= seconds);
            playing.cues.insert(index, (seconds, Box::new(callback)));
        }
        self
    }

    /// Runs `callback` once, when the sound has played to its end - not when
    /// it's stopped.
    pub fn on_finish(mut self, callback: impl FnOnce(&mut App) + 'static) -> Self {
        if let Some(playing) = &mut self.playing {
            playing.on_finish = Some(Box::new(callback));
        }
        self
    }

    /// Which sound it is - for `Audio::stop` and the like.
    pub fn id(self) -> SoundId {
        self.id
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use rodio::{Decoder, Source};

    use super::*;

    /// `seconds` of silence as a WAV file's bytes - something to decode.
    fn silent_wav(seconds: u32) -> Vec<u8> {
        let rate = 8000u32;
        let data_len = rate * seconds * 2;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&rate.to_le_bytes());
        bytes.extend_from_slice(&(rate * 2).to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        bytes.resize(bytes.len() + data_len as usize, 0);
        bytes
    }

    fn clip() -> Clip {
        Decoder::new(Cursor::new(silent_wav(1))).unwrap().buffered()
    }

    /// A sound on a player that's never heard - it stays where it starts.
    /// `queued`: whether it has the sound in it (it's "finished" without).
    fn playing(id: u64, queued: bool, cues: &[f32], on_finish: bool) -> Playing {
        let (sink, _output) = Sink::new_idle();
        if queued {
            sink.append(clip());
        }
        Playing {
            id: SoundId(id),
            sink,
            clip: clip(),
            cues: cues.iter().map(|time| (*time, Box::new(|_: &mut App| {}) as SoundCallback)).collect(),
            on_finish: on_finish.then(|| Box::new(|_: &mut App| {}) as SoundCallback),
        }
    }

    fn audio(playing: Vec<Playing>) -> Audio {
        Audio { output: None, playing, next_id: 100 }
    }

    #[test]
    fn a_cue_runs_once_its_time_comes_and_not_before() {
        let mut audio = audio(vec![playing(1, true, &[0.0, 0.5], true)]);
        // At its start: the 0 s cue, not the 0.5 s one, no finish yet.
        assert_eq!(audio.take_due().len(), 1);
        assert_eq!(audio.take_due().len(), 0, "a cue runs once");
        assert!(audio.is_playing(SoundId(1)));
    }

    #[test]
    fn a_finished_sound_runs_what_it_has_left_then_its_on_finish_and_goes() {
        let mut audio = audio(vec![playing(1, false, &[0.5, 9.0], true)]);
        assert_eq!(audio.take_due().len(), 3, "both cues past where it ended, then on_finish");
        assert!(!audio.is_playing(SoundId(1)));
        assert_eq!(audio.take_due().len(), 0);
    }

    #[test]
    fn a_stopped_sound_runs_nothing() {
        let mut audio = audio(vec![playing(1, true, &[0.5], true)]);
        audio.stop(SoundId(1));
        assert!(!audio.is_playing(SoundId(1)));
        assert_eq!(audio.take_due().len(), 0);
    }

    #[test]
    fn cues_go_in_by_time_whatever_order_they_are_added_in() {
        let mut audio = audio(Vec::new());
        audio.playing.push(playing(7, true, &[], false));
        let sound = Sound { id: SoundId(7), playing: audio.playing.last_mut() };
        sound.at(2.0, |_| {}).at(0.0, |_| {}).at(1.0, |_| {});
        let times: Vec<f32> = audio.playing[0].cues.iter().map(|(time, _)| *time).collect();
        assert_eq!(times, [0.0, 1.0, 2.0]);
    }

    #[test]
    fn without_an_audio_device_a_sound_does_nothing_and_breaks_nothing() {
        let mut audio = audio(Vec::new());
        let id = audio.play(clip()).volume(0.5).looping().at(1.0, |_| {}).on_finish(|_| {}).id();
        assert!(!audio.is_playing(id));
        assert_eq!(audio.take_due().len(), 0);
    }
}
