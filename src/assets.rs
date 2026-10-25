//! The project's assets folder - `app.assets`.
//!
//! Every file in it is found when the app starts (only the names - nothing is
//! read yet), so the game refers to any of them by its path inside the
//! folder, e.g. a node's `Model::new("models/f16.glb")`. A model is loaded the
//! first time a scene uses it - before the scene starts, under the loading
//! screen (see SceneManager) - and dropped once no open scene uses it.
//!
//! Images (a UI image, `UiNode::image("ui/logo.png", ..)`) work the same way:
//! the ones a scene's UI uses load with it, under the loading screen, and go
//! when a scene that doesn't use them opens. One that turns up while the
//! scene runs loads the first frame it's drawn. Each is loaded once, however
//! many things draw it.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use wgpu::{BindGroup, BindGroupLayout, Device, Queue};

use crate::rendering::instance_management::ModelDataInstance;
use crate::rendering::models::model::Model;
use crate::rendering::models::textures::Texture;
use crate::resources::ModelSource;

/// The folder `Assets::new` uses unless `set_root` says otherwise.
pub const DEFAULT_ROOT: &str = "assets";

/// The assets folder, for code that reads a file without an `App` at hand
/// (UI images, particle textures, skybox faces on a loading thread) - kept
/// in step with `Assets::set_root`.
static ROOT: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Where the file `relative` (a path inside the assets folder) is on disk.
pub fn asset_path(relative: impl AsRef<Path>) -> PathBuf {
    let root = ROOT.read().ok().and_then(|root| root.clone()).unwrap_or_else(|| PathBuf::from(DEFAULT_ROOT));
    root.join(relative)
}

pub struct Assets {
    root: PathBuf,
    /// Every file under `root`, by its path inside it ("models/f16.glb",
    /// always with `/`) - to where it is on disk.
    files: HashMap<String, PathBuf>,

    /// Models with at least one node drawing them - each with its GPU
    /// instance buffer - by key (see ModelSource::key).
    pub(crate) game_models: HashMap<String, ModelDataInstance>,
    /// Models loaded but not drawn by anything yet - a model moves into
    /// `game_models` when a node first uses it (see
    /// render_bridge::register_static_model): an instance buffer can't exist
    /// with zero instances (wgpu rejects a zero-size buffer).
    pub(crate) loaded_models: HashMap<String, Model>,
    /// Keys of the models the current scene loaded from files - dropped when
    /// it ends, see resources::unload_level_models.
    pub(crate) level_model_keys: HashSet<String>,
    /// Named images (see resources::register_texture).
    pub(crate) textures: HashMap<String, Texture>,
    /// Images by their path inside the folder (see `load_image`) - what UI
    /// images draw.
    images: HashMap<String, Image>,
    /// Images that couldn't be loaded - reported once, not tried again every
    /// frame. Cleared when a scene opens, so a fixed file gets another go.
    failed_images: HashSet<String>,
    /// What an image's bind group is made for - the Ui image shaders' layout.
    image_layout: BindGroupLayout,
}

/// An image on the GPU, ready to draw - see `Assets::load_image`.
pub struct Image {
    pub texture: Texture,
    pub(crate) bind_group: BindGroup,
}

impl Image {
    /// Its size in pixels.
    pub fn size(&self) -> (u32, u32) {
        let size = self.texture.texture.size();
        (size.width, size.height)
    }
}

impl Assets {
    /// `image_layout`: the bind group layout images are drawn with (see
    /// `Image`).
    pub(crate) fn new(image_layout: BindGroupLayout) -> Self {
        let mut assets = Self {
            root: PathBuf::new(),
            files: HashMap::new(),
            game_models: HashMap::new(),
            loaded_models: HashMap::new(),
            level_model_keys: HashSet::new(),
            textures: HashMap::new(),
            images: HashMap::new(),
            failed_images: HashSet::new(),
            image_layout,
        };
        assets.set_root(DEFAULT_ROOT);
        assets
    }

    /// Uses the folder at `path` (relative to where the game runs, or
    /// absolute) as the assets folder, and finds every file in it.
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

