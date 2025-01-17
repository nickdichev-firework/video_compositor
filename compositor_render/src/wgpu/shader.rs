use std::{sync::Arc, time::Duration};

use compositor_common::scene::{shader::ShaderParam, NodeId};

use self::{common_params::CommonShaderParameters, pipeline::Pipeline};

use super::{
    texture::{NodeTexture, NodeTextureState, Texture},
    validation::{
        validate_contains_header, validate_params, ParametersValidationError, ShaderValidationError,
    },
    WgpuCtx, WgpuError, WgpuErrorScope,
};

pub(super) mod common_params;
pub(super) mod pipeline;

const INPUT_TEXTURES_AMOUNT: u32 = 16;

pub const VERTEX_ENTRYPOINT_NAME: &str = "vs_main";
pub const FRAGMENT_ENTRYPOINT_NAME: &str = "fs_main";

pub const USER_DEFINED_BUFFER_GROUP: u32 = 1;
pub const USER_DEFINED_BUFFER_BINDING: u32 = 0;

#[derive(Debug, thiserror::Error)]
pub enum CreateShaderError {
    #[error(transparent)]
    Wgpu(#[from] WgpuError),

    #[error(transparent)]
    Validation(#[from] ShaderValidationError),

    #[error("Shader parse error: {0}")]
    ParseError(naga::front::wgsl::ParseError),
}

/// Abstraction over single GPU shader. Used for builtins and shaders.
///
/// The bind group layout for the shader:
///
/// ```wgsl
/// var<push_constant> common_params: CommonShaderParameters;
///
/// @group(0) @binding(0) var textures: binding_array<texture_2d<f32>, 16>;
/// @group(1) @binding(0) var<uniform> shaders_custom_buffer: CustomStruct;
/// @group(2) @binding(0) var sampler_: sampler;
/// ```
#[derive(Debug)]
pub struct WgpuShader {
    pub wgpu_ctx: Arc<WgpuCtx>,
    pipeline: Pipeline,
    empty_texture: Texture,
    shader: naga::Module,
}

impl WgpuShader {
    pub fn new(wgpu_ctx: &Arc<WgpuCtx>, shader_src: String) -> Result<Self, CreateShaderError> {
        let scope = WgpuErrorScope::push(&wgpu_ctx.device);

        let shader =
            naga::front::wgsl::parse_str(&shader_src).map_err(CreateShaderError::ParseError)?;

        validate_contains_header(&wgpu_ctx.shader_header, &shader)?;

        let pipeline = Pipeline::new(
            &wgpu_ctx.device,
            wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(shader.clone())),
            &wgpu_ctx.shader_parameters_bind_group_layout,
        );

        let empty_texture = Texture::new(
            wgpu_ctx,
            Some("empty texture"),
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::TEXTURE_BINDING,
        );

        scope.pop(&wgpu_ctx.device)?;

        Ok(Self {
            wgpu_ctx: wgpu_ctx.clone(),
            pipeline,
            empty_texture,
            shader,
        })
    }

    pub fn new_parameters_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shader parameters bind group layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                count: None,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
            }],
        })
    }

    pub fn render(
        &self,
        params: &wgpu::BindGroup,
        sources: &[(&NodeId, &NodeTexture)],
        target: &NodeTextureState,
        pts: Duration,
        clear_color: Option<wgpu::Color>,
    ) {
        let ctx = &self.wgpu_ctx;

        // TODO: sources need to be ordered

        // TODO: most things that happen in this method should not be done every frame

        let textures = sources
            .iter()
            .map(|(_, node_texture)| node_texture.state())
            .collect::<Vec<_>>();
        let mut texture_views = textures
            .iter()
            .map(|node_texture| match node_texture {
                Some(node_texture) => &node_texture.rgba_texture().texture().view,
                None => &self.empty_texture.view,
            })
            .collect::<Vec<_>>();

        texture_views.extend(
            (textures.len()..INPUT_TEXTURES_AMOUNT as usize).map(|_| &self.empty_texture.view),
        );

        let input_textures_bg = ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &self.pipeline.textures_bgl,
            label: None,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureViewArray(&texture_views),
            }],
        });

        let common_shader_params =
            CommonShaderParameters::new(pts, sources.len() as u32, target.resolution());

        self.pipeline.render(
            &input_textures_bg,
            params,
            target.rgba_texture().texture(),
            ctx,
            common_shader_params,
            clear_color,
        );
    }

    pub fn validate_params(&self, params: &ShaderParam) -> Result<(), ParametersValidationError> {
        let ty = self
            .shader
            .global_variables
            .iter()
            .find(|(_, global)| match global.binding.as_ref() {
                Some(binding) => {
                    (binding.group, binding.binding)
                        == (USER_DEFINED_BUFFER_GROUP, USER_DEFINED_BUFFER_BINDING)
                }

                None => false,
            })
            .map(|(_, handle)| handle.ty)
            .ok_or(ParametersValidationError::NoBindingInShader)?;

        validate_params(params, ty, &self.shader)
    }
}
