use crate::UNEMBED_SOURCE_FRAMES_MESSAGE;
use compositor_chromium::cef;
use compositor_common::scene::NodeId;
use log::error;
use shared_memory::{Shmem, ShmemConf, ShmemError};
use std::path::{Path, PathBuf};
use std::{fs, io};

pub struct SharedMemory {
    inner: Option<Shmem>,
    path: PathBuf,
}

impl SharedMemory {
    pub fn new(
        root_path: &Path,
        node_id: &NodeId,
        source_idx: usize,
        size: usize,
    ) -> Result<Self, SharedMemoryError> {
        let path = root_path.join(node_id.to_string());
        Self::init_shared_memory_folder(&path)?;

        Self::from_path(path.join(source_idx.to_string()), size)
    }

    pub fn from_path(path: PathBuf, size: usize) -> Result<Self, SharedMemoryError> {
        let inner = ShmemConf::new()
            .flink(&path)
            .size(size)
            .force_create_flink()
            .create()?;

        Ok(Self {
            inner: Some(inner),
            path,
        })
    }

    pub fn to_path_string(&self) -> String {
        self.path.display().to_string()
    }

    pub fn ensure_size(
        &mut self,
        size: usize,
        frame: &cef::Frame,
    ) -> Result<(), SharedMemoryError> {
        let shmem_len = self.inner.as_ref().map(|shmem| shmem.len()).unwrap();
        if shmem_len == size {
            return Ok(());
        }

        // TODO: This should be synchronised
        let mut process_message = cef::ProcessMessage::new(UNEMBED_SOURCE_FRAMES_MESSAGE);
        process_message.write_string(0, self.path.display().to_string());
        frame.send_process_message(cef::ProcessId::Renderer, process_message)?;
        // -----

        // Releasing the current `Shmem` instance to ensure it does not erase the shared memory descriptor from the file system
        // This is critical to ensure when a new `Shmem` is created at the same location, it doesn't conflict with the old descriptor
        drop(self.inner.take());
        // After releasing the old `Shmem`, establish a new one from the existing path with the updated size
        *self = Self::from_path(self.path.clone(), size)?;

        Ok(())
    }

    pub fn write(&mut self, data: &[u8], offset: usize) -> Result<(), SharedMemoryError> {
        let inner = self.inner.as_ref().unwrap();
        if inner.len() < offset + data.len() {
            return Err(SharedMemoryError::OutOfBounds {
                shared_memory_len: inner.len(),
                write_len: offset + data.len(),
            });
        }

        let shmem = inner.as_ptr();
        unsafe {
            std::ptr::copy(data.as_ptr(), shmem.add(offset), data.len());
        }

        Ok(())
    }

    fn init_shared_memory_folder(root_shmem_folder: &Path) -> Result<(), SharedMemoryError> {
        if root_shmem_folder.exists() {
            return Ok(());
        }

        fs::create_dir_all(root_shmem_folder).map_err(SharedMemoryError::CreateShmemFolderFailed)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SharedMemoryError {
    #[error("Failed to create shared memory")]
    CreateSharedMemoryFailed(#[from] ShmemError),

    #[error(
        "Tried to write outside of the shared memory bounds: {shared_memory_len} < {write_len}"
    )]
    OutOfBounds {
        shared_memory_len: usize,
        write_len: usize,
    },

    #[error("Browser frame is no longer alive")]
    FrameNotAlive(#[from] cef::FrameError),

    #[error("Failed to create folder for shared memory")]
    CreateShmemFolderFailed(#[source] io::Error),
}
