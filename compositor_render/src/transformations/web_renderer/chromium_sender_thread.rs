use std::path::PathBuf;
use std::{
    collections::HashMap,
    sync::Arc,
    thread::{self, JoinHandle},
};

use compositor_chromium::cef;
use compositor_common::{
    error::ErrorStack,
    scene::{NodeId, Resolution},
};
use crossbeam_channel::{Receiver, Sender};
use log::error;

use crate::renderer::RegisterCtx;
use crate::transformations::web_renderer::chromium_sender::{
    ChromiumSenderMessage, UpdateSharedMemoryInfo,
};
use crate::transformations::web_renderer::shared_memory::{SharedMemory, SharedMemoryError};
use crate::transformations::web_renderer::WebRenderer;
use crate::{wgpu::texture::utils::pad_to_256, EMBED_SOURCE_FRAMES_MESSAGE};

use super::{browser::BrowserClient, chromium_context::ChromiumContext};

pub(super) struct ChromiumSenderThread {
    chromium_ctx: Arc<ChromiumContext>,
    url: String,
    browser_client: BrowserClient,

    message_receiver: Receiver<ChromiumSenderMessage>,
    unmap_signal_sender: Sender<()>,
}

impl ChromiumSenderThread {
    pub fn new(
        ctx: &RegisterCtx,
        url: String,
        browser_client: BrowserClient,
        message_receiver: Receiver<ChromiumSenderMessage>,
        unmap_signal_sender: Sender<()>,
    ) -> Self {
        Self {
            chromium_ctx: ctx.chromium.clone(),
            url,
            browser_client,
            message_receiver,
            unmap_signal_sender,
        }
    }

    pub fn spawn(mut self) -> JoinHandle<()> {
        thread::spawn(move || self.run())
    }

    fn run(&mut self) {
        let Ok(browser) = self
            .chromium_ctx
            .start_browser(&self.url, self.browser_client.clone())
        else {
            error!("Couldn't start browser for {}", self.url);
            return;
        };

        let mut state = ThreadState::new(browser, self.chromium_ctx.instance_id());
        loop {
            let result = match self.message_receiver.recv().unwrap() {
                ChromiumSenderMessage::EmbedSources {
                    node_id,
                    resolutions,
                } => self.handle_embed_frames(&mut state, node_id, resolutions),
                ChromiumSenderMessage::EnsureSharedMemory {
                    node_id,
                    resolutions,
                } => self.handle_ensure_shared_memory(&mut state, node_id, resolutions),
                ChromiumSenderMessage::UpdateSharedMemory(info) => {
                    self.handle_shmem_update(&mut state, info)
                }
            };

            if let Err(err) = result {
                error!(
                    "Error occurred in chromium sender thread.\n{}",
                    ErrorStack::new(&err).into_string()
                );
            }
        }
    }

    fn handle_embed_frames(
        &self,
        state: &mut ThreadState,
        node_id: NodeId,
        resolutions: Vec<Option<Resolution>>,
    ) -> Result<(), ChromiumSenderThreadError> {
        let Some(shared_memory) = state.shared_memory.get(&node_id) else {
            return Err(ChromiumSenderThreadError::SharedMemoryNotAllocated(node_id));
        };
        let mut process_message = cef::ProcessMessage::new(EMBED_SOURCE_FRAMES_MESSAGE);
        let mut index = 0;

        // IPC message to chromium renderer subprocess consists of:
        // - shared memory path
        // - texture width
        // - texture height
        for (i, resolution) in resolutions.iter().enumerate() {
            let Resolution { width, height } = resolution.unwrap_or_else(|| Resolution {
                width: 0,
                height: 0,
            });
            process_message.write_string(index, shared_memory[i].to_path_string());
            process_message.write_int(index + 1, width as i32);
            process_message.write_int(index + 2, height as i32);

            index += 3;
        }

        let frame = state.browser.main_frame()?;
        frame.send_process_message(cef::ProcessId::Renderer, process_message)?;

        Ok(())
    }

    fn handle_ensure_shared_memory(
        &self,
        state: &mut ThreadState,
        node_id: NodeId,
        resolutions: Vec<Option<Resolution>>,
    ) -> Result<(), ChromiumSenderThreadError> {
        if !state.shared_memory.contains_key(&node_id) {
            state.shared_memory.insert(node_id.clone(), Vec::new());
        }

        let frame = state.browser.main_frame()?;
        let shared_memory = state.shared_memory.get_mut(&node_id).unwrap();
        for (source_idx, resolution) in resolutions.into_iter().enumerate() {
            let size = match resolution {
                Some(res) => 4 * res.width * res.height,
                None => 1,
            };

            match shared_memory.get_mut(source_idx) {
                Some(shmem) => {
                    shmem.ensure_size(size, &frame)?;
                }
                None => {
                    shared_memory.push(SharedMemory::new(
                        &state.shared_memory_root_path,
                        &node_id,
                        source_idx,
                        size,
                    )?);
                }
            }
        }

        Ok(())
    }

    // TODO: Synchronize shared memory access
    fn handle_shmem_update(
        &self,
        state: &mut ThreadState,
        info: UpdateSharedMemoryInfo,
    ) -> Result<(), ChromiumSenderThreadError> {
        let shared_memory = state.shared_memory(&info.node_id, info.source_idx)?;

        // Writes buffer data to shared memory
        {
            let range = info.buffer.slice(..).get_mapped_range();
            let chunks = range.chunks((4 * pad_to_256(info.size.width)) as usize);
            for (i, chunk) in chunks.enumerate() {
                let bytes_len = (4 * info.size.width) as usize;
                shared_memory.write(&chunk[..bytes_len], i * bytes_len)?;
            }
        }

        self.unmap_signal_sender.send(()).unwrap();
        Ok(())
    }
}

struct ThreadState {
    browser: cef::Browser,
    shared_memory: HashMap<NodeId, Vec<SharedMemory>>,
    shared_memory_root_path: PathBuf,
}

impl ThreadState {
    fn new(browser: cef::Browser, renderer_id: &str) -> Self {
        let shared_memory_root_path = WebRenderer::shared_memory_root_path(renderer_id);
        let shared_memory = HashMap::new();

        Self {
            browser,
            shared_memory,
            shared_memory_root_path,
        }
    }

    fn shared_memory(
        &mut self,
        node_id: &NodeId,
        source_idx: usize,
    ) -> Result<&mut SharedMemory, ChromiumSenderThreadError> {
        let node_shared_memory = self
            .shared_memory
            .get_mut(node_id)
            .ok_or_else(|| ChromiumSenderThreadError::SharedMemoryNotAllocated(node_id.clone()))?;

        Ok(&mut node_shared_memory[source_idx])
    }
}

#[derive(Debug, thiserror::Error)]
enum ChromiumSenderThreadError {
    #[error("Browser is no longer alive")]
    BrowserNotAlive(#[from] cef::BrowserError),

    #[error("Browser frame is no longer alive")]
    FrameNotAlive(#[from] cef::FrameError),

    #[error(transparent)]
    SharedMemoryError(#[from] SharedMemoryError),

    #[error("Shared memory should already be allocated for all inputs of node \"{0}\"")]
    SharedMemoryNotAllocated(NodeId),
}