    /// Finds every file in the assets folder again - for files added while
    /// the game runs.
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
                    self.files.insert(Self::normalize(&relative.to_string_lossy()), path);
                }
            }
        }
    }

    /// Whether `relative` is a file in the assets folder.
    pub fn contains(&self, relative: &str) -> bool {
        self.files.contains_key(&Self::normalize(relative))
    }

    /// Every file in the assets folder, by its path inside it.
    pub fn files(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    /// Where `relative` is on disk - the assets folder joined with it,
    /// whether or not the file exists.
    pub fn path(&self, relative: &str) -> PathBuf {
        self.files.get(&Self::normalize(relative)).cloned().unwrap_or_else(|| self.root.join(relative))
    }

    /// The model file `name` - a path inside the assets folder - with the
    /// default options (a node's `Model` adds its own - see
    /// `Model::double_sided`). None when there's no such file (a name of a
    /// model made in code instead, see resources::register_primitive_model).
    pub(crate) fn model_source(&self, name: &str) -> Option<ModelSource> {
        let path = self.files.get(&Self::normalize(name)).cloned()
            .or_else(|| Some(self.root.join(name)).filter(|path| path.is_file()))?;
        Some(ModelSource::asset(path))
    }

    /// Whether a model is loaded under `key` (see ModelSource::key) -
    /// drawn or not.
    pub(crate) fn is_model_loaded(&self, key: &str) -> bool {
        self.game_models.contains_key(key) || self.loaded_models.contains_key(key)
    }

    /// The image `path` (inside the assets folder), if it's loaded.
    pub fn image(&self, path: &str) -> Option<&Image> {
        self.images.get(&Self::normalize(path))
    }

    /// The image `path` (inside the assets folder), loading it now if it
    /// isn't yet - for one a scene's UI doesn't use while it's built, to
    /// have it ready before it's drawn. Kept until a scene that doesn't use
    /// it opens.
    pub fn load_image(&mut self, device: &Device, queue: &Queue, path: &str) -> Result<&Image, String> {
        let key = Self::normalize(path);
        if !self.images.contains_key(&key) {
            let image = load_image_file(&key, device, queue, &self.image_layout)?;
            self.images.insert(key.clone(), image);
        }
        Ok(&self.images[&key])
    }

    /// Loads each of `paths` not loaded yet, now - a file that can't be is
    /// reported, once. For images that turned up while a scene runs.
    pub(crate) fn load_missing_images<'a>(&mut self, device: &Device, queue: &Queue, paths: impl IntoIterator<Item = &'a str>) {
        for path in paths {
            let key = Self::normalize(path);
            if self.images.contains_key(&key) || self.failed_images.contains(&key) {
                continue;
            }
            if let Err(error) = self.load_image(device, queue, &key) {
                eprintln!("{error}");
                self.failed_images.insert(key);
            }
        }
    }

    /// Which of `paths` aren't loaded - for a scene's loading step.
    pub(crate) fn images_to_load<'a>(&self, paths: impl IntoIterator<Item = &'a String>) -> Vec<String> {
        let mut missing: Vec<String> = paths.into_iter()
            .map(|path| Self::normalize(path))
            .filter(|key| !self.images.contains_key(key))
            .collect();
        missing.sort();
        missing.dedup();
        missing
    }

    /// Takes images a scene's loading thread loaded (see
    /// `load_image_file`) - one that failed is reported, and not tried again
    /// until the next scene.
    pub(crate) fn adopt_images(&mut self, images: Vec<(String, Result<Image, String>)>) {
        for (key, image) in images {
            match image {
                Ok(image) => {
                    self.images.insert(key, image);
                }
                Err(error) => {
                    eprintln!("{error}");
                    self.failed_images.insert(key);
                }
            }
        }
    }

    /// Drops every image but `keep`'s (paths the scene opening uses) - so
    /// a scene starts with only what it draws, and keeps what it shares with
    /// the one before.
    pub(crate) fn keep_only_images<'a>(&mut self, keep: impl IntoIterator<Item = &'a String>) {
        let keep: HashSet<String> = keep.into_iter().map(|path| Self::normalize(path)).collect();
        self.images.retain(|key, _| keep.contains(key));
        self.failed_images.clear();
    }

    /// See `image_layout` - for a loading thread.
    pub(crate) fn image_layout(&self) -> &BindGroupLayout {
        &self.image_layout
    }

    fn normalize(path: &str) -> String {
        path.replace('\\', "/").trim_start_matches("./").to_owned()
    }
}

/// Reads the image `path` (inside the assets folder) onto the GPU, with a
/// bind group made for `layout` - no `Assets` needed, so a scene's loading
/// thread can do it (see `Assets::adopt_images`).
pub(crate) fn load_image_file(path: &str, device: &Device, queue: &Queue, layout: &BindGroupLayout) -> Result<Image, String> {
    let bytes = std::fs::read(asset_path(path)).map_err(|error| format!("failed to load image '{path}': {error}"))?;
    let decoded = image::load_from_memory(&bytes).map_err(|error| format!("failed to load image '{path}': {error}"))?;
    let texture = Texture::from_image(&decoded, device, queue, Some(path)).map_err(|error| format!("failed to load image '{path}': {error}"))?;
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(path),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&texture.view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&texture.sampler) },
        ],
    });
    Ok(Image { texture, bind_group })
}
