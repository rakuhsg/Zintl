use crate::task::RuntimeTask;
use crate::{RuntimeError, Services};
use runtime_event_loop::CompletionNotifier;
use runtime_event_loop::worker::{WorkerCompletion, WorkerError};
use runtime_filesystem::{FilesystemRights, RelativePath};
use runtime_resource::ResourceHandle;
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex, Weak};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileMetadata {
    pub kind: FileKind,
    pub size: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileKind {
    File,
    Directory,
}

impl fmt::Display for FileKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::File => "file",
            Self::Directory => "directory",
        })
    }
}

pub struct Directory {
    pub(crate) services: Weak<Services>,
    pub(crate) handle: ResourceHandle,
    pub(crate) locator: Vec<u8>,
    pub(crate) rights: u64,
    pub(crate) quota: u64,
    closed: AtomicBool,
}

impl Directory {
    pub(crate) fn new(
        services: Weak<Services>,
        handle: ResourceHandle,
        locator: Vec<u8>,
        rights: u64,
        quota: u64,
    ) -> Self {
        Self {
            services,
            handle,
            locator,
            rights,
            quota,
            closed: AtomicBool::new(false),
        }
    }

    /// Opens a typed, attenuated regular-file resource away from the caller.
    ///
    /// # Errors
    ///
    /// Rejects stale/closed authority, unsafe paths, rights escalation, symlinks,
    /// wrong file types, and executor saturation.
    pub fn open_file(
        &self,
        path: impl Into<String>,
        rights: FilesystemRights,
        create: bool,
        truncate: bool,
    ) -> Result<RuntimeTask<FileResource>, RuntimeError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(RuntimeError::ResourceClosed);
        }
        let services = self.services.upgrade().ok_or(RuntimeError::ShuttingDown)?;
        services.ensure_running()?;
        let directory = self.handle;
        let path = path.into();
        let weak = self.services.clone();
        services.host.submit(move |cancellation| {
            if cancellation.is_cancelled() {
                return Err(RuntimeError::Cancelled);
            }
            let services = weak.upgrade().ok_or(RuntimeError::ShuttingDown)?;
            services.ensure_running()?;
            let path = RelativePath::parse(&path).map_err(RuntimeError::from_filesystem)?;
            let handle = services
                .filesystem
                .open_relative(directory, &path, rights, create, truncate)
                .map_err(RuntimeError::from_filesystem)?;
            services.track_resource(handle)?;
            if let Err(error) = services.ensure_running() {
                let _ = services.close_resource(handle);
                return Err(error);
            }
            Ok(FileResource::new(Weak::clone(&weak), handle))
        })
    }

    /// Closes this runtime-local directory capability.
    ///
    /// # Errors
    ///
    /// Rejects a duplicate close or unavailable runtime.
    pub fn close(&self) -> Result<(), RuntimeError> {
        if self.closed.swap(true, Ordering::AcqRel) {
            return Err(RuntimeError::ResourceClosed);
        }
        let services = self.services.upgrade().ok_or(RuntimeError::ShuttingDown)?;
        services.close_resource(self.handle)
    }

    #[must_use]
    pub fn rights(&self) -> u64 {
        self.rights
    }

    #[must_use]
    pub fn quota(&self) -> u64 {
        self.quota
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }
}

impl Drop for Directory {
    fn drop(&mut self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Some(services) = self.services.upgrade() {
            let _ = services.close_resource(self.handle);
        }
    }
}

pub struct FileResource {
    services: Weak<Services>,
    handle: ResourceHandle,
    closed: AtomicBool,
}

impl FileResource {
    fn new(services: Weak<Services>, handle: ResourceHandle) -> Self {
        Self {
            services,
            handle,
            closed: AtomicBool::new(false),
        }
    }

    /// Reads from the file's current offset on the bounded filesystem pool.
    ///
    /// # Errors
    ///
    /// Rejects missing rights, closed/stale authority, invalid limits, queue
    /// saturation, cancellation, and I/O failure.
    pub fn read(&self, maximum_bytes: usize) -> Result<RuntimeTask<Vec<u8>>, RuntimeError> {
        self.submit(move |services, request_id, handle| {
            services
                .filesystem
                .submit_file_read(request_id, handle, maximum_bytes)
                .map_err(RuntimeError::from_filesystem)?;
            services.wait_for_filesystem(request_id)
        })
    }

