use std::collections::{HashMap, HashSet};

use wgpu::{BindGroup, BindGroupLayout, Device, Queue};

use super::folder::{asset_path, normalize};
use crate::rendering::models::textures::Texture;

/// An image on the GPU, ready to draw.
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

/// Every image in memory, by its path inside the assets folder - each loaded
/// once, however many things draw it. A scene's load with it and go when a
/// scene that doesn't use them opens.
pub struct ImageCache {
    loaded: HashMap<String, Image>,
    /// Couldn't be loaded - said once, not tried again until the next scene.
    failed: HashSet<String>,
    /// What an image's bind group is made for - the UI image shaders'.
    layout: BindGroupLayout,
}

impl ImageCache {
    pub(crate) fn new(layout: BindGroupLayout) -> Self {
        Self { loaded: HashMap::new(), failed: HashSet::new(), layout }
    }

    /// The image `path`, if it's loaded.
    pub fn get(&self, path: &str) -> Option<&Image> {
        self.loaded.get(&normalize(path))
    }

    /// The image `path`, loaded now if it isn't yet.
    pub fn load(&mut self, device: &Device, queue: &Queue, path: &str) -> Result<&Image, String> {
        let key = normalize(path);
        if !self.loaded.contains_key(&key) {
            let image = load_image_file(&key, device, queue, &self.layout)?;
            self.loaded.insert(key.clone(), image);
        }
        Ok(&self.loaded[&key])
    }

    /// Loads each of `paths` not loaded yet - the ones that turned up while a
    /// scene runs.
    pub(crate) fn load_missing<'a>(&mut self, device: &Device, queue: &Queue, paths: impl IntoIterator<Item = &'a str>) {
        for path in paths {
            let key = normalize(path);
            if self.loaded.contains_key(&key) || self.failed.contains(&key) {
                continue;
            }
            if let Err(error) = self.load(device, queue, &key) {
                eprintln!("{error}");
                self.failed.insert(key);
            }
        }
    }

    /// Which of `paths` aren't loaded - for a scene's loading thread.
    pub(crate) fn missing_from<'a>(&self, paths: impl IntoIterator<Item = &'a String>) -> Vec<String> {
        let mut missing: Vec<String> = paths.into_iter()
            .map(|path| normalize(path))
            .filter(|key| !self.loaded.contains_key(key))
            .collect();
        missing.sort();
        missing.dedup();
        missing
    }

    /// Takes the images a scene's loading thread loaded.
    pub(crate) fn adopt(&mut self, images: Vec<(String, Result<Image, String>)>) {
        for (key, image) in images {
            match image {
                Ok(image) => {
                    self.loaded.insert(key, image);
                }
                Err(error) => {
                    eprintln!("{error}");
                    self.failed.insert(key);
                }
            }
        }
    }

    /// Drops every image but `keep`'s - the scene opening's.
    pub(crate) fn keep_only<'a>(&mut self, keep: impl IntoIterator<Item = &'a String>) {
        let keep: HashSet<String> = keep.into_iter().map(|path| normalize(path)).collect();
        self.loaded.retain(|key, _| keep.contains(key));
        self.failed.clear();
    }

    pub(crate) fn layout(&self) -> &BindGroupLayout {
        &self.layout
    }
}

/// Reads the image `path` onto the GPU, its bind group made for `layout` -
/// without the cache, so a scene's loading thread can (see ImageCache::adopt).
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
