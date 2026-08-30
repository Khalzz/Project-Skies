use wgpu::{BindGroup, BindGroupLayout, Device, RenderPipeline, Sampler, SurfaceConfiguration, TextureView};

use crate::engine::rendering::camera::handler::CameraResources;
use crate::engine::rendering::enviroment::light::Light;
use crate::engine::rendering::instance_management::InstanceRaw;
use crate::engine::rendering::models::model::{self, Vertex};
use crate::engine::rendering::models::textures::Texture;
use crate::engine::rendering::ui::rendering_utils;

/// A second draw pipeline, alongside `App::render_pipeline`, used only for
/// model_refs in `App::water_shaded_models` (see `render_pass.rs`'s own
/// `render_water_pass`, which is what actually draws with this instead of
/// the generic `draw_model_instanced_from_list` every other model goes
/// through). Group 3 (light) still rides the same shared bind group every
/// other pipeline uses (see `LightUniform::time`'s own doc comment), and
/// groups 1/2 (camera/mesh transform) reuse the same layouts too - but group
/// 0 is its own thing here: not the model's own material (water doesn't
/// sample a texture at all, see water.wgsl's own doc comment), instead a
/// snapshot of the depth buffer (`DepthRender::foam_depth_copy`) the shader
/// samples to find solid geometry near/behind it, for shore-intersection
/// foam. That's also why water can't go through the shared
/// `draw_model_instanced_from_list`/`draw_mesh_instanced` helpers other
/// models use - those unconditionally bind the mesh's own *material* at
/// group 0, which would stomp this.
pub struct WaterRenderData {
    depth_bind_group_layout: BindGroupLayout,
    depth_bind_group: BindGroup,
    pub render_pipeline: RenderPipeline,
}

impl WaterRenderData {
    pub fn new(device: &Device, config: &SurfaceConfiguration, camera: &CameraResources, light: &Light, depth_copy_view: &TextureView, depth_copy_sampler: &Sampler) -> Self {
        let depth_bind_group_layout = Self::create_depth_bind_group_layout(device);
        let depth_bind_group = Self::create_depth_bind_group(device, &depth_bind_group_layout, depth_copy_view, depth_copy_sampler);

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Water Pipeline Layout"),
            bind_group_layouts: &[
                &depth_bind_group_layout,
                &camera.bind_group_layout,
                &model::Mesh::create_bind_group_layout(device),
                &light.rendering_data.bind_group_layout,
            ],
            push_constant_ranges: &[],
        });

        let shader = wgpu::ShaderModuleDescriptor {
            label: Some("Water Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/water.wgsl").into()),
        };

        let render_pipeline = rendering_utils::create_render_pipeline(
            device,
            &layout,
            config.format,
            Some(Texture::DEPTH_FORMAT),
            &[model::ModelVertex::desc(), InstanceRaw::desc()],
            shader,
        );

        Self { depth_bind_group_layout, depth_bind_group, render_pipeline }
    }

    // Non-filtering/non-comparison, Float{filterable: false} sample type -
    // the same shape DepthRender's own bind_group_layout already uses to
    // sample this exact Depth32Float format as a plain texture (see
    // depth_map.wgsl, the debug depth-visualization shader) - proven to work
    // in this engine already, so mirrored here rather than reaching for
    // WGSL's dedicated texture_depth_2d/Depth sample type instead. No
    // uniform buffer entry (DepthRender's own layout has a 3rd one for
    // near/far) since water.wgsl gets near/far from its own NEAR/FAR
    // constants instead - see that file's own comment on why.
    fn create_depth_bind_group_layout(device: &Device) -> BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("water_depth_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
            ],
        })
    }

    fn create_depth_bind_group(device: &Device, layout: &BindGroupLayout, depth_copy_view: &TextureView, depth_copy_sampler: &Sampler) -> BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("water_depth_bind_group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(depth_copy_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(depth_copy_sampler) },
            ],
        })
    }

    pub fn bind_group(&self) -> &BindGroup {
        &self.depth_bind_group
    }

    /// Rebuilds the depth bind group against a freshly-resized
    /// `foam_depth_copy` - call from `App::resize`, after
    /// `DepthRender::resize` has already run (a bind group is tied to a
    /// specific texture view; the old one becomes invalid the instant the
    /// view it pointed at is dropped/replaced).
    pub fn rebuild_depth_bind_group(&mut self, device: &Device, depth_copy_view: &TextureView, depth_copy_sampler: &Sampler) {
        self.depth_bind_group = Self::create_depth_bind_group(device, &self.depth_bind_group_layout, depth_copy_view, depth_copy_sampler);
    }
}
