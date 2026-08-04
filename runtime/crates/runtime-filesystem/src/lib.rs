//! Descriptor-relative filesystem authority and bounded worker operations.

#![forbid(unsafe_code)]

use runtime_event_loop::CompletionNotifier;
use runtime_event_loop::worker::{
    CancellationToken, WorkerCompletion, WorkerError, WorkerPool, WorkerPoolError,
};
use runtime_resource::{
    Resource, ResourceError, ResourceHandle, ResourceKind, ResourceOwner, ResourceRights,
    ResourceTable,
};
use rustix::fd::OwnedFd;
use rustix::fs::{
    AtFlags, CWD, Dir, FileType, Mode, OFlags, fstat, mkdirat, openat, renameat, statat, unlinkat,
};
use std::any::Any;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use unicode_normalization::UnicodeNormalization;

const MAX_PATH_BYTES: usize = 4_096;
const MAX_COMPONENT_BYTES: usize = 255;
const MAX_COMPONENTS: usize = 128;
const DIRECTORY_MODE: Mode = Mode::from_raw_mode(0o700);
const FILE_MODE: Mode = Mode::from_raw_mode(0o600);
const MAX_TRANSIENT_IO_RETRIES: usize = 16;

/// Filesystem-specific rights attached to an opened directory resource.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FilesystemRights(u64);

impl FilesystemRights {
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const CREATE: Self = Self(1 << 2);
    pub const METADATA: Self = Self(1 << 3);
    pub const ENUMERATE: Self = Self(1 << 4);
    pub const TRUNCATE: Self = Self(1 << 5);
    const ALL_BITS: u64 = Self::READ.0
        | Self::WRITE.0
        | Self::CREATE.0
        | Self::METADATA.0
        | Self::ENUMERATE.0
        | Self::TRUNCATE.0;

    /// Validates a non-empty set containing only known filesystem rights.
    ///
    /// # Errors
    ///
    /// Rejects empty or unknown bits.
    pub const fn from_bits(bits: u64) -> Result<Self, FilesystemError> {
        if bits == 0 || bits & !Self::ALL_BITS != 0 {
            Err(FilesystemError::PermissionDenied)
        } else {
            Ok(Self(bits))
        }
    }

    #[must_use]
    pub const fn bits(self) -> u64 {
        self.0
    }

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// A directory locator returned by an explicit trusted approval decision.
/// Constructing this value grants no access until the directory is opened.
#[derive(Clone, Debug)]
pub struct ApprovedDirectory {
    locator: PathBuf,
    rights: FilesystemRights,
}

/// Stable directory identity authenticated inside a permission envelope.
/// This is trusted persistence metadata, not a JavaScript-visible handle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DirectoryIdentity {
    device: u64,
    file: u64,
}

impl DirectoryIdentity {
    /// Decodes the fixed-width identity stored inside an authenticated envelope.
    ///
    /// # Errors
    ///
    /// Rejects every non-canonical length.
    pub fn from_authenticated_bytes(bytes: &[u8]) -> Result<Self, FilesystemError> {
        if bytes.len() != 16 {
            return Err(FilesystemError::InvalidIdentity);
        }
        let device = u64::from_be_bytes(
            bytes[..8]
                .try_into()
                .map_err(|_| FilesystemError::InvalidIdentity)?,
        );
        let file = u64::from_be_bytes(
            bytes[8..]
                .try_into()
                .map_err(|_| FilesystemError::InvalidIdentity)?,
        );
        if device == 0 || file == 0 {
            return Err(FilesystemError::InvalidIdentity);
        }
        Ok(Self { device, file })
    }

    /// Encodes identity for placement inside an embedder-sealed envelope.
    #[must_use]
    pub fn authenticated_bytes(self) -> [u8; 16] {
        let mut output = [0_u8; 16];
        output[..8].copy_from_slice(&self.device.to_be_bytes());
        output[8..].copy_from_slice(&self.file.to_be_bytes());
        output
    }
}

impl ApprovedDirectory {
    /// Records an allow decision made by the trusted embedder.
    ///
    /// # Errors
    ///
    /// Rejects relative, empty, overlong, non-UTF-8, or authority-free input.
    pub fn from_trusted_approval(
        locator: impl Into<PathBuf>,
        rights: FilesystemRights,
    ) -> Result<Self, FilesystemError> {
        let locator = locator.into();
        validate_approved_locator(&locator)?;
        if rights.is_empty() {
            return Err(FilesystemError::PermissionDenied);
        }
        Ok(Self { locator, rights })
    }
}

/// Validated relative components. It cannot represent an absolute path or `..`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelativePath {
    components: Vec<String>,
}

