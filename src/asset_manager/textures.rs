use std::collections::{HashMap, HashSet};

use wgpu::{BindGroup, BindGroupLayout, Device, Queue};

use super::folder::{asset_path, normalize};
use crate::rendering::models::textures::Texture;

/// An image ready to draw on a model, in place of its own texture - see
/// `Material::texture`. It repeats past its edges, so a mesh whose texture
/// coordinates go past 0..1 shows it tiled.
pub struct ModelTexture {
    pub texture: Texture,
    pub(crate) bind_group: BindGroup,
}

/// Every model texture in memory, by its path inside the assets folder - each
/// read once, however many nodes use it. A scene's load with it (under the
/// loading screen) and go when a scene that doesn't use them opens.
pub struct TextureCache {
    loaded: HashMap<String, ModelTexture>,
    /// Couldn't be loaded - said once, not tried again until the next scene.
    failed: HashSet<String>,
    /// What a model's texture is bound with - its bind group 0.
    layout: BindGroupLayout,
}

impl TextureCache {
    pub(crate) fn new(layout: BindGroupLayout) -> Self {
        Self { loaded: HashMap::new(), failed: HashSet::new(), layout }
    }

    /// The texture `path`, if it's loaded.
    pub fn get(&self, path: &str) -> Option<&ModelTexture> {
        self.loaded.get(&normalize(path))
    }

    /// Loads each of `paths` not loaded yet - the ones that turned up while a
    /// scene runs (a node spawned with a new one).
    pub(crate) fn load_missing<'a>(&mut self, device: &Device, queue: &Queue, paths: impl IntoIterator<Item = &'a str>) {
        for path in paths {
            let key = normalize(path);
            if self.loaded.contains_key(&key) || self.failed.contains(&key) {
                continue;
            }
            match load_texture_file(&key, device, queue, &self.layout) {
                Ok(texture) => {
                    self.loaded.insert(key, texture);
                }
                Err(error) => {
                    eprintln!("{error}");
                    self.failed.insert(key);
                }
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

    /// Takes the textures a scene's loading thread loaded.
    pub(crate) fn adopt(&mut self, textures: Vec<(String, Result<ModelTexture, String>)>) {
        for (key, texture) in textures {
            match texture {
                Ok(texture) => {
                    self.loaded.insert(key, texture);
                }
                Err(error) => {
                    eprintln!("{error}");
                    self.failed.insert(key);
                }
            }
        }
    }

    /// Drops every texture but `keep`'s - the scene opening's.
    pub(crate) fn keep_only<'a>(&mut self, keep: impl IntoIterator<Item = &'a String>) {
        let keep: HashSet<String> = keep.into_iter().map(|path| normalize(path)).collect();
        self.loaded.retain(|key, _| keep.contains(key));
        self.failed.clear();
    }

    pub(crate) fn layout(&self) -> &BindGroupLayout {
        &self.layout
    }
}

/// Reads the texture `path` onto the GPU, repeating past its edges, its bind
/// group made for `layout` - without the cache, so a scene's loading thread
/// can (see TextureCache::adopt).
pub(crate) fn load_texture_file(path: &str, device: &Device, queue: &Queue, layout: &BindGroupLayout) -> Result<ModelTexture, String> {
    let fail = |error: &dyn std::fmt::Display| format!("failed to load texture '{path}': {error}");
    let bytes = std::fs::read(asset_path(path)).map_err(|error| fail(&error))?;
    let decoded = image::load_from_memory(&bytes).map_err(|error| fail(&error))?;
    // Repeating, with its mipmaps - see Texture::from_image.
    let texture = Texture::from_image(&decoded, device, queue, Some(path)).map_err(|error| fail(&error))?;
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(path),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&texture.view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&texture.sampler) },
        ],
    });
    Ok(ModelTexture { texture, bind_group })
}
