use std::sync::Arc;

use compositor_common::scene::builtin_transformations::BuiltinSpec;

use crate::wgpu::{
    shader::{CreateShaderError, WgpuShader},
    WgpuCtx,
};

use super::{error::InitBuiltinError, BuiltinState, BuiltinTransition};

pub struct BuiltinTransformations {
    apply_matrix: ApplyTransformationMatrix,
    mirror_image: MirrorImage,
    corners_rounding: CornersRounding,
}

impl BuiltinTransformations {
    pub fn new(wgpu_ctx: &Arc<WgpuCtx>) -> Result<Self, InitBuiltinError> {
        Ok(Self {
            apply_matrix: ApplyTransformationMatrix::new(wgpu_ctx)
                .map_err(InitBuiltinError::ApplyTransformationMatrix)?,
            mirror_image: MirrorImage::new(wgpu_ctx).map_err(InitBuiltinError::MirrorImage)?,
            corners_rounding: CornersRounding::new(wgpu_ctx)
                .map_err(InitBuiltinError::CornersRounding)?,
        })
    }

    pub fn gpu_shader(&self, state: &BuiltinState) -> Arc<WgpuShader> {
        match state {
            BuiltinState::Interpolated { transition, .. } => match transition {
                BuiltinTransition::FixedPositionLayout(_, _) => self.apply_matrix.0.clone(),
            },
            BuiltinState::Static(spec) => match spec {
                BuiltinSpec::FitToResolution(_)
                | BuiltinSpec::FillToResolution { .. }
                | BuiltinSpec::StretchToResolution { .. }
                | BuiltinSpec::FixedPositionLayout { .. }
                | BuiltinSpec::TiledLayout { .. } => self.apply_matrix.0.clone(),
                BuiltinSpec::MirrorImage { .. } => self.mirror_image.0.clone(),
                BuiltinSpec::CornersRounding { .. } => self.corners_rounding.0.clone(),
            },
        }
    }
}

pub struct ApplyTransformationMatrix(Arc<WgpuShader>);

impl ApplyTransformationMatrix {
    fn new(wgpu_ctx: &Arc<WgpuCtx>) -> Result<Self, CreateShaderError> {
        Ok(Self(Arc::new(WgpuShader::new(
            wgpu_ctx,
            include_str!("./apply_transformation_matrix.wgsl").into(),
        )?)))
    }
}

pub struct MirrorImage(Arc<WgpuShader>);

impl MirrorImage {
    fn new(wgpu_ctx: &Arc<WgpuCtx>) -> Result<Self, CreateShaderError> {
        Ok(Self(Arc::new(WgpuShader::new(
            wgpu_ctx,
            include_str!("./mirror_image.wgsl").into(),
        )?)))
    }
}

pub struct CornersRounding(Arc<WgpuShader>);

impl CornersRounding {
    fn new(wgpu_ctx: &Arc<WgpuCtx>) -> Result<Self, CreateShaderError> {
        Ok(Self(Arc::new(WgpuShader::new(
            wgpu_ctx,
            include_str!("./corners_rounding.wgsl").into(),
        )?)))
    }
}
