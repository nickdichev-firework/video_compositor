use std::sync::{Arc, Mutex};

use bytes::Bytes;
use compositor_chromium::cef;
use compositor_common::scene::{NodeId, Resolution};
use crossbeam_channel::bounded;
use log::error;
use shared_memory::ShmemError;

use crate::{
    renderer::{RegisterCtx, RenderCtx},
    wgpu::texture::NodeTexture,
};

use super::chromium_sender::ChromiumSender;

pub(super) struct BrowserController {
    chromium_sender: ChromiumSender,
    frame_data: Arc<Mutex<Bytes>>,
}

impl BrowserController {
    pub fn new(ctx: &RegisterCtx, url: String, resolution: Resolution) -> Self {
        let frame_data = Arc::new(Mutex::new(Bytes::new()));
        let client = BrowserClient::new(frame_data.clone(), resolution);
        let chromium_sender = ChromiumSender::new(ctx, url, client);

        Self {
            chromium_sender,
            frame_data,
        }
    }

    pub fn retrieve_frame(&mut self) -> Option<Bytes> {
        let frame_data = self.frame_data.lock().unwrap();
        if frame_data.is_empty() {
            return None;
        }
        Some(frame_data.clone())
    }

    pub fn send_sources(
        &mut self,
        ctx: &RenderCtx,
        node_id: NodeId,
        sources: &[(&NodeId, &NodeTexture)],
        buffers: &[Arc<wgpu::Buffer>],
    ) -> Result<(), EmbedFrameError> {
        self.chromium_sender
            .ensure_shared_memory(node_id.clone(), sources);
        self.copy_sources_to_buffers(ctx, sources, buffers)?;

        let mut pending_downloads = Vec::new();
        for (source_idx, ((_, texture), buffer)) in sources.iter().zip(buffers).enumerate() {
            let Some(texture_state) = texture.state() else {
                continue;
            };
            let size = texture_state.rgba_texture().size();
            pending_downloads.push(self.copy_buffer_to_shmem(
                node_id.clone(),
                source_idx,
                size,
                buffer.clone(),
            ));
        }

        ctx.wgpu_ctx.device.poll(wgpu::Maintain::Wait);

        for pending in pending_downloads {
            pending()?;
        }

        self.chromium_sender.embed_sources(node_id, sources);
        Ok(())
    }

    fn copy_sources_to_buffers(
        &self,
        ctx: &RenderCtx,
        sources: &[(&NodeId, &NodeTexture)],
        buffers: &[Arc<wgpu::Buffer>],
    ) -> Result<(), EmbedFrameError> {
        let mut encoder = ctx
            .wgpu_ctx
            .device
            .create_command_encoder(&Default::default());

        for ((_, texture), buffer) in sources.iter().zip(buffers) {
            let Some(texture_state) = texture.state() else {
                continue;
            };
            texture_state
                .rgba_texture()
                .copy_to_buffer(&mut encoder, buffer);
        }
        ctx.wgpu_ctx.queue.submit(Some(encoder.finish()));

        Ok(())
    }

    fn copy_buffer_to_shmem(
        &self,
        node_id: NodeId,
        source_idx: usize,
        size: wgpu::Extent3d,
        source: Arc<wgpu::Buffer>,
    ) -> impl FnOnce() -> Result<(), EmbedFrameError> + '_ {
        let (s, r) = bounded(1);
        source
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                if let Err(err) = s.send(result) {
                    error!("channel send error: {err}")
                }
            });

        move || {
            r.recv().unwrap()?;

            self.chromium_sender
                .update_shared_memory(node_id, source_idx, source.clone(), size);
            source.unmap();

            Ok(())
        }
    }
}

#[derive(Clone)]
pub(super) struct BrowserClient {
    frame_data: Arc<Mutex<Bytes>>,
    resolution: Resolution,
}

impl cef::Client for BrowserClient {
    type RenderHandlerType = RenderHandler;

    fn render_handler(&self) -> Option<Self::RenderHandlerType> {
        Some(RenderHandler::new(self.frame_data.clone(), self.resolution))
    }
}

impl BrowserClient {
    pub fn new(frame_data: Arc<Mutex<Bytes>>, resolution: Resolution) -> Self {
        Self {
            frame_data,
            resolution,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EmbedFrameError {
    #[error("Failed to create shared memory")]
    CreateSharedMemory(#[from] ShmemError),

    #[error("Failed to download source frame")]
    DownloadFrame(#[from] wgpu::BufferAsyncError),

    #[error("Browser is no longer alive")]
    BrowserNotAlive(#[from] cef::BrowserError),

    #[error("Could not send IPC message")]
    MessageNotSent(#[from] cef::FrameError),

    #[error("Download buffer does not exist")]
    ExpectDownloadBuffer,
}

pub(super) struct RenderHandler {
    frame_data: Arc<Mutex<Bytes>>,
    resolution: Resolution,
}

impl cef::RenderHandler for RenderHandler {
    fn resolution(&self, _browser: &cef::Browser) -> Resolution {
        self.resolution
    }

    fn on_paint(&self, _browser: &cef::Browser, buffer: &[u8], _resolution: Resolution) {
        let mut frame_data = self.frame_data.lock().unwrap();
        *frame_data = Bytes::copy_from_slice(buffer);
    }
}

impl RenderHandler {
    pub fn new(frame_data: Arc<Mutex<Bytes>>, resolution: Resolution) -> Self {
        Self {
            frame_data,
            resolution,
        }
    }
}