impl RelativePath {
    /// Parses a non-empty, NFC-normalized relative path.
    ///
    /// # Errors
    ///
    /// Rejects absolute syntax, empty/dot/parent components, NUL, backslash,
    /// non-NFC text, and configured component/path limits.
    pub fn parse(value: &str) -> Result<Self, FilesystemError> {
        if value.is_empty()
            || value.len() > MAX_PATH_BYTES
            || value.starts_with('/')
            || value.contains(['\0', '\\'])
        {
            return Err(FilesystemError::InvalidPath);
        }
        let components = value
            .split('/')
            .map(|component| {
                if component.is_empty()
                    || matches!(component, "." | "..")
                    || component.len() > MAX_COMPONENT_BYTES
                    || component.nfc().ne(component.chars())
                {
                    Err(FilesystemError::InvalidPath)
                } else {
                    Ok(component.to_owned())
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        if components.is_empty() || components.len() > MAX_COMPONENTS {
            return Err(FilesystemError::InvalidPath);
        }
        Ok(Self { components })
    }
}

/// Bounded descriptor-relative operations executed by the filesystem pool.
#[derive(Clone, Debug)]
pub enum FilesystemOperation {
    ReadFile {
        path: RelativePath,
        max_bytes: usize,
    },
    WriteFile {
        path: RelativePath,
        data: Vec<u8>,
        create: bool,
        truncate: bool,
    },
    CreateDirectory {
        path: RelativePath,
    },
    ListDirectory {
        path: Option<RelativePath>,
        max_entries: usize,
        max_encoded_bytes: usize,
    },
    Metadata {
        path: RelativePath,
    },
    RemoveFile {
        path: RelativePath,
    },
    RemoveDirectory {
        path: RelativePath,
    },
    Rename {
        source: RelativePath,
        destination: RelativePath,
    },
}

impl FilesystemOperation {
    const fn required_right(&self) -> FilesystemRights {
        match self {
            Self::ReadFile { .. } => FilesystemRights::READ,
            Self::WriteFile {
                create, truncate, ..
            } => {
                let mut rights = FilesystemRights::WRITE;
                if *create {
                    rights = rights.union(FilesystemRights::CREATE);
                }
                if *truncate {
                    rights = rights.union(FilesystemRights::TRUNCATE);
                }
                rights
            }
            Self::CreateDirectory { .. } => FilesystemRights::CREATE,
            Self::RemoveFile { .. } | Self::RemoveDirectory { .. } | Self::Rename { .. } => {
                FilesystemRights::WRITE
            }
            Self::ListDirectory { .. } => FilesystemRights::ENUMERATE,
            Self::Metadata { .. } => FilesystemRights::METADATA,
        }
    }
}

struct DirectoryResource {
    descriptor: Option<OwnedFd>,
}

struct FileResource {
    descriptor: Option<OwnedFd>,
}

impl Resource for FileResource {
    fn close(&mut self) {
        self.descriptor.take();
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl Resource for DirectoryResource {
    fn close(&mut self) {
        self.descriptor.take();
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Runtime-local filesystem resources plus a fixed, bounded blocking pool.
pub struct Filesystem {
    resources: Mutex<ResourceTable>,
    directory_kind: ResourceKind,
    file_kind: ResourceKind,
    workers: WorkerPool,
    max_operation_bytes: usize,
}

impl Filesystem {
    /// Creates an empty deny-by-default filesystem service.
    ///
    /// # Errors
    ///
    /// Rejects zero limits or worker construction failure.
    pub fn new(
        owner: ResourceOwner,
        max_resources: usize,
        worker_count: usize,
        queue_limit: usize,
        completion_limit: usize,
        max_operation_bytes: usize,
        notifier: Arc<dyn CompletionNotifier>,
    ) -> Result<Self, FilesystemError> {
        if max_operation_bytes == 0 {
            return Err(FilesystemError::QuotaExceeded);
        }
        Ok(Self {
            resources: Mutex::new(ResourceTable::new(owner, max_resources)?),
            directory_kind: ResourceKind::new("fs.directory")?,
            file_kind: ResourceKind::new("fs.file")?,
            workers: WorkerPool::new(
                worker_count,
                queue_limit,
                completion_limit,
                max_operation_bytes,
                notifier,
            )?,
            max_operation_bytes,
        })
    }

    /// Opens a directory only after a trusted approval value exists and stores
    /// the descriptor behind an opaque generational resource handle.
    ///
    /// This potentially blocking method must be called on a host/filesystem
    /// executor, never the JavaScript executor.
    ///
    /// # Errors
    ///
    /// Rejects symlinks in every locator component, non-directories, I/O
    /// failures, resource exhaustion, and poisoned state.
    pub fn open_approved_directory(
        &self,
        approved: &ApprovedDirectory,
    ) -> Result<ResourceHandle, FilesystemError> {
        let descriptor = open_approved_root(&approved.locator)?;
        self.insert_directory(descriptor, approved.rights)
    }

    /// Reopens an imported directory and compares its actual identity with the
    /// authenticated export before minting a new runtime-local resource.
    ///
    /// # Errors
    ///
    /// Rejects replacement, wrong type, symlink, invalid path, and resource
    /// exhaustion without retaining the opened descriptor.
    pub fn open_imported_directory(
        &self,
        approved: &ApprovedDirectory,
        expected_identity: DirectoryIdentity,
    ) -> Result<ResourceHandle, FilesystemError> {
        let descriptor = open_approved_root(&approved.locator)?;
        if directory_identity(&descriptor)? != expected_identity {
            return Err(FilesystemError::IdentityMismatch);
        }
        self.insert_directory(descriptor, approved.rights)
    }

    /// Returns trusted persistence identity for an opened directory.
    ///
    /// # Errors
    ///
    /// Rejects forged, cross-runtime, stale, or wrong-kind handles.
    pub fn directory_identity(
        &self,
        handle: ResourceHandle,
    ) -> Result<DirectoryIdentity, FilesystemError> {
        self.resources
            .lock()
            .map_err(|_| FilesystemError::Internal)?
            .with_resource(
                handle,
                &self.directory_kind,
                ResourceRights::from_bits(0),
                |resource| {
                    resource
                        .as_any_mut()
                        .downcast_mut::<DirectoryResource>()
                        .and_then(|directory| directory.descriptor.as_ref())
                        .ok_or(FilesystemError::InvalidResource)
                        .and_then(directory_identity)
                },
            )?
    }

    /// Opens one regular file relative to an authorized directory and stores it
    /// as a separately typed, attenuated resource capability.
    ///
    /// This blocking call is intended for the filesystem/host worker, never the
    /// JavaScript executor.
    ///
    /// # Errors
    ///
    /// Rejects path escape/symlink, authority increase, wrong type, stale
    /// directory, and resource exhaustion before returning a handle.
    pub fn open_relative(
        &self,
        directory: ResourceHandle,
        path: &RelativePath,
        rights: FilesystemRights,
        create: bool,
        truncate: bool,
    ) -> Result<ResourceHandle, FilesystemError> {
        let file_rights = FilesystemRights::READ
            .union(FilesystemRights::WRITE)
            .union(FilesystemRights::METADATA);
        if rights.is_empty() || rights.0 & !file_rights.0 != 0 {
            return Err(FilesystemError::PermissionDenied);
        }
        let mut directory_rights = rights;
        if create {
            directory_rights = directory_rights.union(FilesystemRights::CREATE);
        }
        if truncate {
            directory_rights = directory_rights.union(FilesystemRights::TRUNCATE);
        }
        let root = self.duplicate_root(directory, directory_rights)?;
        let (parent, name) = walk_parent(&root, path)?;
        reject_destination_symlink(&parent, name)?;
        let mut flags = OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK;
        flags |= if rights.contains(FilesystemRights::READ)
            && rights.contains(FilesystemRights::WRITE)
        {
            OFlags::RDWR
        } else if rights.contains(FilesystemRights::WRITE) {
            OFlags::WRONLY
        } else {
            OFlags::RDONLY
        };
        if create {
            flags |= OFlags::CREATE;
        }
        if truncate {
            flags |= OFlags::TRUNC;
        }
        let descriptor = openat(&parent, name, flags, FILE_MODE).map_err(map_open_error)?;
        require_regular_file(&descriptor)?;
        self.resources
            .lock()
            .map_err(|_| FilesystemError::Internal)?
            .insert(
                self.file_kind.clone(),
                ResourceRights::from_bits(rights.0),
                FileResource {
                    descriptor: Some(descriptor),
                },
            )
            .map_err(Into::into)
    }

    /// Enqueues a bounded read on an opened file resource.
    ///
    /// # Errors
    ///
    /// Rejects stale/wrong-kind handles, missing read authority, invalid
    /// limits, and worker saturation.
    pub fn submit_file_read(
        &self,
        request_id: u64,
        file: ResourceHandle,
        max_bytes: usize,
    ) -> Result<CancellationToken, FilesystemError> {
        if max_bytes == 0 || max_bytes > self.max_operation_bytes {
            return Err(FilesystemError::QuotaExceeded);
        }
        let descriptor = self.duplicate_file(file, FilesystemRights::READ)?;
        self.workers
            .submit(request_id, move |cancellation| {
                let mut file = File::from(descriptor);
                read_bounded(&mut file, max_bytes, || cancellation.is_cancelled())
                    .map_err(Into::into)
            })
            .map_err(Into::into)
    }

    /// Enqueues a bounded write on an opened file resource.
    ///
    /// # Errors
    ///
    /// Rejects stale/wrong-kind handles, missing write authority, oversized
    /// input, and worker saturation.
    pub fn submit_file_write(
        &self,
        request_id: u64,
        file: ResourceHandle,
        data: Vec<u8>,
    ) -> Result<CancellationToken, FilesystemError> {
        if data.len() > self.max_operation_bytes {
            return Err(FilesystemError::QuotaExceeded);
        }
        let descriptor = self.duplicate_file(file, FilesystemRights::WRITE)?;
        self.workers
            .submit(request_id, move |cancellation| {
                let mut file = File::from(descriptor);
                write_bounded(&mut file, &data, || cancellation.is_cancelled())?;
                Ok(Vec::new())
            })
            .map_err(Into::into)
    }

    /// Enqueues metadata for an opened file without reauthorizing its old path.
    ///
    /// # Errors
    ///
    /// Rejects stale/wrong-kind handles, missing metadata authority, and
    /// worker saturation.
    pub fn submit_file_metadata(
        &self,
        request_id: u64,
        file: ResourceHandle,
    ) -> Result<CancellationToken, FilesystemError> {
        let descriptor = self.duplicate_file(file, FilesystemRights::METADATA)?;
        self.workers
            .submit(request_id, move |cancellation| {
                if cancellation.is_cancelled() {
                    return Err(WorkerError::Cancelled);
                }
                file_metadata(&descriptor).map_err(Into::into)
            })
            .map_err(Into::into)
    }

    fn insert_directory(
        &self,
        descriptor: OwnedFd,
        rights: FilesystemRights,
    ) -> Result<ResourceHandle, FilesystemError> {
        self.resources
            .lock()
            .map_err(|_| FilesystemError::Internal)?
            .insert(
                self.directory_kind.clone(),
                ResourceRights::from_bits(rights.0),
                DirectoryResource {
                    descriptor: Some(descriptor),
                },
            )
            .map_err(Into::into)
    }

    /// Closes an opened directory and invalidates its generation.
    ///
    /// # Errors
    ///
    /// Rejects forged, cross-runtime, stale, or already-closed handles.
    pub fn close(&self, handle: ResourceHandle) -> Result<(), FilesystemError> {
        self.resources
            .lock()
            .map_err(|_| FilesystemError::Internal)?
            .close(handle)
            .map_err(Into::into)
    }

    /// Validates the resource and rights, duplicates its root descriptor, then
    /// enqueues blocking work without retaining the resource-table lock.
    ///
    /// # Errors
    ///
    /// Rejects invalid resources/rights, oversized operations, queue
    /// exhaustion, invalid request IDs, and shutdown state.
    pub fn submit(
        &self,
        request_id: u64,
        handle: ResourceHandle,
        operation: FilesystemOperation,
    ) -> Result<CancellationToken, FilesystemError> {
        validate_operation_limits(&operation, self.max_operation_bytes)?;
        let root = self.duplicate_root(handle, operation.required_right())?;
        self.workers
            .submit(request_id, move |cancellation| {
                execute_operation(&root, operation, &cancellation).map_err(Into::into)
            })
            .map_err(Into::into)
    }

    /// Non-blockingly drains one filesystem completion.
    ///
    /// # Errors
    ///
    /// Returns an internal error if worker synchronization was poisoned.
    pub fn next_completion(&self) -> Result<Option<WorkerCompletion>, FilesystemError> {
        self.workers.next_completion().map_err(Into::into)
    }

    fn duplicate_root(
        &self,
        handle: ResourceHandle,
        right: FilesystemRights,
    ) -> Result<OwnedFd, FilesystemError> {
        self.resources
            .lock()
            .map_err(|_| FilesystemError::Internal)?
            .with_resource(
                handle,
                &self.directory_kind,
                ResourceRights::from_bits(right.0),
                |resource| {
                    resource
                        .as_any_mut()
                        .downcast_mut::<DirectoryResource>()
                        .and_then(|directory| directory.descriptor.as_ref())
                        .ok_or(FilesystemError::InvalidResource)
                        .and_then(|descriptor| {
                            rustix::io::dup(descriptor).map_err(|_| FilesystemError::Io)
                        })
                },
            )?
    }

    fn duplicate_file(
        &self,
        handle: ResourceHandle,
        right: FilesystemRights,
    ) -> Result<OwnedFd, FilesystemError> {
        self.resources
            .lock()
            .map_err(|_| FilesystemError::Internal)?
            .with_resource(
                handle,
                &self.file_kind,
                ResourceRights::from_bits(right.0),
                |resource| {
                    resource
                        .as_any_mut()
                        .downcast_mut::<FileResource>()
                        .and_then(|file| file.descriptor.as_ref())
                        .ok_or(FilesystemError::InvalidResource)
                        .and_then(|descriptor| {
                            rustix::io::dup(descriptor).map_err(|_| FilesystemError::Io)
                        })
                },
            )?
    }
}

fn directory_identity(descriptor: &OwnedFd) -> Result<DirectoryIdentity, FilesystemError> {
    let stat = fstat(descriptor).map_err(|_| FilesystemError::Io)?;
    let device = u64::try_from(stat.st_dev).map_err(|_| FilesystemError::InvalidIdentity)?;
    let file = stat.st_ino;
    if device == 0 || file == 0 {
        return Err(FilesystemError::InvalidIdentity);
    }
    Ok(DirectoryIdentity { device, file })
}

fn validate_approved_locator(path: &Path) -> Result<(), FilesystemError> {
    let bytes = path.as_os_str().as_bytes();
    if !path.is_absolute()
        || bytes.is_empty()
        || bytes.len() > MAX_PATH_BYTES
        || bytes.contains(&0)
        || path.to_str().is_none()
    {
        return Err(FilesystemError::InvalidPath);
    }
    let mut count = 0;
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::Normal(value) if valid_locator_component(value) => count += 1,
            _ => return Err(FilesystemError::InvalidPath),
        }
    }
    if count == 0 || count > MAX_COMPONENTS {
        return Err(FilesystemError::InvalidPath);
    }
    Ok(())
}

fn valid_locator_component(value: &OsStr) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty() && bytes.len() <= MAX_COMPONENT_BYTES && !bytes.contains(&0)
}

fn open_approved_root(path: &Path) -> Result<OwnedFd, FilesystemError> {
    let mut current = openat(
        CWD,
        "/",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(|_| FilesystemError::Io)?;
    for component in path.components() {
        if let Component::Normal(component) = component {
            reject_symlink_os(&current, component)?;
            current = openat(
                &current,
                component,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(map_open_error)?;
            require_directory(&current)?;
        }
    }
    Ok(current)
}

fn validate_operation_limits(
    operation: &FilesystemOperation,
    max_bytes: usize,
) -> Result<(), FilesystemError> {
    match operation {
        FilesystemOperation::ReadFile {
            max_bytes: requested,
            ..
        } if *requested == 0 || *requested > max_bytes => Err(FilesystemError::QuotaExceeded),
        FilesystemOperation::WriteFile { data, .. } if data.len() > max_bytes => {
            Err(FilesystemError::QuotaExceeded)
        }
        FilesystemOperation::ListDirectory {
            max_entries,
            max_encoded_bytes,
            ..
        } if *max_entries == 0
            || *max_encoded_bytes < 4
            || *max_encoded_bytes > max_bytes
            || *max_entries > (max_encoded_bytes.saturating_sub(4) / 3) =>
        {
            Err(FilesystemError::QuotaExceeded)
        }
        _ => Ok(()),
    }
}

fn execute_operation(
    root: &OwnedFd,
    operation: FilesystemOperation,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, FilesystemError> {
    if cancellation.is_cancelled() {
        return Err(FilesystemError::Cancelled);
    }
    match operation {
        FilesystemOperation::ReadFile { path, max_bytes } => {
            read_file(root, &path, max_bytes, cancellation)
        }
        FilesystemOperation::WriteFile {
            path,
            data,
            create,
            truncate,
        } => {
            write_file(root, &path, &data, create, truncate, cancellation)?;
            Ok(Vec::new())
        }
        FilesystemOperation::CreateDirectory { path } => {
            let (parent, name) = walk_parent(root, &path)?;
            mkdirat(&parent, name, DIRECTORY_MODE).map_err(map_io_error)?;
            Ok(Vec::new())
        }
        FilesystemOperation::ListDirectory {
            path,
            max_entries,
            max_encoded_bytes,
        } => list_directory(
            root,
            path.as_ref(),
            max_entries,
            max_encoded_bytes,
            cancellation,
        ),
        FilesystemOperation::Metadata { path } => metadata(root, &path),
        FilesystemOperation::RemoveFile { path } => {
            let (parent, name) = walk_parent(root, &path)?;
            reject_symlink(&parent, name)?;
            unlinkat(&parent, name, AtFlags::empty()).map_err(map_io_error)?;
            Ok(Vec::new())
        }
        FilesystemOperation::RemoveDirectory { path } => {
            let (parent, name) = walk_parent(root, &path)?;
            reject_symlink(&parent, name)?;
            unlinkat(&parent, name, AtFlags::REMOVEDIR).map_err(map_io_error)?;
            Ok(Vec::new())
        }
        FilesystemOperation::Rename {
            source,
            destination,
        } => {
            let (source_parent, source_name) = walk_parent(root, &source)?;
            let (destination_parent, destination_name) = walk_parent(root, &destination)?;
            reject_symlink(&source_parent, source_name)?;
            reject_destination_symlink(&destination_parent, destination_name)?;
            renameat(
                &source_parent,
                source_name,
                &destination_parent,
                destination_name,
            )
            .map_err(map_io_error)?;
            Ok(Vec::new())
        }
    }
}

fn walk_parent<'a>(
    root: &OwnedFd,
    path: &'a RelativePath,
) -> Result<(OwnedFd, &'a str), FilesystemError> {
    let (name, parents) = path
        .components
        .split_last()
        .ok_or(FilesystemError::InvalidPath)?;
    let parent = walk_directories(root, parents)?;
    Ok((parent, name))
}

fn walk_directories(root: &OwnedFd, components: &[String]) -> Result<OwnedFd, FilesystemError> {
    let mut current = rustix::io::dup(root).map_err(|_| FilesystemError::Io)?;
    for component in components {
        reject_symlink(&current, component)?;
        current = openat(
            &current,
            component,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(map_open_error)?;
        require_directory(&current)?;
    }
    Ok(current)
}

fn read_file(
    root: &OwnedFd,
    path: &RelativePath,
    max_bytes: usize,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, FilesystemError> {
    let (parent, name) = walk_parent(root, path)?;
    reject_symlink(&parent, name)?;
    let descriptor = openat(
        &parent,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(map_open_error)?;
    require_regular_file(&descriptor)?;
    let mut file = File::from(descriptor);
    read_bounded(&mut file, max_bytes, || cancellation.is_cancelled())
}

fn read_bounded(
    reader: &mut impl Read,
    max_bytes: usize,
    mut is_cancelled: impl FnMut() -> bool,
) -> Result<Vec<u8>, FilesystemError> {
    let mut output = Vec::with_capacity(max_bytes.min(64 * 1024));
    let mut chunk = [0_u8; 16 * 1024];
    let mut transient_retries = 0;
    loop {
        if is_cancelled() {
            return Err(FilesystemError::Cancelled);
        }
        let read = match reader.read(&mut chunk) {
            Ok(read) => {
                transient_retries = 0;
                read
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                ) && transient_retries < MAX_TRANSIENT_IO_RETRIES =>
            {
                transient_retries += 1;
                std::thread::yield_now();
                continue;
            }
            Err(_) => return Err(FilesystemError::Io),
        };
        if read == 0 {
            return Ok(output);
        }
        if output.len().saturating_add(read) > max_bytes {
            return Err(FilesystemError::QuotaExceeded);
        }
        output.extend_from_slice(&chunk[..read]);
    }
}

fn write_file(
    root: &OwnedFd,
    path: &RelativePath,
    data: &[u8],
    create: bool,
    truncate: bool,
    cancellation: &CancellationToken,
) -> Result<(), FilesystemError> {
    let (parent, name) = walk_parent(root, path)?;
    reject_destination_symlink(&parent, name)?;
    let mut flags = OFlags::WRONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
    if create {
        flags |= OFlags::CREATE;
    }
    if truncate {
        flags |= OFlags::TRUNC;
    }
    let descriptor = openat(&parent, name, flags, FILE_MODE).map_err(map_open_error)?;
    require_regular_file(&descriptor)?;
    if cancellation.is_cancelled() {
        return Err(FilesystemError::Cancelled);
    }
    let mut file = File::from(descriptor);
    write_bounded(&mut file, data, || cancellation.is_cancelled())
}

fn write_bounded(
    writer: &mut impl Write,
    data: &[u8],
    mut is_cancelled: impl FnMut() -> bool,
) -> Result<(), FilesystemError> {
    let mut written = 0;
    let mut transient_retries = 0;
    while written < data.len() {
        if is_cancelled() {
            return Err(FilesystemError::Cancelled);
        }
        let end = written.saturating_add(16 * 1024).min(data.len());
        let count = match writer.write(&data[written..end]) {
            Ok(count) => {
                transient_retries = 0;
                count
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::Interrupted | std::io::ErrorKind::WouldBlock
                ) && transient_retries < MAX_TRANSIENT_IO_RETRIES =>
            {
                transient_retries += 1;
                std::thread::yield_now();
                continue;
            }
            Err(_) => return Err(FilesystemError::Io),
        };
        if count == 0 {
            return Err(FilesystemError::Io);
        }
        written += count;
    }
    Ok(())
}

fn list_directory(
    root: &OwnedFd,
    path: Option<&RelativePath>,
    max_entries: usize,
    max_encoded_bytes: usize,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, FilesystemError> {
    let directory = path.map_or_else(
        || rustix::io::dup(root).map_err(|_| FilesystemError::Io),
        |path| walk_directories(root, &path.components),
    )?;
    let mut names = Vec::new();
    let mut encoded_bytes = 4_usize;
    for entry in Dir::read_from(&directory).map_err(|_| FilesystemError::Io)? {
        if cancellation.is_cancelled() {
            return Err(FilesystemError::Cancelled);
        }
        let entry = entry.map_err(|_| FilesystemError::Io)?;
        let name = entry.file_name().to_bytes();
        if matches!(name, b"." | b"..") {
            continue;
        }
        let name = std::str::from_utf8(name).map_err(|_| FilesystemError::Protocol)?;
        if name.nfc().ne(name.chars()) {
            return Err(FilesystemError::Protocol);
        }
        encoded_bytes = encoded_bytes
            .checked_add(2)
            .and_then(|length| length.checked_add(name.len()))
            .ok_or(FilesystemError::QuotaExceeded)?;
        if encoded_bytes > max_encoded_bytes {
            return Err(FilesystemError::QuotaExceeded);
        }
        names.push(name.as_bytes().to_vec());
        if names.len() > max_entries {
            return Err(FilesystemError::QuotaExceeded);
        }
    }
    names.sort_unstable();
    encode_names(&names, max_encoded_bytes)
}

fn encode_names(names: &[Vec<u8>], max_bytes: usize) -> Result<Vec<u8>, FilesystemError> {
    let mut output = Vec::new();
    output.extend_from_slice(
        &u32::try_from(names.len())
            .map_err(|_| FilesystemError::QuotaExceeded)?
            .to_be_bytes(),
    );
    for name in names {
        let length = u16::try_from(name.len()).map_err(|_| FilesystemError::Protocol)?;
        if output.len().saturating_add(2).saturating_add(name.len()) > max_bytes {
            return Err(FilesystemError::QuotaExceeded);
        }
        output.extend_from_slice(&length.to_be_bytes());
        output.extend_from_slice(name);
    }
    Ok(output)
}

fn metadata(root: &OwnedFd, path: &RelativePath) -> Result<Vec<u8>, FilesystemError> {
    let (parent, name) = walk_parent(root, path)?;
    let stat = statat(&parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(map_io_error)?;
    let file_type = FileType::from_raw_mode(stat.st_mode);
    if file_type.is_symlink() {
        return Err(FilesystemError::SymlinkDenied);
    }
    let kind = if file_type.is_file() {
        1
    } else if file_type.is_dir() {
        2
    } else {
        return Err(FilesystemError::NotSupported);
    };
    let length = u64::try_from(stat.st_size).map_err(|_| FilesystemError::Protocol)?;
    let mut output = Vec::with_capacity(9);
    output.push(kind);
    output.extend_from_slice(&length.to_be_bytes());
    Ok(output)
}

fn file_metadata(descriptor: &OwnedFd) -> Result<Vec<u8>, FilesystemError> {
    let stat = fstat(descriptor).map_err(|_| FilesystemError::Io)?;
    if !FileType::from_raw_mode(stat.st_mode).is_file() {
        return Err(FilesystemError::WrongKind);
    }
    let length = u64::try_from(stat.st_size).map_err(|_| FilesystemError::Protocol)?;
    Ok([vec![1], length.to_be_bytes().to_vec()].concat())
}

fn require_regular_file(descriptor: &OwnedFd) -> Result<(), FilesystemError> {
    let stat = fstat(descriptor).map_err(|_| FilesystemError::Io)?;
    let file_type = FileType::from_raw_mode(stat.st_mode);
    if file_type.is_file() {
        Ok(())
    } else if file_type.is_symlink() {
        Err(FilesystemError::SymlinkDenied)
    } else {
        Err(FilesystemError::NotSupported)
    }
}

fn require_directory(descriptor: &OwnedFd) -> Result<(), FilesystemError> {
    let stat = fstat(descriptor).map_err(|_| FilesystemError::Io)?;
    let file_type = FileType::from_raw_mode(stat.st_mode);
    if file_type.is_dir() {
        Ok(())
    } else if file_type.is_symlink() {
        Err(FilesystemError::SymlinkDenied)
    } else {
        Err(FilesystemError::WrongKind)
    }
}

fn reject_symlink(parent: &OwnedFd, name: &str) -> Result<(), FilesystemError> {
    reject_symlink_os(parent, OsStr::new(name))
}

fn reject_symlink_os(parent: &OwnedFd, name: &OsStr) -> Result<(), FilesystemError> {
    let stat = statat(parent, name, AtFlags::SYMLINK_NOFOLLOW).map_err(map_io_error)?;
    if FileType::from_raw_mode(stat.st_mode).is_symlink() {
        Err(FilesystemError::SymlinkDenied)
    } else {
        Ok(())
    }
}

fn reject_destination_symlink(parent: &OwnedFd, name: &str) -> Result<(), FilesystemError> {
    match statat(parent, name, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(stat) if FileType::from_raw_mode(stat.st_mode).is_symlink() => {
            Err(FilesystemError::SymlinkDenied)
        }
        Ok(_) | Err(rustix::io::Errno::NOENT) => Ok(()),
        Err(_) => Err(FilesystemError::Io),
    }
}

fn map_open_error(error: rustix::io::Errno) -> FilesystemError {
    if matches!(error, rustix::io::Errno::LOOP) {
        FilesystemError::SymlinkDenied
    } else {
        map_io_error(error)
    }
}

fn map_io_error(error: rustix::io::Errno) -> FilesystemError {
    match error {
        rustix::io::Errno::NOENT => FilesystemError::NotFound,
        rustix::io::Errno::EXIST => FilesystemError::AlreadyExists,
        rustix::io::Errno::ACCESS | rustix::io::Errno::PERM => FilesystemError::PermissionDenied,
        rustix::io::Errno::NOTDIR | rustix::io::Errno::ISDIR => FilesystemError::WrongKind,
        _ => FilesystemError::Io,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilesystemError {
    InvalidPath,
    InvalidIdentity,
    IdentityMismatch,
    InvalidResource,
    PermissionDenied,
    ResourceClosed,
    WrongKind,
    SymlinkDenied,
    NotFound,
    AlreadyExists,
    NotSupported,
    QuotaExceeded,
    Cancelled,
    Io,
    Protocol,
    ShuttingDown,
    Internal,
}

impl From<ResourceError> for FilesystemError {
    fn from(error: ResourceError) -> Self {
        match error {
            ResourceError::InvalidResource | ResourceError::WrongRuntime => Self::InvalidResource,
            ResourceError::WrongKind => Self::WrongKind,
            ResourceError::PermissionDenied | ResourceError::InvalidRights => {
                Self::PermissionDenied
            }
            ResourceError::ResourceClosed => Self::ResourceClosed,
            ResourceError::QuotaExceeded | ResourceError::IdentifierExhausted => {
                Self::QuotaExceeded
            }
            ResourceError::InvalidKind => Self::Internal,
        }
    }
}

impl From<WorkerPoolError> for FilesystemError {
    fn from(error: WorkerPoolError) -> Self {
        match error {
            WorkerPoolError::InvalidRequest => Self::InvalidResource,
            WorkerPoolError::QueueFull => Self::QuotaExceeded,
            WorkerPoolError::ShuttingDown => Self::ShuttingDown,
            WorkerPoolError::InvalidConfig
            | WorkerPoolError::Poisoned
            | WorkerPoolError::ThreadCreation => Self::Internal,
        }
    }
}

impl From<FilesystemError> for WorkerError {
    fn from(error: FilesystemError) -> Self {
        match error {
            FilesystemError::InvalidPath => Self::InvalidArgument,
            FilesystemError::InvalidResource => Self::InvalidResource,
            FilesystemError::PermissionDenied
            | FilesystemError::SymlinkDenied
            | FilesystemError::InvalidIdentity
            | FilesystemError::IdentityMismatch => Self::PermissionDenied,
            FilesystemError::ResourceClosed => Self::ResourceClosed,
            FilesystemError::WrongKind | FilesystemError::NotSupported => Self::NotSupported,
            FilesystemError::QuotaExceeded => Self::QuotaExceeded,
            FilesystemError::Cancelled => Self::Cancelled,
            FilesystemError::Protocol => Self::Protocol,
            FilesystemError::ShuttingDown => Self::ShuttingDown,
            FilesystemError::NotFound | FilesystemError::AlreadyExists | FilesystemError::Io => {
                Self::Io
            }
            FilesystemError::Internal => Self::WorkerFailed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ApprovedDirectory, DirectoryIdentity, Filesystem, FilesystemError, FilesystemOperation,
        FilesystemRights, RelativePath, read_bounded, write_bounded,
    };
    use runtime_event_loop::CompletionNotifier;
    use runtime_event_loop::worker::{WorkerCompletion, WorkerError};
    use runtime_resource::ResourceOwner;
    use std::collections::VecDeque;
    use std::fs;
    use std::io::{self, Read, Write};
    use std::os::unix::fs::symlink;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, mpsc};
    use std::time::Duration;
    use unicode_normalization::UnicodeNormalization;

    static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    struct ChannelNotifier(mpsc::Sender<()>);

    impl CompletionNotifier for ChannelNotifier {
        fn notify_drain_needed(&self) {
            let _ = self.0.send(());
        }
    }

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let identity = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let directory = std::env::temp_dir().join(format!(
                "zintl-runtime-fs-{}-{label}-{identity}",
                std::process::id()
            ));
            fs::create_dir(&directory).expect("create isolated test directory");
            Self(fs::canonicalize(directory).expect("canonical test directory"))
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn all_rights() -> FilesystemRights {
        FilesystemRights::READ
            .union(FilesystemRights::WRITE)
            .union(FilesystemRights::CREATE)
            .union(FilesystemRights::METADATA)
            .union(FilesystemRights::ENUMERATE)
            .union(FilesystemRights::TRUNCATE)
    }

    fn service(
        owner: u64,
    ) -> (
        Filesystem,
        mpsc::Receiver<()>,
        ApprovedDirectory,
        TestDirectory,
    ) {
        let directory = TestDirectory::new("root");
        let approved = ApprovedDirectory::from_trusted_approval(directory.path(), all_rights())
            .expect("approved locator");
        let (sender, receiver) = mpsc::channel();
        let filesystem = Filesystem::new(
            ResourceOwner::new(owner).expect("owner"),
            8,
            2,
            8,
            8,
            1_024,
            Arc::new(ChannelNotifier(sender)),
        )
        .expect("filesystem");
        (filesystem, receiver, approved, directory)
    }

    fn completion(filesystem: &Filesystem, receiver: &mpsc::Receiver<()>) -> WorkerCompletion {
        receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("worker notification");
        filesystem
            .next_completion()
            .expect("completion drain")
            .expect("completion")
    }

    fn path(value: &str) -> RelativePath {
        RelativePath::parse(value).expect("valid relative path")
    }

    struct ScriptedReader {
        steps: VecDeque<Result<Vec<u8>, io::ErrorKind>>,
    }

    impl Read for ScriptedReader {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            match self.steps.pop_front().unwrap_or_else(|| Ok(Vec::new())) {
                Ok(bytes) => {
                    let count = bytes.len().min(output.len());
                    output[..count].copy_from_slice(&bytes[..count]);
                    Ok(count)
                }
                Err(kind) => Err(io::Error::from(kind)),
            }
        }
    }

    struct ScriptedWriter {
        steps: VecDeque<Result<usize, io::ErrorKind>>,
        output: Vec<u8>,
    }

    impl Write for ScriptedWriter {
        fn write(&mut self, input: &[u8]) -> io::Result<usize> {
            match self.steps.pop_front().unwrap_or(Ok(input.len())) {
                Ok(maximum) => {
                    let count = maximum.min(input.len());
                    self.output.extend_from_slice(&input[..count]);
                    Ok(count)
                }
                Err(kind) => Err(io::Error::from(kind)),
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    // Verifies partial I/O, EINTR, would-block, and EOF follow bounded retry semantics.
    fn stream_helpers_handle_partial_and_transient_io() {
        let mut reader = ScriptedReader {
            steps: VecDeque::from([
                Err(io::ErrorKind::Interrupted),
                Err(io::ErrorKind::WouldBlock),
                Ok(vec![1, 2]),
                Ok(vec![3]),
                Ok(Vec::new()),
            ]),
        };
        assert_eq!(read_bounded(&mut reader, 8, || false), Ok(vec![1, 2, 3]));

        let mut writer = ScriptedWriter {
            steps: VecDeque::from([
                Err(io::ErrorKind::Interrupted),
                Err(io::ErrorKind::WouldBlock),
                Ok(1),
                Ok(2),
            ]),
            output: Vec::new(),
        };
        write_bounded(&mut writer, &[1, 2, 3], || false).expect("bounded write");
        assert_eq!(writer.output, [1, 2, 3]);
    }

    #[test]
    // Verifies untrusted path text cannot express traversal, aliases, or ambiguous normalization.
    fn relative_paths_reject_escape_and_ambiguity() {
        for invalid in [
            "",
            "/absolute",
            "../escape",
            "parent/../escape",
            "parent//child",
            "./child",
            "windows\\child",
            "e\u{301}.txt",
        ] {
            assert_eq!(
                RelativePath::parse(invalid),
                Err(FilesystemError::InvalidPath),
                "accepted {invalid:?}"
            );
        }
        assert!(RelativePath::parse("parent/é.txt").is_ok());
    }

    #[test]
    // Verifies generated separator, Unicode, NUL, and component combinations parse fail-closed.
    fn generated_path_inputs_preserve_parser_invariants() {
        let alphabet = ['a', '/', '.', '\\', '\0', 'é', '\u{301}'];
        let mut state = 0xd1b5_4a32_d192_ed03_u64;
        for length in 0..512 {
            let mut value = String::new();
            for _ in 0..length {
                state = state.rotate_left(9).wrapping_add(0x9e37_79b9);
                let index = usize::from(state.to_le_bytes()[0]) % alphabet.len();
                value.push(alphabet[index]);
            }
            if RelativePath::parse(&value).is_ok() {
                assert!(!value.starts_with('/'));
                assert!(!value.contains(['\0', '\\']));
                assert!(
                    value
                        .split('/')
                        .all(|item| !matches!(item, "" | "." | ".."))
                );
                assert!(value.nfc().eq(value.chars()));
            }
        }
    }

    #[test]
    // Verifies a denied or authority-free decision cannot become an opened directory.
    fn approval_requires_explicit_nonempty_rights() {
        let directory = TestDirectory::new("denied");
        assert!(
            ApprovedDirectory::from_trusted_approval(directory.path(), FilesystemRights::READ)
                .is_ok()
        );
        assert_eq!(
            ApprovedDirectory::from_trusted_approval(directory.path(), FilesystemRights(0))
                .expect_err("empty authority must fail"),
            FilesystemError::PermissionDenied
        );
        assert_eq!(
            ApprovedDirectory::from_trusted_approval("relative", FilesystemRights::READ)
                .expect_err("relative locator must fail"),
            FilesystemError::InvalidPath
        );
    }

    #[test]
    // Verifies create, write, read, metadata, listing, rename, and remove stay beneath one root.
    fn descriptor_relative_operations_round_trip() {
        let (filesystem, receiver, approved, _directory) = service(1);
        let handle = filesystem
            .open_approved_directory(&approved)
            .expect("opened resource");

        filesystem
            .submit(
                1,
                handle,
                FilesystemOperation::CreateDirectory {
                    path: path("child"),
                },
            )
            .expect("create submitted");
        assert_eq!(completion(&filesystem, &receiver).result, Ok(Vec::new()));

        filesystem
            .submit(
                2,
                handle,
                FilesystemOperation::WriteFile {
                    path: path("child/data.bin"),
                    data: vec![1, 2, 3],
                    create: true,
                    truncate: true,
                },
            )
            .expect("write submitted");
        assert_eq!(completion(&filesystem, &receiver).result, Ok(Vec::new()));

        filesystem
            .submit(
                3,
                handle,
                FilesystemOperation::ReadFile {
                    path: path("child/data.bin"),
                    max_bytes: 4,
                },
            )
            .expect("read submitted");
        assert_eq!(completion(&filesystem, &receiver).result, Ok(vec![1, 2, 3]));

        filesystem
            .submit(
                4,
                handle,
                FilesystemOperation::Metadata {
                    path: path("child/data.bin"),
                },
            )
            .expect("metadata submitted");
        let metadata = completion(&filesystem, &receiver).result.expect("metadata");
        assert_eq!(metadata, [vec![1], 3_u64.to_be_bytes().to_vec()].concat());

        filesystem
            .submit(
                5,
                handle,
                FilesystemOperation::ListDirectory {
                    path: Some(path("child")),
                    max_entries: 4,
                    max_encoded_bytes: 64,
                },
            )
            .expect("list submitted");
        assert_eq!(
            completion(&filesystem, &receiver).result,
            Ok([
                0_u8, 0, 0, 1, 0, 8, b'd', b'a', b't', b'a', b'.', b'b', b'i', b'n'
            ]
            .to_vec())
        );

        filesystem
            .submit(
                6,
                handle,
                FilesystemOperation::Rename {
                    source: path("child/data.bin"),
                    destination: path("child/renamed.bin"),
                },
            )
            .expect("rename submitted");
        assert_eq!(completion(&filesystem, &receiver).result, Ok(Vec::new()));
        filesystem
            .submit(
                7,
                handle,
                FilesystemOperation::RemoveFile {
                    path: path("child/renamed.bin"),
                },
            )
            .expect("remove file submitted");
        assert_eq!(completion(&filesystem, &receiver).result, Ok(Vec::new()));
        filesystem
            .submit(
                8,
                handle,
                FilesystemOperation::RemoveDirectory {
                    path: path("child"),
                },
            )
            .expect("remove directory submitted");
        assert_eq!(completion(&filesystem, &receiver).result, Ok(Vec::new()));
    }

    #[test]
    // Verifies intermediate and leaf symlinks cannot redirect an authorized operation.
    fn symlink_escape_is_denied() {
        let outside = TestDirectory::new("outside");
        fs::write(outside.path().join("secret"), b"secret").expect("outside file");
        let (filesystem, receiver, approved, directory) = service(1);
        symlink(outside.path(), directory.path().join("escape")).expect("directory symlink");
        symlink(outside.path().join("secret"), directory.path().join("leaf"))
            .expect("file symlink");
        let handle = filesystem
            .open_approved_directory(&approved)
            .expect("opened resource");

        for (request_id, target) in [(1, "escape/secret"), (2, "leaf")] {
            filesystem
                .submit(
                    request_id,
                    handle,
                    FilesystemOperation::ReadFile {
                        path: path(target),
                        max_bytes: 16,
                    },
                )
                .expect("read submitted");
            assert_eq!(
                completion(&filesystem, &receiver).result,
                Err(WorkerError::PermissionDenied)
            );
        }
    }

    #[test]
    // Verifies an approved locator is itself walked without following a symlink component.
    fn approved_locator_symlink_is_denied() {
        let outside = TestDirectory::new("approved-target");
        let parent = TestDirectory::new("approved-parent");
        let locator = parent.path().join("link");
        symlink(outside.path(), &locator).expect("locator symlink");
        let approved = ApprovedDirectory::from_trusted_approval(&locator, FilesystemRights::READ)
            .expect("trusted approval records untrusted locator");
        let (sender, _receiver) = mpsc::channel();
        let filesystem = Filesystem::new(
            ResourceOwner::new(1).expect("owner"),
            1,
            1,
            1,
            1,
            16,
            Arc::new(ChannelNotifier(sender)),
        )
        .expect("filesystem");
        assert_eq!(
            filesystem
                .open_approved_directory(&approved)
                .expect_err("locator symlink must not open"),
            FilesystemError::SymlinkDenied
        );
    }

    #[test]
    // Verifies imported permission reopening mints authority only for the authenticated directory identity.
    fn imported_directory_reopen_checks_identity_before_insert() {
        let (filesystem, _receiver, approved, _directory) = service(1);
        let original = filesystem
            .open_approved_directory(&approved)
            .expect("opened original");
        let identity = filesystem
            .directory_identity(original)
            .expect("trusted identity");
        assert_eq!(
            DirectoryIdentity::from_authenticated_bytes(&identity.authenticated_bytes())
                .expect("decoded identity"),
            identity
        );
        assert!(
            filesystem
                .open_imported_directory(&approved, identity)
                .is_ok()
        );

        let replacement = TestDirectory::new("replacement");
        let replacement_approval =
            ApprovedDirectory::from_trusted_approval(replacement.path(), all_rights())
                .expect("replacement approval");
        assert_eq!(
            filesystem
                .open_imported_directory(&replacement_approval, identity)
                .expect_err("identity substitution must fail"),
            FilesystemError::IdentityMismatch
        );
        assert_eq!(
            DirectoryIdentity::from_authenticated_bytes(&[0; 16])
                .expect_err("zero identity must fail"),
            FilesystemError::InvalidIdentity
        );
    }

    #[test]
    // Verifies resource rights are checked before any filesystem worker job is accepted.
    fn resource_rights_fail_closed() {
        let directory = TestDirectory::new("readonly");
        let approved =
            ApprovedDirectory::from_trusted_approval(directory.path(), FilesystemRights::READ)
                .expect("read approval");
        let (sender, _receiver) = mpsc::channel();
        let filesystem = Filesystem::new(
            ResourceOwner::new(1).expect("owner"),
            2,
            1,
            2,
            2,
            32,
            Arc::new(ChannelNotifier(sender)),
        )
        .expect("filesystem");
        let handle = filesystem
            .open_approved_directory(&approved)
            .expect("opened");
        assert_eq!(
            filesystem
                .submit(
                    1,
                    handle,
                    FilesystemOperation::WriteFile {
                        path: path("denied"),
                        data: Vec::new(),
                        create: true,
                        truncate: true,
                    }
                )
                .expect_err("write authority must be absent"),
            FilesystemError::PermissionDenied
        );
        assert!(!directory.path().join("denied").exists());
    }

    #[test]
    // Verifies relative open attenuates directory authority into typed file read/stat/close operations.
    fn opened_file_resource_is_typed_attenuated_and_stale_after_close() {
        let (filesystem, receiver, approved, directory) = service(1);
        fs::write(directory.path().join("data"), b"file resource").expect("fixture");
        let directory_handle = filesystem
            .open_approved_directory(&approved)
            .expect("directory");
        let file_rights = FilesystemRights::READ.union(FilesystemRights::METADATA);
        let file = filesystem
            .open_relative(directory_handle, &path("data"), file_rights, false, false)
            .expect("relative file");
        filesystem
            .submit_file_read(1, file, 64)
            .expect("file read submitted");
        assert_eq!(
            completion(&filesystem, &receiver).result,
            Ok(b"file resource".to_vec())
        );
        filesystem
            .submit_file_metadata(2, file)
            .expect("file stat submitted");
        assert_eq!(
            completion(&filesystem, &receiver).result,
            Ok([vec![1], 13_u64.to_be_bytes().to_vec()].concat())
        );
        assert_eq!(
            filesystem
                .submit_file_write(3, file, vec![1])
                .expect_err("read-only file cannot write"),
            FilesystemError::PermissionDenied
        );
        filesystem.close(file).expect("file closed");
        assert_eq!(
            filesystem
                .submit_file_read(4, file, 1)
                .expect_err("stale file"),
            FilesystemError::InvalidResource
        );
        assert_eq!(
            filesystem
                .submit_file_read(5, directory_handle, 1)
                .expect_err("wrong resource kind"),
            FilesystemError::WrongKind
        );
    }

    #[test]
    // Verifies close invalidates the handle and bounded reads reject excess output.
    fn close_and_read_quota_are_enforced() {
        let (filesystem, receiver, approved, directory) = service(1);
        fs::write(directory.path().join("large"), [0_u8; 8]).expect("fixture");
        let handle = filesystem
            .open_approved_directory(&approved)
            .expect("opened");
        filesystem
            .submit(
                1,
                handle,
                FilesystemOperation::ReadFile {
                    path: path("large"),
                    max_bytes: 4,
                },
            )
            .expect("read submitted");
        assert_eq!(
            completion(&filesystem, &receiver).result,
            Err(WorkerError::QuotaExceeded)
        );
        assert_eq!(
            filesystem
                .submit(
                    2,
                    handle,
                    FilesystemOperation::ListDirectory {
                        path: None,
                        max_entries: usize::MAX,
                        max_encoded_bytes: 8,
                    }
                )
                .expect_err("listing count must be bounded by its byte budget"),
            FilesystemError::QuotaExceeded
        );
        filesystem.close(handle).expect("closed");
        assert_eq!(
            filesystem
                .submit(
                    3,
                    handle,
                    FilesystemOperation::ReadFile {
                        path: path("large"),
                        max_bytes: 4,
                    }
                )
                .expect_err("stale handle"),
            FilesystemError::InvalidResource
        );
    }

    #[test]
    // Verifies close rejects new work while an operation that already duplicated the root finishes safely.
    fn close_race_has_a_defined_inflight_policy() {
        let (filesystem, receiver, approved, directory) = service(1);
        fs::write(directory.path().join("data"), b"already submitted").expect("fixture");
        let handle = filesystem
            .open_approved_directory(&approved)
            .expect("opened");
        filesystem
            .submit(
                1,
                handle,
                FilesystemOperation::ReadFile {
                    path: path("data"),
                    max_bytes: 64,
                },
            )
            .expect("read submitted");

        filesystem.close(handle).expect("close wins for new work");
        assert_eq!(
            filesystem
                .submit(
                    2,
                    handle,
                    FilesystemOperation::ReadFile {
                        path: path("data"),
                        max_bytes: 64,
                    },
                )
                .expect_err("closed handle must reject new work"),
            FilesystemError::InvalidResource
        );
        assert_eq!(
            completion(&filesystem, &receiver).result,
            Ok(b"already submitted".to_vec())
        );
        assert!(filesystem.next_completion().expect("drain state").is_none());
    }

    #[test]
    // Verifies cancellation and worker completion race to one terminal result without duplicate delivery.
    fn cancellation_race_delivers_one_completion() {
        let (filesystem, receiver, approved, directory) = service(1);
        fs::write(directory.path().join("data"), vec![7_u8; 1_024]).expect("fixture");
        let handle = filesystem
            .open_approved_directory(&approved)
            .expect("opened");
        let cancellation = filesystem
            .submit(
                1,
                handle,
                FilesystemOperation::ReadFile {
                    path: path("data"),
                    max_bytes: 1_024,
                },
            )
            .expect("read submitted");
        cancellation.cancel();

        let result = completion(&filesystem, &receiver).result;
        assert!(matches!(result, Ok(_) | Err(WorkerError::Cancelled)));
        assert!(filesystem.next_completion().expect("drain state").is_none());
    }
}
