use std::collections::{HashMap, HashSet};
use std::io::Cursor;

use rodio::source::Buffered;
use rodio::{Decoder, Source};

use super::folder::{asset_path, normalize};

/// A sound file, ready to play - decoded as it's first played, and that
/// decoding shared by every play after (cloning one is cheap).
pub type Clip = Buffered<Decoder<Cursor<Vec<u8>>>>;

/// Every sound in memory, by its path inside the assets folder - each read
/// once, however many times it plays. Kept for the app's life.
#[derive(Default)]
pub struct SoundCache {
    loaded: HashMap<String, Clip>,
    /// Couldn't be loaded - said once, not tried again.
    failed: HashSet<String>,
}

impl SoundCache {
    /// The sound `path`, read now if it isn't yet - call it ahead of time to
    /// have a sound ready before it first plays.
    pub fn load(&mut self, path: &str) -> Result<Clip, String> {
        let key = normalize(path);
        if let Some(clip) = self.loaded.get(&key) {
            return Ok(clip.clone());
        }
        if self.failed.contains(&key) {
            return Err(format!("sound '{key}' couldn't be loaded"));
        }
        match read_clip(&key) {
            Ok(clip) => {
                self.loaded.insert(key, clip.clone());
                Ok(clip)
            }
            Err(error) => {
                self.failed.insert(key);
                Err(error)
            }
        }
    }

    /// `load`, saying why it couldn't only the first time - for playing a
    /// sound that might be missing every frame without flooding the console.
    pub(crate) fn load_for_playing(&mut self, path: &str) -> Option<Clip> {
        let failed_before = self.failed.contains(&normalize(path));
        match self.load(path) {
            Ok(clip) => Some(clip),
            Err(error) => {
                if !failed_before {
                    eprintln!("{error}");
                }
                None
            }
        }
    }

    /// Lets go of the sound `path` - read again the next time it plays.
    pub fn unload(&mut self, path: &str) {
        self.loaded.remove(&normalize(path));
    }
}

fn read_clip(path: &str) -> Result<Clip, String> {
    let bytes = std::fs::read(asset_path(path)).map_err(|error| format!("failed to load sound '{path}': {error}"))?;
    let decoder = Decoder::new(Cursor::new(bytes)).map_err(|error| format!("failed to load sound '{path}': {error}"))?;
    Ok(decoder.buffered())
}
