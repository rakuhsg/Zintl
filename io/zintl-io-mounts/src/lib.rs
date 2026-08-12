//! IO-side capability mounts and their data-only request protocol.

#![forbid(unsafe_code)]

use cap_std::ambient_authority;
#[cfg(not(target_os = "linux"))]
use cap_std::fs::OpenOptions as CapOpenOptions;
use cap_std::fs::{Dir, File};
use messageloop_core::{Sender, SenderResult};
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_OPERATION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct OperationId(pub u64);
impl OperationId {
    #[must_use]
    pub fn new() -> Self {
        Self(NEXT_OPERATION_ID.fetch_add(1, Ordering::Relaxed))
    }
}
impl Default for OperationId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct MountId(pub u64);
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FileHandleId(pub u64);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BoundaryResolve {
    #[default]
    InRoot,
    DescendantsOnly,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SymlinkPolicy {
    #[default]
    Allow,
    Disallow,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DirResolve {
    pub boundary: BoundaryResolve,
    pub symlinks: SymlinkPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Source {
    Directory {
        root_path: PathBuf,
        readonly: bool,
        resolve: DirResolve,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MountConfig {
    pub name: String,
    pub source: Source,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Component {
    Parent,
    Normal(String),
}

/// Once-decoded, platform-neutral path from a `mount://` URI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MountPath {
    components: Vec<Component>,
}

impl MountPath {
    fn resolve(&self, boundary: BoundaryResolve) -> Result<PathBuf, MountError> {
        let mut resolved = Vec::<&str>::new();
        for component in &self.components {
            match component {
                Component::Parent if resolved.pop().is_none() => {
                    if boundary == BoundaryResolve::DescendantsOnly {
                        return Err(MountError::BoundaryViolation);
                    }
                }
                Component::Parent => {}
                Component::Normal(value) => resolved.push(value),
            }
        }
        let mut path = PathBuf::new();
        for component in resolved {
            path.push(component);
        }
        Ok(path)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MountUri {
    pub mount_name: String,
    pub path: MountPath,
}

impl MountUri {
    /// Parses and once-decodes a mount URI.
    ///
    /// # Errors
    /// Returns a specific syntax or security-boundary error for invalid input.
    pub fn parse(uri: &str) -> Result<Self, MountError> {
        let rest = uri
            .strip_prefix("mount://")
            .ok_or(MountError::MalformedUri)?;
        let (encoded_name, encoded_path) = rest.split_once('/').unwrap_or((rest, ""));
        if encoded_name.is_empty() {
            return Err(MountError::EmptyMountName);
        }
        let mount_name = decode_component(encoded_name)?;
        if mount_name.is_empty() || mount_name == "." || mount_name == ".." {
            return Err(MountError::InvalidMountName);
        }
        let mut components = Vec::new();
        for encoded in encoded_path.split('/') {
            if encoded.is_empty() {
                continue;
            }
            let decoded = decode_component(encoded)?;
            match decoded.as_str() {
                "." => {}
                ".." => components.push(Component::Parent),
                _ => components.push(Component::Normal(decoded)),
            }
        }
        Ok(Self {
            mount_name,
            path: MountPath { components },
        })
    }
}

fn decode_component(encoded: &str) -> Result<String, MountError> {
    let bytes = encoded.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err(MountError::MalformedPercentEncoding);
            }
            let high = hex(bytes[index + 1]).ok_or(MountError::MalformedPercentEncoding)?;
            let low = hex(bytes[index + 2]).ok_or(MountError::MalformedPercentEncoding)?;
            let decoded = high * 16 + low;
            if decoded == 0 {
                return Err(MountError::NulByte);
            }
            if matches!(decoded, b'/' | b'\\') {
                return Err(MountError::EncodedSeparator);
            }
            output.push(decoded);
            index += 3;
        } else {
            if matches!(bytes[index], 0 | b'\\') {
                return Err(MountError::InvalidPath);
            }
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(output).map_err(|_| MountError::InvalidUtf8)
}

const fn hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct OpenOptions {
    pub read: bool,
    pub write: bool,
    pub create: bool,
    pub truncate: bool,
}
impl Default for OpenOptions {
    fn default() -> Self {
        Self {
            read: true,
            write: false,
            create: false,
            truncate: false,
        }
    }
}
impl OpenOptions {
    fn mutates(self) -> bool {
        self.write || self.create || self.truncate
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MountOperation {
    Open {
        operation_id: OperationId,
        mount_id: MountId,
        path: MountPath,
        options: OpenOptions,
    },
    Read {
        operation_id: OperationId,
        handle: FileHandleId,
        offset: u64,
        maximum_bytes: usize,
    },
    Write {
        operation_id: OperationId,
        handle: FileHandleId,
        offset: u64,
        bytes: Vec<u8>,
    },
    Close {
        operation_id: OperationId,
        handle: FileHandleId,
    },
    Mkdir {
        operation_id: OperationId,
        mount_id: MountId,
        path: MountPath,
    },
    Unlink {
        operation_id: OperationId,
        mount_id: MountId,
        path: MountPath,
    },
    Rename {
        operation_id: OperationId,
        mount_id: MountId,
        from: MountPath,
        to: MountPath,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MountCompletion {
    Opened {
        operation_id: OperationId,
        handle: FileHandleId,
    },
    Bytes {
        operation_id: OperationId,
        bytes: Vec<u8>,
    },
    Written {
        operation_id: OperationId,
        bytes: usize,
    },
    Done {
        operation_id: OperationId,
    },
    Failed {
        operation_id: OperationId,
        error: MountError,
    },
}

pub trait MountOperationSender: Sender
where
    Self::Message: From<MountOperation>,
{
    /// Queues one data-only mount operation.
    ///
    /// # Errors
    /// Returns `Closed` when the receiving IO loop is gone or terminating.
    fn send_mount_operation(&self, operation: MountOperation) -> SenderResult {
        self.send(operation.into())
    }
}
impl<T> MountOperationSender for T
where
    T: Sender,
    T::Message: From<MountOperation>,
{
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MountError {
    MalformedUri,
    EmptyMountName,
    InvalidMountName,
    MalformedPercentEncoding,
    NulByte,
    EncodedSeparator,
    InvalidUtf8,
    InvalidPath,
    BoundaryViolation,
    SymlinkDisallowed,
    SymlinkLoop,
    ReadOnly,
    MountNotFound,
    HandleNotFound,
    DuplicateMountName,
    NotDirectory,
    Io,
}
impl fmt::Display for MountError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for MountError {}

struct DirectoryMount {
    directory: Dir,
    readonly: bool,
    resolve: DirResolve,
}
struct OpenFile {
    file: File,
    writable: bool,
    mount_id: MountId,
}

/// IO-thread-owned mount and logical file-handle table.
pub struct MountService {
    mounts: HashMap<MountId, DirectoryMount>,
    names: HashMap<String, MountId>,
    files: HashMap<FileHandleId, OpenFile>,
    next_mount: u64,
    next_file: u64,
}

impl Default for MountService {
    fn default() -> Self {
        Self {
            mounts: HashMap::new(),
            names: HashMap::new(),
            files: HashMap::new(),
            next_mount: 1,
            next_file: 1,
        }
    }
}

impl MountService {
    /// Opens and installs one directory mount.
    ///
    /// # Errors
    /// Rejects invalid roots, duplicate names, and OS open failures.
    pub fn create_mount(&mut self, config: MountConfig) -> Result<MountId, MountError> {
        if config.name.is_empty() || self.names.contains_key(&config.name) {
            return Err(MountError::DuplicateMountName);
        }
        let Source::Directory {
            root_path,
            readonly,
            resolve,
        } = config.source;
        if !root_path.is_dir() {
            return Err(MountError::NotDirectory);
        }
        let directory =
            Dir::open_ambient_dir(&root_path, ambient_authority()).map_err(|_| MountError::Io)?;
        let id = MountId(self.next_mount);
        self.next_mount = self.next_mount.checked_add(1).ok_or(MountError::Io)?;
        self.mounts.insert(
            id,
            DirectoryMount {
                directory,
                readonly,
                resolve,
            },
        );
        self.names.insert(config.name, id);
        Ok(id)
    }

    /// Removes a mount and every logical file handle opened through it.
    ///
    /// # Errors
    /// Returns `MountNotFound` for an unknown identifier.
    pub fn remove_mount(&mut self, mount_id: MountId) -> Result<(), MountError> {
        if self.mounts.remove(&mount_id).is_none() {
            return Err(MountError::MountNotFound);
        }
        self.names.retain(|_, id| *id != mount_id);
        self.files.retain(|_, file| file.mount_id != mount_id);
        Ok(())
    }

    /// Performs a bounded convenience read entirely inside the IO subsystem.
    ///
    /// # Errors
    /// Returns URI, boundary, handle, or underlying IO failures.
    pub fn read_uri(&mut self, uri: &str, maximum_bytes: usize) -> Result<Vec<u8>, MountError> {
        if maximum_bytes == 0 {
            return Err(MountError::InvalidPath);
        }
        let uri = MountUri::parse(uri)?;
        let mount_id = self
            .mount_id(&uri.mount_name)
            .ok_or(MountError::MountNotFound)?;
        let opened = self.try_handle(MountOperation::Open {
            operation_id: OperationId::new(),
            mount_id,
            path: uri.path,
            options: OpenOptions::default(),
        })?;
        let MountCompletion::Opened { handle, .. } = opened else {
            return Err(MountError::Io);
        };
        let read = self.try_handle(MountOperation::Read {
            operation_id: OperationId::new(),
            handle,
            offset: 0,
            maximum_bytes,
        });
        self.files.remove(&handle);
        match read? {
            MountCompletion::Bytes { bytes, .. } => Ok(bytes),
            _ => Err(MountError::Io),
        }
    }

    #[must_use]
    pub fn mount_id(&self, name: &str) -> Option<MountId> {
        self.names.get(name).copied()
    }

    #[must_use]
    pub fn handle(&mut self, operation: MountOperation) -> MountCompletion {
        let operation_id = operation.id();
        match self.try_handle(operation) {
            Ok(completion) => completion,
            Err(error) => MountCompletion::Failed {
                operation_id,
                error,
            },
        }
    }

    #[allow(clippy::too_many_lines)]
    fn try_handle(&mut self, operation: MountOperation) -> Result<MountCompletion, MountError> {
        match operation {
            MountOperation::Open {
                operation_id,
                mount_id,
                path,
                options,
            } => {
                let mount = self
                    .mounts
                    .get(&mount_id)
                    .ok_or(MountError::MountNotFound)?;
                if mount.readonly && options.mutates() {
                    return Err(MountError::ReadOnly);
                }
                let path = checked_path(mount, &path)?;
                let file = open_file(mount, &path, options)?;
                let handle = FileHandleId(self.next_file);
                self.next_file = self.next_file.checked_add(1).ok_or(MountError::Io)?;
                self.files.insert(
                    handle,
                    OpenFile {
                        file,
                        writable: options.write,
                        mount_id,
                    },
                );
                Ok(MountCompletion::Opened {
                    operation_id,
                    handle,
                })
            }
            MountOperation::Read {
                operation_id,
                handle,
                offset,
                maximum_bytes,
            } => {
                let open = self
                    .files
                    .get_mut(&handle)
                    .ok_or(MountError::HandleNotFound)?;
                open.file
                    .seek(SeekFrom::Start(offset))
                    .map_err(|_| MountError::Io)?;
                let mut bytes = vec![0; maximum_bytes];
                let count = open.file.read(&mut bytes).map_err(|_| MountError::Io)?;
                bytes.truncate(count);
                Ok(MountCompletion::Bytes {
                    operation_id,
                    bytes,
                })
            }
            MountOperation::Write {
                operation_id,
                handle,
                offset,
                bytes,
            } => {
                let open = self
                    .files
                    .get_mut(&handle)
                    .ok_or(MountError::HandleNotFound)?;
                if !open.writable {
                    return Err(MountError::ReadOnly);
                }
                open.file
                    .seek(SeekFrom::Start(offset))
                    .map_err(|_| MountError::Io)?;
                let count = open.file.write(&bytes).map_err(|_| MountError::Io)?;
                Ok(MountCompletion::Written {
                    operation_id,
                    bytes: count,
                })
            }
            MountOperation::Close {
                operation_id,
                handle,
            } => {
                self.files
                    .remove(&handle)
                    .ok_or(MountError::HandleNotFound)?;
                Ok(MountCompletion::Done { operation_id })
            }
            MountOperation::Mkdir {
                operation_id,
                mount_id,
                path,
            } => {
                let mount = mutable_mount(&self.mounts, mount_id)?;
                mount
                    .directory
                    .create_dir(checked_path(mount, &path)?)
                    .map_err(|_| MountError::Io)?;
                Ok(MountCompletion::Done { operation_id })
            }
            MountOperation::Unlink {
                operation_id,
                mount_id,
                path,
            } => {
                let mount = mutable_mount(&self.mounts, mount_id)?;
                mount
                    .directory
                    .remove_file(checked_path(mount, &path)?)
                    .map_err(|_| MountError::Io)?;
                Ok(MountCompletion::Done { operation_id })
            }
            MountOperation::Rename {
                operation_id,
                mount_id,
                from,
                to,
            } => {
                let mount = mutable_mount(&self.mounts, mount_id)?;
                let from = checked_path(mount, &from)?;
                let to = checked_path(mount, &to)?;
                mount
                    .directory
                    .rename(&from, &mount.directory, &to)
                    .map_err(|_| MountError::Io)?;
                Ok(MountCompletion::Done { operation_id })
            }
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn open_file(
    mount: &DirectoryMount,
    path: &Path,
    options: OpenOptions,
) -> Result<File, MountError> {
    let mut cap = CapOpenOptions::new();
    cap.read(options.read)
        .write(options.write)
        .create(options.create)
        .truncate(options.truncate);
    mount
        .directory
        .open_with(path, &cap)
        .map_err(|_| MountError::Io)
}

#[cfg(target_os = "linux")]
fn open_file(
    mount: &DirectoryMount,
    path: &Path,
    options: OpenOptions,
) -> Result<File, MountError> {
    use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
    let access = match (options.read, options.write) {
        (true, true) => OFlags::RDWR,
        (false, true) => OFlags::WRONLY,
        _ => OFlags::RDONLY,
    };
    let mut flags = access | OFlags::CLOEXEC;
    if options.create {
        flags |= OFlags::CREATE;
    }
    if options.truncate {
        flags |= OFlags::TRUNC;
    }
    let boundary = match mount.resolve.boundary {
        BoundaryResolve::InRoot => ResolveFlags::IN_ROOT,
        BoundaryResolve::DescendantsOnly => ResolveFlags::BENEATH,
    };
    let mut resolve = boundary | ResolveFlags::NO_MAGICLINKS;
    if mount.resolve.symlinks == SymlinkPolicy::Disallow {
        resolve |= ResolveFlags::NO_SYMLINKS;
    }
    let fd = openat2(
        &mount.directory,
        path,
        flags,
        Mode::from_raw_mode(0o666),
        resolve,
    )
    .map_err(|_| MountError::Io)?;
    Ok(File::from_std(std::fs::File::from(fd)))
}

fn mutable_mount(
    mounts: &HashMap<MountId, DirectoryMount>,
    id: MountId,
) -> Result<&DirectoryMount, MountError> {
    let mount = mounts.get(&id).ok_or(MountError::MountNotFound)?;
    if mount.readonly {
        return Err(MountError::ReadOnly);
    }
    Ok(mount)
}

fn checked_path(mount: &DirectoryMount, path: &MountPath) -> Result<PathBuf, MountError> {
    let initial = path.resolve(mount.resolve.boundary)?;
    let mut pending = path_components(&initial)?;
    let mut resolved = Vec::<String>::new();
    let mut followed = 0_u8;
    while let Some(component) = pending.pop_front() {
        match component.as_str() {
            "." | "" => {}
            ".." => {
                if resolved.pop().is_none()
                    && mount.resolve.boundary == BoundaryResolve::DescendantsOnly
                {
                    return Err(MountError::BoundaryViolation);
                }
            }
            _ => {
                let probe = resolved
                    .iter()
                    .chain(std::iter::once(&component))
                    .collect::<PathBuf>();
                match mount.directory.symlink_metadata(&probe) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        if mount.resolve.symlinks == SymlinkPolicy::Disallow {
                            return Err(MountError::SymlinkDisallowed);
                        }
                        followed = followed.checked_add(1).ok_or(MountError::SymlinkLoop)?;
                        if followed > 40 {
                            return Err(MountError::SymlinkLoop);
                        }
                        let target = mount
                            .directory
                            .read_link_contents(&probe)
                            .map_err(|_| MountError::Io)?;
                        if target.is_absolute() {
                            if mount.resolve.boundary == BoundaryResolve::DescendantsOnly {
                                return Err(MountError::BoundaryViolation);
                            }
                            resolved.clear();
                        }
                        let target = path_components(&target)?;
                        for component in target.into_iter().rev() {
                            pending.push_front(component);
                        }
                    }
                    Ok(_) => resolved.push(component),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        resolved.push(component);
                    }
                    Err(_) => return Err(MountError::Io),
                }
            }
        }
    }
    Ok(resolved.into_iter().collect())
}

fn path_components(path: &Path) -> Result<VecDeque<String>, MountError> {
    use std::path::Component as OsComponent;
    let mut result = VecDeque::new();
    for component in path.components() {
        match component {
            OsComponent::RootDir | OsComponent::CurDir => {}
            OsComponent::ParentDir => result.push_back("..".into()),
            OsComponent::Normal(value) => {
                result.push_back(value.to_str().ok_or(MountError::InvalidPath)?.to_owned());
            }
            OsComponent::Prefix(_) => return Err(MountError::InvalidPath),
        }
    }
    Ok(result)
}

impl MountOperation {
    const fn id(&self) -> OperationId {
        match self {
            Self::Open { operation_id, .. }
            | Self::Read { operation_id, .. }
            | Self::Write { operation_id, .. }
            | Self::Close { operation_id, .. }
            | Self::Mkdir { operation_id, .. }
            | Self::Unlink { operation_id, .. }
            | Self::Rename { operation_id, .. } => *operation_id,
        }
    }
}

/// JS-side proxy: IDs and a typed sender only, never an OS descriptor.
pub struct MountHandle<S> {
    id: MountId,
    sender: S,
}
impl<S> MountHandle<S>
where
    S: MountOperationSender,
    S::Message: From<MountOperation>,
{
    #[must_use]
    pub const fn new(id: MountId, sender: S) -> Self {
        Self { id, sender }
    }
    #[must_use]
    pub const fn id(&self) -> MountId {
        self.id
    }
    /// Queues an operation for this proxy's IO-side mount.
    ///
    /// # Errors
    /// Returns `Closed` when its IO loop no longer accepts messages.
    pub fn send(&self, operation: MountOperation) -> SenderResult {
        self.sender.send_mount_operation(operation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn uri(value: &str) -> Result<MountUri, MountError> {
        MountUri::parse(value)
    }

    #[test]
    fn parses_mount_uri_and_decodes_dot_segments_once() {
        // Verifies normal, encoded-dot, parent, and double-encoded components share one representation.
        let parsed = uri("mount://project/src/%2e/a/%2e%2e/%252e%252e.js").unwrap();
        assert_eq!(parsed.mount_name, "project");
        assert_eq!(
            parsed.path.resolve(BoundaryResolve::InRoot).unwrap(),
            PathBuf::from("src/%2e%2e.js")
        );
    }

    #[test]
    fn rejects_malformed_or_ambiguous_mount_uris() {
        // Verifies malformed escapes, empty names, NUL, and encoded separators are rejected.
        for value in [
            "fs://x/a",
            "mount:///a",
            "mount://x/%",
            "mount://x/%GG",
            "mount://x/%00",
            "mount://x/%2f",
            "mount://x/%5C",
        ] {
            assert!(uri(value).is_err(), "accepted {value}");
        }
    }

    #[test]
    fn boundary_policies_clamp_or_reject_parents() {
        // Verifies InRoot clamps traversal while DescendantsOnly rejects it.
        let path = uri("mount://x/../../external").unwrap().path;
        assert_eq!(
            path.resolve(BoundaryResolve::InRoot).unwrap(),
            PathBuf::from("external")
        );
        assert_eq!(
            path.resolve(BoundaryResolve::DescendantsOnly),
            Err(MountError::BoundaryViolation)
        );
    }

    #[test]
    fn service_uses_logical_handles_and_rejects_stale_ids() {
        // Verifies open/read/close never expose an OS descriptor and stale IDs fail.
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("a.txt"), b"hello").unwrap();
        let mut service = MountService::default();
        let mount_id = service
            .create_mount(MountConfig {
                name: "project".into(),
                source: Source::Directory {
                    root_path: root.path().into(),
                    readonly: false,
                    resolve: DirResolve::default(),
                },
            })
            .unwrap();
        let path = uri("mount://project/a.txt").unwrap().path;
        let opened = service.handle(MountOperation::Open {
            operation_id: OperationId(1),
            mount_id,
            path,
            options: OpenOptions::default(),
        });
        let MountCompletion::Opened { handle, .. } = opened else {
            panic!("open failed")
        };
        assert!(
            matches!(service.handle(MountOperation::Read { operation_id: OperationId(2), handle, offset: 0, maximum_bytes: 5 }), MountCompletion::Bytes { bytes, .. } if bytes == b"hello")
        );
        assert!(matches!(
            service.handle(MountOperation::Close {
                operation_id: OperationId(3),
                handle
            }),
            MountCompletion::Done { .. }
        ));
        assert!(matches!(
            service.handle(MountOperation::Read {
                operation_id: OperationId(4),
                handle,
                offset: 0,
                maximum_bytes: 1
            }),
            MountCompletion::Failed {
                error: MountError::HandleNotFound,
                ..
            }
        ));
    }

    #[test]
    fn readonly_rejects_all_mutating_operations() {
        // Verifies mutation policy is enforced before open and directory operations.
        let root = tempfile::tempdir().unwrap();
        let mut service = MountService::default();
        let mount_id = service
            .create_mount(MountConfig {
                name: "ro".into(),
                source: Source::Directory {
                    root_path: root.path().into(),
                    readonly: true,
                    resolve: DirResolve::default(),
                },
            })
            .unwrap();
        let path = uri("mount://ro/new").unwrap().path;
        assert!(matches!(
            service.handle(MountOperation::Open {
                operation_id: OperationId(1),
                mount_id,
                path: path.clone(),
                options: OpenOptions {
                    read: false,
                    write: true,
                    create: true,
                    truncate: false
                }
            }),
            MountCompletion::Failed {
                error: MountError::ReadOnly,
                ..
            }
        ));
        assert!(matches!(
            service.handle(MountOperation::Mkdir {
                operation_id: OperationId(2),
                mount_id,
                path
            }),
            MountCompletion::Failed {
                error: MountError::ReadOnly,
                ..
            }
        ));
    }

    #[test]
    fn removing_mount_invalidates_its_file_handles() {
        // Verifies neither a mount proxy nor an IO-side handle survives mount deletion.
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("a"), b"a").unwrap();
        let mut service = MountService::default();
        let mount_id = service
            .create_mount(MountConfig {
                name: "x".into(),
                source: Source::Directory {
                    root_path: root.path().into(),
                    readonly: false,
                    resolve: DirResolve::default(),
                },
            })
            .unwrap();
        let MountCompletion::Opened { handle, .. } = service.handle(MountOperation::Open {
            operation_id: OperationId(1),
            mount_id,
            path: uri("mount://x/a").unwrap().path,
            options: OpenOptions::default(),
        }) else {
            panic!("open failed")
        };
        service.remove_mount(mount_id).unwrap();
        assert!(matches!(
            service.handle(MountOperation::Read {
                operation_id: OperationId(2),
                handle,
                offset: 0,
                maximum_bytes: 1
            }),
            MountCompletion::Failed {
                error: MountError::HandleNotFound,
                ..
            }
        ));
        assert!(matches!(
            service.handle(MountOperation::Open {
                operation_id: OperationId(3),
                mount_id,
                path: uri("mount://x/a").unwrap().path,
                options: OpenOptions::default()
            }),
            MountCompletion::Failed {
                error: MountError::MountNotFound,
                ..
            }
        ));
    }

    #[test]
    fn rename_checks_both_paths_against_boundary_policy() {
        // Verifies traversal in either rename endpoint is rejected before renameat-style work.
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("a"), b"a").unwrap();
        let mut service = MountService::default();
        let mount_id = service
            .create_mount(MountConfig {
                name: "x".into(),
                source: Source::Directory {
                    root_path: root.path().into(),
                    readonly: false,
                    resolve: DirResolve {
                        boundary: BoundaryResolve::DescendantsOnly,
                        symlinks: SymlinkPolicy::Allow,
                    },
                },
            })
            .unwrap();
        for (from, to) in [
            ("mount://x/../a", "mount://x/b"),
            ("mount://x/a", "mount://x/../b"),
        ] {
            assert!(matches!(
                service.handle(MountOperation::Rename {
                    operation_id: OperationId(1),
                    mount_id,
                    from: uri(from).unwrap().path,
                    to: uri(to).unwrap().path
                }),
                MountCompletion::Failed {
                    error: MountError::BoundaryViolation,
                    ..
                }
            ));
        }
    }

    #[cfg(unix)]
    #[test]
    fn disallow_policy_rejects_symlink_components() {
        // Verifies symlinks encountered during component resolution are rejected.
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("real")).unwrap();
        fs::write(root.path().join("real/a"), b"a").unwrap();
        symlink("real", root.path().join("link")).unwrap();
        let mut service = MountService::default();
        let mount_id = service
            .create_mount(MountConfig {
                name: "x".into(),
                source: Source::Directory {
                    root_path: root.path().into(),
                    readonly: false,
                    resolve: DirResolve {
                        boundary: BoundaryResolve::InRoot,
                        symlinks: SymlinkPolicy::Disallow,
                    },
                },
            })
            .unwrap();
        let result = service.handle(MountOperation::Open {
            operation_id: OperationId(1),
            mount_id,
            path: uri("mount://x/link/a").unwrap().path,
            options: OpenOptions::default(),
        });
        assert!(matches!(
            result,
            MountCompletion::Failed {
                error: MountError::SymlinkDisallowed,
                ..
            }
        ));
    }

    #[cfg(unix)]
    #[test]
    fn allow_policy_resolves_nested_and_absolute_links_inside_root() {
        // Verifies relative, nested, and absolute link targets use mount-root semantics.
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("real")).unwrap();
        fs::write(root.path().join("real/a"), b"inside").unwrap();
        symlink("real", root.path().join("first")).unwrap();
        symlink("first", root.path().join("nested")).unwrap();
        symlink("/real/a", root.path().join("absolute")).unwrap();
        let mut service = MountService::default();
        service
            .create_mount(MountConfig {
                name: "x".into(),
                source: Source::Directory {
                    root_path: root.path().into(),
                    readonly: false,
                    resolve: DirResolve::default(),
                },
            })
            .unwrap();
        assert_eq!(
            service.read_uri("mount://x/nested/a", 16).unwrap(),
            b"inside"
        );
        assert_eq!(
            service.read_uri("mount://x/absolute", 16).unwrap(),
            b"inside"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_and_loops_fail_closed() {
        // Verifies external relative targets and cyclic links cannot escape or hang resolution.
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("secret"), b"secret").unwrap();
        symlink(outside.path().join("secret"), root.path().join("escape")).unwrap();
        symlink("loop-b", root.path().join("loop-a")).unwrap();
        symlink("loop-a", root.path().join("loop-b")).unwrap();
        let mut service = MountService::default();
        service
            .create_mount(MountConfig {
                name: "x".into(),
                source: Source::Directory {
                    root_path: root.path().into(),
                    readonly: false,
                    resolve: DirResolve::default(),
                },
            })
            .unwrap();
        assert!(service.read_uri("mount://x/escape", 16).is_err());
        assert_eq!(
            service.read_uri("mount://x/loop-a", 16),
            Err(MountError::SymlinkLoop)
        );
    }
}