    /// Writes bytes at the file's current offset on the bounded filesystem pool.
    ///
    /// # Errors
    ///
    /// Rejects missing rights, closed/stale authority, oversized input, queue
    /// saturation, cancellation, and I/O failure.
    pub fn write(&self, bytes: Vec<u8>) -> Result<RuntimeTask<()>, RuntimeError> {
        self.submit(move |services, request_id, handle| {
            services
                .filesystem
                .submit_file_write(request_id, handle, bytes)
                .map_err(RuntimeError::from_filesystem)?;
            services.wait_for_filesystem(request_id).map(|_| ())
        })
    }

    /// Reads fixed-width sanitized metadata for this opened file.
    ///
    /// # Errors
    ///
    /// Rejects missing rights, closed/stale authority, queue saturation, and
    /// malformed worker output.
    pub fn stat(&self) -> Result<RuntimeTask<FileMetadata>, RuntimeError> {
        self.submit(move |services, request_id, handle| {
            services
                .filesystem
                .submit_file_metadata(request_id, handle)
                .map_err(RuntimeError::from_filesystem)?;
            decode_metadata(&services.wait_for_filesystem(request_id)?)
        })
    }

    /// Closes this runtime-local file capability.
    ///
    /// # Errors
    ///
    /// Rejects duplicate close or an unavailable runtime.
    pub fn close(&self) -> Result<(), RuntimeError> {
        if self.closed.swap(true, Ordering::AcqRel) {
            return Err(RuntimeError::ResourceClosed);
        }
        let services = self.services.upgrade().ok_or(RuntimeError::ShuttingDown)?;
        services.close_resource(self.handle)
    }

    fn submit<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&Services, u64, ResourceHandle) -> Result<T, RuntimeError>
        + Send
        + 'static,
    ) -> Result<RuntimeTask<T>, RuntimeError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(RuntimeError::ResourceClosed);
        }
        let services = self.services.upgrade().ok_or(RuntimeError::ShuttingDown)?;
        services.ensure_running()?;
        let weak = self.services.clone();
        let handle = self.handle;
        services.host.submit(move |cancellation| {
            if cancellation.is_cancelled() {
                return Err(RuntimeError::Cancelled);
            }
            let services = weak.upgrade().ok_or(RuntimeError::ShuttingDown)?;
            services.ensure_running()?;
            let request_id = services.next_request_id()?;
            operation(&services, request_id, handle)
        })
    }
}

impl Drop for FileResource {
    fn drop(&mut self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        if let Some(services) = self.services.upgrade() {
            let _ = services.close_resource(self.handle);
        }
    }
}

#[derive(Default)]
pub(crate) struct FilesystemCompletions {
    pending_signal: Mutex<bool>,
    changed: Condvar,
    results: Mutex<HashMap<u64, Result<Vec<u8>, WorkerError>>>,
    drain: Mutex<()>,
}

impl CompletionNotifier for FilesystemCompletions {
    fn notify_drain_needed(&self) {
        if let Ok(mut pending) = self.pending_signal.lock() {
            *pending = true;
            self.changed.notify_all();
        }
    }
}

impl FilesystemCompletions {
    pub(crate) fn store(&self, completion: WorkerCompletion) -> Result<(), RuntimeError> {
        self.results
            .lock()
            .map_err(|_| RuntimeError::Internal)?
            .insert(completion.request_id, completion.result);
        Ok(())
    }

    pub(crate) fn take(
        &self,
        request_id: u64,
    ) -> Result<Option<Result<Vec<u8>, WorkerError>>, RuntimeError> {
        Ok(self
            .results
            .lock()
            .map_err(|_| RuntimeError::Internal)?
            .remove(&request_id))
    }

    pub(crate) fn wait_signal(&self) -> Result<(), RuntimeError> {
        let pending = self
            .pending_signal
            .lock()
            .map_err(|_| RuntimeError::Internal)?;
        let (mut pending, _) = self
            .changed
            .wait_timeout_while(pending, Duration::from_secs(1), |pending| !*pending)
            .map_err(|_| RuntimeError::Internal)?;
        *pending = false;
        Ok(())
    }

    pub(crate) fn drain_guard(&self) -> Result<std::sync::MutexGuard<'_, ()>, RuntimeError> {
        self.drain.lock().map_err(|_| RuntimeError::Internal)
    }
}

fn decode_metadata(bytes: &[u8]) -> Result<FileMetadata, RuntimeError> {
    if bytes.len() != 9 {
        return Err(RuntimeError::Protocol);
    }
    let size = u64::from_be_bytes(bytes[1..].try_into().map_err(|_| RuntimeError::Protocol)?);
    let kind = match bytes[0] {
        1 => FileKind::File,
        2 => FileKind::Directory,
        _ => return Err(RuntimeError::Protocol),
    };
    Ok(FileMetadata { kind, size })
}
