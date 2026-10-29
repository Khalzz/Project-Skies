use image::{DynamicImage, GenericImageView};
use wgpu::{BindGroupLayoutDescriptor, Device, Extent3d, Queue, Sampler, TextureView};
use anyhow::*;

/// This code is dedicated to generation of textures and creation of rendering data for it
#[derive(Clone)]
pub struct Texture {
    pub texture: wgpu::Texture,
    pub view: TextureView,
    pub sampler: Sampler
}

impl Texture {
    pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float; // this is the format the depth will get into the render pipeline
    
    pub fn new() {
        
    }

    pub fn create_texture(device: &wgpu::Device, desc: wgpu::TextureDescriptor) -> Self {
        
        let texture = device.create_texture(&desc);

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        // we tecnically do not need to sample our depth texture, but if we want to display it is needed to be like this
        let sampler = device.create_sampler(
            &wgpu::SamplerDescriptor {
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::FilterMode::Nearest,
                compare: Some(wgpu::CompareFunction::LessEqual),
                lod_min_clamp: 0.0,
                lod_max_clamp: 100.0,
                ..Default::default()
            }
        );

        Self { texture, view, sampler }
    }

    pub fn create_depth_texture(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration, label: &str) -> Self {
        let size = wgpu::Extent3d { // the depth texture needs to be the same size of our screen (like our surface)
            width: config.width,
            height: config.height,
            depth_or_array_layers: 1,
        };
        let desc = wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Self::DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        };
        let texture = device.create_texture(&desc);

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        // we tecnically do not need to sample our depth texture, but if we want to display it is needed to be like this
        let sampler = device.create_sampler(
            &wgpu::SamplerDescriptor {
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::FilterMode::Nearest,
                compare: Some(wgpu::CompareFunction::LessEqual),
                lod_min_clamp: 0.0,
                lod_max_clamp: 100.0,
                ..Default::default()
            }
        );

