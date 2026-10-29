use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

pub const DEFAULT_ROOT: &str = "assets";

static ROOT: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Where the file `relative` (a path inside the assets folder) is on disk -
/// for code without the `AssetManager` at hand (a loading thread).
pub fn asset_path(relative: impl AsRef<Path>) -> PathBuf {
    let root = ROOT.read().ok().and_then(|root| root.clone()).unwrap_or_else(|| PathBuf::from(DEFAULT_ROOT));
    root.join(relative)
}

/// A path inside the assets folder the one way it's kept: `/` between
/// folders, no leading `./`.
pub(crate) fn normalize(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches("./").to_owned()
}

/// The assets folder, and every file in it - found once, by name, without
/// reading any of them.
pub struct AssetFolder {
    root: PathBuf,
    files: HashMap<String, PathBuf>,
}

impl AssetFolder {
    pub(crate) fn new(root: impl Into<PathBuf>) -> Self {
        let mut folder = Self { root: PathBuf::new(), files: HashMap::new() };
        folder.set_root(root);
        folder
    }

    /// Uses the folder at `path` (relative to where the game runs, or
    /// absolute), and finds every file in it.
    pub fn set_root(&mut self, path: impl Into<PathBuf>) {
        self.root = path.into();
        if let Ok(mut root) = ROOT.write() {
            *root = Some(self.root.clone());
        }
        self.refresh();
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Finds every file again - for files added while the game runs.
    pub fn refresh(&mut self) {
        self.files.clear();
        let mut pending = vec![self.root.clone()];
        while let Some(folder) = pending.pop() {
            let Ok(entries) = std::fs::read_dir(&folder) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                } else if let Ok(relative) = path.strip_prefix(&self.root) {
                    self.files.insert(normalize(&relative.to_string_lossy()), path);
                }
            }
        }
    }

    pub fn contains(&self, relative: &str) -> bool {
        self.files.contains_key(&normalize(relative))
    }

    /// Every file, by its path inside the folder.
    pub fn files(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    /// Where `relative` is on disk, whether or not the file exists.
    pub fn path(&self, relative: &str) -> PathBuf {
        self.files.get(&normalize(relative)).cloned().unwrap_or_else(|| self.root.join(relative))
    }

    /// Where `relative` is on disk - None when there's no such file.
    pub(crate) fn find(&self, relative: &str) -> Option<PathBuf> {
        self.files.get(&normalize(relative)).cloned()
            .or_else(|| Some(self.root.join(relative)).filter(|path| path.is_file()))
    }
}
