use std::path::PathBuf;
use std::sync::Arc;

/// Describes the operation for which a virtual filesystem asks its authority.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthorizationOperation {
    /// Read all or part of a regular file.
    ReadFile,
}

/// A path-scoped authorization request made by a registered virtual filesystem.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizationRequest<'a> {
    /// Name of the virtual filesystem being accessed.
    pub vfs: &'a str,
    /// Normalized path relative to the virtual filesystem root.
    pub path: &'a str,
    /// Operation requested by the script.
    pub operation: AuthorizationOperation,
}

/// Result returned by an application-owned [`Authority`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthorizationResult {
    /// Permit this operation.
    Allow,
    /// Reject this operation.
    Deny,
}

/// Application-owned policy for one virtual filesystem.
///
/// The runtime does not cache decisions. Applications may keep, persist, or
/// revoke their own allow-list and decide independently for every request.
pub trait Authority: Send + Sync + 'static {
    /// Decides whether one normalized VFS path may be accessed.
    fn authorization_requested(&self, request: &AuthorizationRequest<'_>) -> AuthorizationResult;
}

/// Host data exposed through a virtual filesystem.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Source {
    /// A real directory opened once by the trusted host during runtime build.
    LoadDir { path: PathBuf },
}

/// Configuration for a named virtual filesystem.
pub struct VfsConfig {
    /// URL scheme used by JavaScript, for example `project` in
    /// `project://example.txt`.
    pub name: String,
    /// Trusted host source mounted at the VFS root.
    pub source: Source,
    /// Optional application-owned per-request policy. Absence allows access.
    pub authority: Option<Arc<dyn Authority>>,
}

/// Stable runtime-local identifier assigned to a registered VFS.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct VfsDescriptor(pub(crate) u32);

pub(crate) fn valid_vfs_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(b'a'..=b'z'))
        && bytes.all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'+' | b'-' | b'.'))
}