        Self { texture, view, sampler }
    }

    #[allow(unused)]
    pub fn create_depth_texture_non_comparison_sampler(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration, label: &str) -> Self {
        let size = wgpu::Extent3d {
            width: config.width,
            height: config.height,
            depth_or_array_layers: 1,
        };
        
        let desc = wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: Self::DEPTH_FORMAT, // we store the image as sRGB

            // COPY_SRC/COPY_DST beyond the original RENDER_ATTACHMENT |
            // TEXTURE_BINDING - lets a texture built with this same fn serve
            // as either side of a copy_texture_to_texture (see DepthRender's
            // own foam_depth_copy field: a per-frame snapshot of the real
            // depth texture, taken so the water shader can sample "what's
            // behind me" without reading the same texture it's still being
            // depth-tested/written against in the same pass).
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST,
            view_formats: &[Self::DEPTH_FORMAT],
        };

        let texture = device.create_texture(&desc);

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            compare: None,
            lod_min_clamp: 0.0,
            lod_max_clamp: 100.0,
            ..Default::default()
        });

        Self {
            texture,
            view,
            sampler,
        }
    }

    pub fn from_bytes(bytes: &[u8], device: &Device, queue: &Queue, label: &str) -> Result<Self> {
        let image = image::load_from_memory(bytes).unwrap(); // load the image
        return Self::from_image(&image, device, queue, Some(label))
    }

    /// `image` on the GPU, with every smaller version of it (its mipmaps:
    /// half, a quarter... down to 1 x 1, each averaged from the one before)
    /// - what the GPU reads from when it's drawn small, far away, instead of
    /// picking single texels out of the full image (which sparkles).
    pub fn from_image(image: &DynamicImage, device: &Device, queue: &Queue, label: Option<&str>) -> Result<Self> {
        let (width, height) = image.dimensions();
        let levels = mip_level_count(width, height);

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            size: Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            label: label.or(Some("diffuse_texture")),
            view_formats: &[],
        });

        let mut level = image.to_rgba8();
        for mip_level in 0..levels {
            let (level_width, level_height) = level.dimensions();
            queue.write_texture(
                wgpu::TexelCopyTextureInfo { texture: &texture, mip_level, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                &level,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(4 * level_width), rows_per_image: Some(level_height) },
                Extent3d { width: level_width, height: level_height, depth_or_array_layers: 1 },
            );
            if mip_level + 1 < levels {
                level = image::imageops::resize(&level, (level_width / 2).max(1), (level_height / 2).max(1), image::imageops::FilterType::Triangle);
            }
        }

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&Self::mipmapped_sampler(wgpu::AddressMode::Repeat, label));
        Ok(Self { texture, view, sampler })
    }

    /// A sampler that blends between texels and between mipmap sizes, and
    /// reads several samples along a surface seen at a low angle (anisotropic
    /// filtering - a floor stays sharp into the distance), addressing past
    /// the edges with `address`.
    pub(crate) fn mipmapped_sampler(address: wgpu::AddressMode, label: Option<&str>) -> wgpu::SamplerDescriptor<'_> {
        wgpu::SamplerDescriptor {
            label,
            address_mode_u: address,
            address_mode_v: address,
            address_mode_w: address,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            anisotropy_clamp: 16,
            ..Default::default()
        }
    }

    /// Builds a single texture_cube from 6 already-decoded face images, in the
    /// standard +X, -X, +Y, -Y, +Z, -Z order.
    pub fn from_cube_images(images: [DynamicImage; 6], device: &Device, queue: &Queue, label: &str) -> Result<Self> {
        let dimensions = images[0].dimensions();

        let texture_size = Extent3d {
            width: dimensions.0,
            height: dimensions.1,
            depth_or_array_layers: 6,
        };

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        for (face_index, image) in images.iter().enumerate() {
            let rgba = image.to_rgba8();
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d { x: 0, y: 0, z: face_index as u32 },
                    aspect: wgpu::TextureAspect::All,
                },
                &rgba,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * dimensions.0),
                    rows_per_image: Some(dimensions.1),
                },
                Extent3d { width: dimensions.0, height: dimensions.1, depth_or_array_layers: 1 },
            );
        }

        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some(label),
            dimension: Some(wgpu::TextureViewDimension::Cube),
            array_layer_count: Some(6),
            ..Default::default()
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Ok(Self { texture, view, sampler })
    }

    /// Decodes 6 encoded face images (PNG/JPEG bytes, +X, -X, +Y, -Y, +Z, -Z order) into a cube texture.
    pub fn from_cube_bytes(bytes: [&[u8]; 6], device: &Device, queue: &Queue, label: &str) -> Result<Self> {
        let images = [
            image::load_from_memory(bytes[0])?,
            image::load_from_memory(bytes[1])?,
            image::load_from_memory(bytes[2])?,
            image::load_from_memory(bytes[3])?,
            image::load_from_memory(bytes[4])?,
            image::load_from_memory(bytes[5])?,
        ];
        Self::from_cube_images(images, device, queue, label)
    }

    pub fn create_bind_group_layout(device: &Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&BindGroupLayoutDescriptor {
            label: Some("texture_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        })
    }
}

/// How many mipmap sizes a `width` x `height` image has - itself, then each
/// half the one before, down to 1 x 1.
pub(crate) fn mip_level_count(width: u32, height: u32) -> u32 {
    32 - width.max(height).max(1).leading_zeros()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_image_has_a_mipmap_for_each_halving_down_to_one_pixel() {
        assert_eq!(mip_level_count(1, 1), 1);
        assert_eq!(mip_level_count(2, 2), 2);
        assert_eq!(mip_level_count(1024, 1024), 11);
        assert_eq!(mip_level_count(1024, 16), 11, "the larger side decides");
        assert_eq!(mip_level_count(300, 200), 9, "300, 150, 75, 37, 18, 9, 4, 2, 1");
    }
}
