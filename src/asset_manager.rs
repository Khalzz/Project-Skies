pub mod folder;
pub mod images;
pub mod models;
pub mod sounds;
pub mod textures;

pub use folder::{asset_path, AssetFolder, DEFAULT_ROOT};
pub(crate) use folder::normalize;
pub use images::{Image, ImageCache};
pub(crate) use images::load_image_file;
pub use models::ModelCache;
pub use sounds::{Clip, SoundCache};
pub use textures::{ModelTexture, TextureCache};
pub(crate) use textures::load_texture_file;

use std::sync::Arc;

use wgpu::BindGroupLayout;

use crate::resources::{DoubleSidedMeshes, MeshData, ModelLoad, ModelSource, ModelVertex};

/// Everything the game loads goes through here - `app.assets`.
///
/// The `folder` knows every file in the assets folder by name from the start,
/// but reads none of them. A model or image is read the first time a scene
/// uses it (under the loading screen), kept in its cache while any open scene
/// uses it, and dropped after - each loaded once, however many things use it.
pub struct AssetManager {
    pub folder: AssetFolder,
    pub(crate) models: ModelCache,
    pub images: ImageCache,
    pub sounds: SoundCache,
    pub textures: TextureCache,
}

impl AssetManager {
    /// `image_layout`: what images are drawn with (see `Image`);
    /// `texture_layout`: what a model's texture is (see `ModelTexture`).
    pub(crate) fn new(image_layout: BindGroupLayout, texture_layout: BindGroupLayout) -> Self {
        Self {
            folder: AssetFolder::new(DEFAULT_ROOT),
            models: ModelCache::default(),
            images: ImageCache::new(image_layout),
            sounds: SoundCache::default(),
            textures: TextureCache::new(texture_layout),
        }
    }

    /// Adds a mesh the game made - its `vertices` (position, texture
    /// coordinates, normal) and `indices` (three a triangle, counter-
    /// clockwise seen from the front) - as `name`, for nodes to draw with
    /// `Model::new(ModelSource::mesh(name))`, placed, turned and sized by
    /// their node like any model. Kept while the game runs; a scene using it
    /// builds it with its other models, under the loading screen.
    pub fn add_mesh(&mut self, name: impl Into<String>, vertices: Vec<ModelVertex>, indices: Vec<u32>) -> Result<(), String> {
        let name = name.into();
        if self.models.meshes.contains_key(&name) {
            return Err(format!("there's a mesh named '{name}' already"));
        }
        let mesh = MeshData { vertices, indices };
        mesh.check(&name)?;
        self.models.meshes.insert(name, Arc::new(mesh));
        Ok(())
    }

    /// What loading `source`'s model takes - or why it can't be: no such
    /// file in the assets folder, no mesh added under that name.
    pub(crate) fn model_load(&self, source: &ModelSource, double_sided: &DoubleSidedMeshes) -> Result<ModelLoad, String> {
        let key = source.key();
        let double_sided = double_sided.clone();
        match source {
            ModelSource::File(path) => self.folder.find(path)
                .map(|file| ModelLoad::file(key, file, double_sided))
                .ok_or_else(|| format!("there's no model file '{path}' in the assets folder ({})", self.folder.root().display())),
            ModelSource::Mesh(name) => self.models.meshes.get(name)
                .map(|mesh| ModelLoad::mesh(key, Arc::clone(mesh), double_sided))
                .ok_or_else(|| format!("there's no mesh '{name}' - add it with app.assets.add_mesh")),
            ModelSource::Cube => Ok(ModelLoad::cube(key, double_sided)),
            ModelSource::Plane { subdivisions } => Ok(ModelLoad::plane(key, *subdivisions, double_sided)),
        }
    }
}
