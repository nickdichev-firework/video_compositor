use std::marker::PhantomData;

use bytes::Bytes;
use compositor_common::{frame::YuvData, scene::Resolution};
use wgpu::Buffer;

use crate::wgpu::WgpuCtx;

use super::base::Texture;

pub struct YUVPendingDownload<'a, F, E>
where
    F: FnOnce() -> Result<Bytes, E> + 'a,
{
    y: F,
    u: F,
    v: F,
    _phantom: PhantomData<&'a F>,
}

impl<'a, F, E> YUVPendingDownload<'a, F, E>
where
    F: FnOnce() -> Result<Bytes, E>,
{
    pub(super) fn new(y: F, u: F, v: F) -> Self {
        Self {
            y,
            u,
            v,
            _phantom: PhantomData,
        }
    }

    /// `device.poll(wgpu::MaintainBase::Wait)` needs to be called after download
    /// is started, but before this method is called.
    pub fn wait(self) -> Result<YuvData, E> {
        let YUVPendingDownload { y, u, v, _phantom } = self;
        Ok(YuvData {
            y_plane: y()?,
            u_plane: u()?,
            v_plane: v()?,
        })
    }
}

pub struct YUVTextures {
    pub(super) planes: [Texture; 3],
    pub(super) resolution: Resolution,
}

impl YUVTextures {
    pub fn new(ctx: &WgpuCtx, resolution: Resolution) -> Self {
        Self {
            planes: [
                Self::new_plane(ctx, resolution.width, resolution.height),
                Self::new_plane(ctx, resolution.width / 2, resolution.height / 2),
                Self::new_plane(ctx, resolution.width / 2, resolution.height / 2),
            ],
            resolution,
        }
    }

    pub fn plane(&self, i: usize) -> &Texture {
        &self.planes[i]
    }

    fn new_plane(ctx: &WgpuCtx, width: usize, height: usize) -> Texture {
        Texture::new(
            ctx,
            None,
            wgpu::Extent3d {
                width: width as u32,
                height: height as u32,
                depth_or_array_layers: 1,
            },
            wgpu::TextureFormat::R8Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
        )
    }

    pub fn new_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        let create_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            ty: Texture::DEFAULT_BINDING_TYPE,
            visibility: wgpu::ShaderStages::FRAGMENT,
            count: None,
        };
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("yuv all textures bind group layout"),
            entries: &[create_entry(0), create_entry(1), create_entry(2)],
        })
    }

    pub(super) fn new_bind_group(
        &self,
        ctx: &WgpuCtx,
        layout: &wgpu::BindGroupLayout,
    ) -> wgpu::BindGroup {
        ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("yuv all textures bind group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&self.planes[0].view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.planes[1].view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.planes[2].view),
                },
            ],
        })
    }

    pub(super) fn new_download_buffers(&self, ctx: &WgpuCtx) -> [Buffer; 3] {
        [
            self.planes[0].new_download_buffer(ctx),
            self.planes[1].new_download_buffer(ctx),
            self.planes[2].new_download_buffer(ctx),
        ]
    }

    pub(super) fn copy_to_buffers(&self, ctx: &WgpuCtx, buffers: &[Buffer; 3]) {
        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("transfer result yuv texture to buffers encoder"),
            });

        for plane in [0, 1, 2] {
            self.planes[plane].copy_to_buffer(&mut encoder, &buffers[plane]);
        }

        ctx.queue.submit(Some(encoder.finish()));
    }

    pub fn upload(&self, ctx: &WgpuCtx, data: &YuvData) {
        self.planes[0].upload_data(&ctx.queue, &data.y_plane, 1);
        self.planes[1].upload_data(&ctx.queue, &data.u_plane, 1);
        self.planes[2].upload_data(&ctx.queue, &data.v_plane, 1);
    }
}
